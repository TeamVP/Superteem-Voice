/* SPDX-License-Identifier: MIT */
/* Step 0 radio test firmware for the XIAO nRF52840 Sense.
 *
 * Streams ADPCM frames over an L2CAP channel (PSM 0x0080), or GATT notifications as a fallback,
 * only while a bonded phone is subscribed. The green LED is on while streaming audio. Capture to
 * flash is not implemented yet, so the red "recording locally" state does not exist here.
 */

#include <string.h>
#include <zephyr/audio/dmic.h>
#include <zephyr/bluetooth/bluetooth.h>
#include <zephyr/bluetooth/conn.h>
#include <zephyr/bluetooth/gatt.h>
#include <zephyr/bluetooth/l2cap.h>
#include <zephyr/bluetooth/uuid.h>
#include <zephyr/drivers/gpio.h>
#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>
#include <zephyr/net_buf.h>
#include <zephyr/random/random.h>
#include <zephyr/settings/settings.h>
#include <zephyr/sys/byteorder.h>

#include "adpcm.h"

LOG_MODULE_REGISTER(stv_pin, LOG_LEVEL_INF);

#define SAMPLES_PER_FRAME 320
#define BLOCK_BYTES       (SAMPLES_PER_FRAME * 2)
#define BLOCK_COUNT       8
#define HEADER_LEN        15
#define CODEC_IMA_ADPCM   2
#define PAYLOAD_LEN       (STV_ADPCM_STATE_LEN + SAMPLES_PER_FRAME / 2)
#define FRAME_LEN         (HEADER_LEN + PAYLOAD_LEN)
#define AUDIO_PSM         0x0080

K_MEM_SLAB_DEFINE_STATIC(pcm_slab, BLOCK_BYTES, BLOCK_COUNT, 4);
NET_BUF_POOL_FIXED_DEFINE(tx_pool, 8, BT_L2CAP_SDU_BUF_SIZE(FRAME_LEN), CONFIG_BT_CONN_TX_USER_DATA_SIZE,
			  NULL);

static const struct device *const dmic = DEVICE_DT_GET(DT_NODELABEL(pdm0));
static const struct gpio_dt_spec green_led = GPIO_DT_SPEC_GET(DT_ALIAS(led1), gpios);

static struct bt_conn *active_conn;
static bool l2cap_open;
static bool gatt_subscribed;
static K_SEM_DEFINE(stream_wanted, 0, 1);

#define STV_SERVICE_UUID BT_UUID_128_ENCODE(0x5354564f, 0x1000, 0x4e00, 0x8000, 0x535456000001)
#define STV_AUDIO_UUID   BT_UUID_128_ENCODE(0x5354564f, 0x1001, 0x4e00, 0x8000, 0x535456000001)

static void audio_ccc_changed(const struct bt_gatt_attr *attr, uint16_t value);

BT_GATT_SERVICE_DEFINE(stv_service, BT_GATT_PRIMARY_SERVICE(BT_UUID_DECLARE_128(STV_SERVICE_UUID)),
		       BT_GATT_CHARACTERISTIC(BT_UUID_DECLARE_128(STV_AUDIO_UUID), BT_GATT_CHRC_NOTIFY,
					      BT_GATT_PERM_NONE, NULL, NULL, NULL),
		       BT_GATT_CCC(audio_ccc_changed, BT_GATT_PERM_READ | BT_GATT_PERM_WRITE_ENCRYPT));

static bool streaming_wanted(void)
{
	return active_conn != NULL && (l2cap_open || gatt_subscribed);
}

static void update_stream_state(void)
{
	if (streaming_wanted()) {
		k_sem_give(&stream_wanted);
	}
}

static void audio_ccc_changed(const struct bt_gatt_attr *attr, uint16_t value)
{
	ARG_UNUSED(attr);
	gatt_subscribed = value == BT_GATT_CCC_NOTIFY;
	update_stream_state();
}

static int l2cap_recv(struct bt_l2cap_chan *chan, struct net_buf *buf)
{
	ARG_UNUSED(chan);
	ARG_UNUSED(buf);
	return 0;
}

static void l2cap_connected(struct bt_l2cap_chan *chan)
{
	ARG_UNUSED(chan);
	l2cap_open = true;
	update_stream_state();
}

static void l2cap_disconnected(struct bt_l2cap_chan *chan)
{
	ARG_UNUSED(chan);
	l2cap_open = false;
}

static const struct bt_l2cap_chan_ops l2cap_ops = {
	.recv = l2cap_recv,
	.connected = l2cap_connected,
	.disconnected = l2cap_disconnected,
};

static struct bt_l2cap_le_chan audio_chan = {
	.chan.ops = &l2cap_ops,
	.rx.mtu = 64,
};

static int l2cap_accept(struct bt_conn *conn, struct bt_l2cap_server *server, struct bt_l2cap_chan **chan)
{
	ARG_UNUSED(conn);
	ARG_UNUSED(server);
	if (l2cap_open) {
		return -ENOMEM;
	}
	*chan = &audio_chan.chan;
	return 0;
}

static struct bt_l2cap_server l2cap_server = {
	.psm = AUDIO_PSM,
	.sec_level = BT_SECURITY_L2,
	.accept = l2cap_accept,
};

