package io.superteem.voice

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothSocket
import android.os.SystemClock
import android.util.Log
import java.io.File
import java.io.InputStream
import java.util.concurrent.ConcurrentHashMap

const val AUDIO_PSM = 0x0080
private const val TAG = "StvSource"
private const val SYNC_INTERVAL_MS = 1000L
private const val RECONNECT_DELAY_MS = 2000L
/** Two seconds of 20 ms frames of exact zeros: a microphone that is switched off, not a quiet room. */
const val SILENT_FRAMES = 100
const val SILENCED_DETAIL = "Android is giving Superteem silence. Check that Microphone access is on in Quick Settings, " +
    "and that no call or voice assistant is using the microphone."

data class SourceStats(
    val state: String = "starting",
    val frames: Long = 0,
    val lost: Long = 0,
    val reconnects: Int = 0,
    val detail: String = "",
    /** Input level from 0 to 1 for the meter; pins report none. */
    val level: Float = 0f,
    /** The last change of microphone to tell the person about, numbered so each is told once. */
    val notice: String = "",
    val noticeId: Int = 0,
    /** Frames recorded by the microphone chosen, not one fallen back to: what says the chosen one works. */
    val ownFrames: Long = 0,
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

/** Live per-stream health, read by a screen. */
class StatsTable {
    val sources = ConcurrentHashMap<String, SourceStats>()

    fun update(name: String, change: (SourceStats) -> SourceStats) {
        sources.compute(name) { _, old -> change(old ?: SourceStats()) }
    }
}

/** The recording's streams, read by the session screen. */
val Stats = StatsTable()

/** Microphones being listened to before a meeting, so the New meeting page can show their meters. */
val ProbeStats = StatsTable()

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

