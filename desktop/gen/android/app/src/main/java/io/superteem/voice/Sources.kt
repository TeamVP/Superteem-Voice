package io.superteem.voice

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothSocket
import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.Build
import android.os.SystemClock
import android.util.Log
import java.io.File
import java.io.InputStream
import java.util.concurrent.ConcurrentHashMap

const val AUDIO_PSM = 0x0080
private const val TAG = "StvSource"
private const val SYNC_INTERVAL_MS = 1000L
private const val RECONNECT_DELAY_MS = 2000L

data class SourceStats(
    val state: String = "starting",
    val frames: Long = 0,
    val lost: Long = 0,
    val reconnects: Int = 0,
    val detail: String = "",
    /** Input level from 0 to 1 for the meter; pins report none. */
    val level: Float = 0f,
)

/** Maps a 16-bit PCM frame's RMS level to 0..1 over a 70 dB range, matching the Windows meter. */
fun meterLevel(pcm: ByteArray, length: Int): Float {
    var sum = 0.0
    var i = 0
    while (i + 1 < length) {
        val s = ((pcm[i + 1].toInt() shl 8) or (pcm[i].toInt() and 0xFF)).toShort().toDouble()
        sum += s * s
        i += 2
    }
    val power = maxOf(sum / maxOf(length / 2, 1), 1.0) / (32768.0 * 32768.0)
    val db = 10 * kotlin.math.log10(power)
    return ((db + 70) / 70).toFloat().coerceIn(0f, 1f)
}

/** Live per-stream health, read by the session screen. */
object Stats {
    val sources = ConcurrentHashMap<String, SourceStats>()

    fun update(name: String, change: (SourceStats) -> SourceStats) {
        sources.compute(name) { _, old -> change(old ?: SourceStats()) }
    }
}

private fun nowUs() = SystemClock.elapsedRealtimeNanos() / 1000

/** Receives one pin's frames, reconnecting until stopped. Each connection gets its own journal. */
@SuppressLint("MissingPermission")
class PinSource(private val device: BluetoothDevice, private val dir: File, private val label: String) : Thread("pin-$label") {
    @Volatile private var running = true
    @Volatile private var socket: BluetoothSocket? = null

    override fun run() {
        var attempt = 0
        while (running) {
            try {
                Stats.update(label) { it.copy(state = "connecting") }
                val sock = device.createL2capChannel(AUDIO_PSM)
                socket = sock
                sock.connect()
                Stats.update(label) { it.copy(state = "recording") }
                JournalWriter(File(dir, "$label-$attempt.stvj")).use { receive(sock.inputStream, it) }
            } catch (e: Exception) {
                Log.w(TAG, "$label link ended", e)
            }
            attempt++
            runCatching { socket?.close() }
            if (running) {
                Stats.update(label) { it.copy(state = "reconnecting", reconnects = it.reconnects + 1) }
                runCatching { sleep(RECONNECT_DELAY_MS) }
            }
        }
        Stats.update(label) { it.copy(state = "stopped") }
    }

    private fun receive(input: InputStream, journal: JournalWriter) {
        val header = ByteArray(FRAME_HEADER_LEN)
        var lastSequence: Int? = null
        var lastSync = SystemClock.elapsedRealtime()
        while (running) {
            if (!readFully(input, header, header.size)) return
            val arrival = nowUs()
            val frame = header.copyOf(FRAME_HEADER_LEN + framePayloadLength(header))
            if (!readFully(input, frame, frame.size, FRAME_HEADER_LEN)) return
            journal.append(arrival, frame)

            val sequence = frameSequence(header)
            val lost = lastSequence?.let { (sequence - it - 1).coerceAtLeast(0).toLong() } ?: 0
            lastSequence = sequence
            Stats.update(label) { it.copy(frames = it.frames + 1, lost = it.lost + lost) }

            if (SystemClock.elapsedRealtime() - lastSync >= SYNC_INTERVAL_MS) {
                journal.sync()
                lastSync = SystemClock.elapsedRealtime()
            }
        }
    }