static void start_advertising(struct k_work *work)
{
	const struct bt_data ad[] = {
		BT_DATA_BYTES(BT_DATA_FLAGS, BT_LE_AD_GENERAL | BT_LE_AD_NO_BREDR),
		BT_DATA_BYTES(BT_DATA_UUID128_ALL, STV_SERVICE_UUID),
	};
	const struct bt_data sd[] = {
		BT_DATA(BT_DATA_NAME_COMPLETE, CONFIG_BT_DEVICE_NAME, sizeof(CONFIG_BT_DEVICE_NAME) - 1),
	};
	int err = bt_le_adv_start(BT_LE_ADV_CONN_FAST_2, ad, ARRAY_SIZE(ad), sd, ARRAY_SIZE(sd));

	ARG_UNUSED(work);
	if (err != 0 && err != -EALREADY) {
		LOG_ERR("advertising failed: %d", err);
	}
}

static K_WORK_DEFINE(adv_work, start_advertising);

static void on_connected(struct bt_conn *conn, uint8_t err)
{
	static const struct bt_conn_le_phy_param phy = {
		.options = BT_CONN_LE_PHY_OPT_NONE,
		.pref_tx_phy = BT_GAP_LE_PHY_2M,
		.pref_rx_phy = BT_GAP_LE_PHY_2M,
	};

	if (err != 0) {
		return;
	}
	active_conn = bt_conn_ref(conn);
	/* The phone may refuse these. Its choice is logged on the app side. */
	bt_conn_le_param_update(conn, BT_LE_CONN_PARAM(24, 36, 0, 400));
	bt_conn_le_phy_update(conn, &phy);
}

static void on_disconnected(struct bt_conn *conn, uint8_t reason)
{
	ARG_UNUSED(conn);
	LOG_INF("disconnected: %u", reason);
	bt_conn_unref(active_conn);
	active_conn = NULL;
	gatt_subscribed = false;
}

static void on_recycled(void)
{
	k_work_submit(&adv_work);
}

BT_CONN_CB_DEFINE(conn_callbacks) = {
	.connected = on_connected,
	.disconnected = on_disconnected,
	.recycled = on_recycled,
};

static void write_header(uint8_t *out, uint16_t session, uint32_t sequence, uint32_t sample_counter)
{
	out[0] = 0;
	out[1] = 0;
	sys_put_le16(session, &out[2]);
	sys_put_le32(sequence, &out[4]);
	sys_put_le32(sample_counter, &out[8]);
	out[12] = CODEC_IMA_ADPCM;
	sys_put_le16(PAYLOAD_LEN, &out[13]);
}

/* A frame that cannot be queued is dropped. The sequence gap makes the loss visible to the app. */
static void send_frame(const uint8_t *frame)
{
	if (l2cap_open) {
		struct net_buf *buf = net_buf_alloc(&tx_pool, K_NO_WAIT);

		if (buf == NULL) {
			return;
		}
		net_buf_reserve(buf, BT_L2CAP_SDU_CHAN_SEND_RESERVE);
		net_buf_add_mem(buf, frame, FRAME_LEN);
		if (bt_l2cap_chan_send(&audio_chan.chan, buf) < 0) {
			net_buf_unref(buf);
		}
	} else if (gatt_subscribed && active_conn != NULL) {
		bt_gatt_notify(active_conn, &stv_service.attrs[1], frame, FRAME_LEN);
	}
}

static void stream_until_stopped(void)
{
	static uint8_t frame[FRAME_LEN];
	uint16_t session = (uint16_t)sys_rand32_get();
	uint32_t sequence = 0;
	uint32_t sample_counter = 0;
	uint8_t index = 0;

	if (dmic_trigger(dmic, DMIC_TRIGGER_START) < 0) {
		LOG_ERR("mic start failed");
		return;
	}
	gpio_pin_set_dt(&green_led, 1);
	while (streaming_wanted()) {
		void *block;
		size_t size;

		if (dmic_read(dmic, 0, &block, &size, 1000) < 0) {
			LOG_ERR("mic read failed");
			break;
		}
		write_header(frame, session, sequence, sample_counter);
		stv_adpcm_encode(block, size / 2, &index, &frame[HEADER_LEN]);
		k_mem_slab_free(&pcm_slab, block);
		send_frame(frame);
		sequence++;
		sample_counter += SAMPLES_PER_FRAME;
	}
	gpio_pin_set_dt(&green_led, 0);
	dmic_trigger(dmic, DMIC_TRIGGER_STOP);
}

static int configure_mic(void)
{
	struct pcm_stream_cfg stream = {
		.pcm_rate = 16000,
		.pcm_width = 16,
		.block_size = BLOCK_BYTES,
		.mem_slab = &pcm_slab,
	};
	struct dmic_cfg cfg = {
		.io = {
			.min_pdm_clk_freq = 1000000,
			.max_pdm_clk_freq = 3500000,
			.min_pdm_clk_dc = 40,
			.max_pdm_clk_dc = 60,
		},
		.streams = &stream,
		.channel = {
			.req_num_streams = 1,
			.req_num_chan = 1,
			.req_chan_map_lo = dmic_build_channel_map(0, 0, PDM_CHAN_LEFT),
		},
	};

	return device_is_ready(dmic) ? dmic_configure(dmic, &cfg) : -ENODEV;
}

int main(void)
{
	int err;

	gpio_pin_configure_dt(&green_led, GPIO_OUTPUT_INACTIVE);

	err = configure_mic();
	if (err != 0) {
		LOG_ERR("mic setup failed: %d", err);
		return err;
	}
	err = bt_enable(NULL);
	if (err != 0) {
		LOG_ERR("bluetooth failed: %d", err);
		return err;
	}
	settings_load();
	err = bt_l2cap_server_register(&l2cap_server);
	if (err != 0) {
		LOG_ERR("l2cap failed: %d", err);
		return err;
	}
	k_work_submit(&adv_work);

	while (true) {
		k_sem_take(&stream_wanted, K_FOREVER);
		if (streaming_wanted()) {
			stream_until_stopped();
		}
	}
}