    private fun readFully(input: InputStream, into: ByteArray, end: Int, start: Int = 0): Boolean {
        var at = start
        while (at < end) {
            val n = input.read(into, at, end - at)
            if (n < 0) return false
            at += n
        }
        return true
    }

    fun shutdown() {
        running = false
        runCatching { socket?.close() }
    }
}

/**
 * Records the Bluetooth headset mic, or the phone's own mic when `headset` is false.
 * The phone's own sequence numbers fill the frame header.
 */
@SuppressLint("MissingPermission")
class MicSource(
    private val context: Context,
    private val dir: File,
    val label: String,
    private val headset: Boolean,
) : Thread(label) {
    @Volatile private var running = true

    override fun run() {
        val audio = context.getSystemService(AudioManager::class.java)
        var device: AudioDeviceInfo? = null
        if (headset) {
            audio.mode = AudioManager.MODE_IN_COMMUNICATION
            device = routeToHeadset(audio)
            if (device == null) {
                Stats.update(label) { it.copy(state = "no headset", detail = "Connect a Bluetooth headset and start again") }
                audio.mode = AudioManager.MODE_NORMAL
                return
            }
        }
        val rate = 16_000
        val samples = rate / 50
        val minBuffer = AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val source = if (headset) MediaRecorder.AudioSource.VOICE_COMMUNICATION else MediaRecorder.AudioSource.MIC
        val record = AudioRecord(
            source, rate, AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT, maxOf(minBuffer, samples * 8),
        )
        try {
            device?.let { record.setPreferredDevice(it) }
            record.startRecording()
            val actual = "${record.sampleRate} Hz via ${record.routedDevice?.productName ?: "unknown"}"
            Stats.update(label) { it.copy(state = "recording", detail = actual) }
            JournalWriter(File(dir, "$label.stvj")).use { journal ->
                val buffer = ByteArray(samples * 2)
                var sequence = 0
                var lastSync = SystemClock.elapsedRealtime()
                while (running) {
                    var at = 0
                    while (at < buffer.size && running) {
                        val n = record.read(buffer, at, buffer.size - at)
                        if (n < 0) throw IllegalStateException("AudioRecord error $n")
                        at += n
                    }
                    if (at < buffer.size) break
                    journal.append(nowUs(), encodeFrame(1, sequence, sequence * samples, CODEC_RAW_PCM16, buffer, buffer.size))
                    sequence++
                    val level = meterLevel(buffer, buffer.size)
                    Stats.update(label) { it.copy(frames = it.frames + 1, level = level) }
                    if (SystemClock.elapsedRealtime() - lastSync >= SYNC_INTERVAL_MS) {
                        journal.sync()
                        lastSync = SystemClock.elapsedRealtime()
                    }
                }
            }
            Stats.update(label) { it.copy(state = "stopped") }
        } catch (e: Exception) {
            Log.e(TAG, "headset capture failed", e)
            Stats.update(label) { it.copy(state = "failed", detail = e.message ?: "") }
        } finally {
            runCatching { record.stop() }
            record.release()
            if (headset) {
                clearRoute(audio)
                audio.mode = AudioManager.MODE_NORMAL
            }
        }
    }

    private fun routeToHeadset(audio: AudioManager): AudioDeviceInfo? {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val device = audio.availableCommunicationDevices.firstOrNull {
                it.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO || it.type == AudioDeviceInfo.TYPE_BLE_HEADSET
            } ?: return null
            return device.takeIf { audio.setCommunicationDevice(it) }
        }
        val device = audio.getDevices(AudioManager.GET_DEVICES_INPUTS)
            .firstOrNull { it.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO } ?: return null
        @Suppress("DEPRECATION")
        audio.startBluetoothSco()
        return device
    }

    private fun clearRoute(audio: AudioManager) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            audio.clearCommunicationDevice()
        } else {
            @Suppress("DEPRECATION")
            audio.stopBluetoothSco()
        }
    }

    fun shutdown() {
        running = false
    }
}
