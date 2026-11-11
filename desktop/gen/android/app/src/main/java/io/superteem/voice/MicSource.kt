package io.superteem.voice

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioRecordingConfiguration
import android.media.AudioRouting
import android.media.MediaRecorder
import android.os.Build
import android.os.SystemClock
import android.util.Log
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean

private const val TAG = "StvMic"
private const val RATE = 16_000
private const val FRAME_SAMPLES = RATE / 50
/** No audio at all for this long: the stream has died, as when a Bluetooth link drops. */
private const val STALL_MS = 1500L
/** A headset giving exact digital silence for this many frames (4 s) has lost its link; it takes a second or so to start. */
private const val HEADSET_DEAD_FRAMES = 200
/** A headset that hasn't been routed to after this long isn't going to be. */
private const val HEADSET_ROUTE_MS = 5000L
private const val RETRY_MS = 2000L

private fun nowUs() = SystemClock.elapsedRealtimeNanos() / 1000

private fun isBluetooth(device: AudioDeviceInfo?) =
    device?.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO || device?.type == AudioDeviceInfo.TYPE_BLE_HEADSET

/**
 * Records one microphone stream. `wanted` is the microphone chosen; with `fallback` the stream records from the other
 * when that one is missing or stops working, and goes back when it returns, keeping one journal and one timeline
 * throughout. Without `dir` nothing is written: the microphone is only listened to, for its meter.
 */
@SuppressLint("MissingPermission")
class MicSource(
    private val context: Context,
    private val dir: File?,
    val label: String,
    private val wanted: Mic,
    private val stats: StatsTable = Stats,
    private val fallback: Boolean = false,
    /** The phone has a stream of its own in this recording, so this one doesn't fall back to it. */
    private val phoneRecordedElsewhere: Boolean = false,
) : Thread(label) {
    @Volatile private var running = true
    private val lock = Object()
    /** Set when a microphone comes or goes, or the route of the stream changes. */
    private val changed = AtomicBoolean(false)
    /** The headset stopped giving audio; it isn't tried again until a Bluetooth microphone is connected anew. */
    @Volatile private var headsetFailed = false
    private lateinit var audio: AudioManager

    private val deviceWatch = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) {
            if (added.any { it.isSource && isBluetooth(it) }) headsetFailed = false
            poke()
        }

        override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) = poke()
    }

    private fun poke() {
        changed.set(true)
        synchronized(lock) { lock.notifyAll() }
    }

    private enum class End { STOPPED, CHANGED, DEAD }

    private class Capture(val record: AudioRecord, val mic: Mic, val silenced: AtomicBoolean, val watch: AudioManager.AudioRecordingCallback)

    override fun run() {
        audio = context.getSystemService(AudioManager::class.java)
        audio.registerAudioDeviceCallback(deviceWatch, null)
        val journal = dir?.let { JournalWriter(File(it, "$label.stvj")) }
        val timeline = Timeline(RATE, FRAME_SAMPLES)
        var current: Mic? = null
        try {
            while (running) {
                changed.set(false)
                val choice = choose()
                if (choice == null) {
                    val detail = if (current == Mic.HEADSET) "The Bluetooth headset disconnected. Waiting for it to come back"
                    else "Connect a Bluetooth headset and start again"
                    stats.update(label) { it.copy(state = if (fallback) "waiting" else "no headset", detail = detail, level = 0f) }
                    pause(RETRY_MS)
                    continue
                }
                val capture = try {
                    open(choice)
                } catch (e: Exception) {
                    Log.w(TAG, "$label: ${choice.input} could not be opened", e)
                    if (choice == Mic.HEADSET) releaseHeadset()
                    null
                }
                if (capture == null) {
                    if (choice == Mic.HEADSET && fallback) {
                        headsetFailed = true
                    } else {
                        stats.update(label) { it.copy(state = "failed", detail = "The microphone could not be opened. Another app may be using it.", level = 0f) }
                        pause(RETRY_MS)
                    }
                    continue
                }
                MicChoice.notice(wanted, current, choice)?.let { text ->
                    Log.i(TAG, "$label: $text")
                    stats.update(label) { it.copy(notice = text, noticeId = it.noticeId + 1) }
                }
                current = choice
                timeline.resumeAt(nowUs())
                val end = try {
                    record(capture, journal, timeline)
                } catch (e: Exception) {
                    Log.w(TAG, "$label capture ended", e)
                    End.DEAD
                } finally {
                    close(capture)
                }
                if (end == End.DEAD) {
                    if (choice == Mic.HEADSET && fallback) headsetFailed = true else pause(RETRY_MS / 4)
                }
            }
            stats.update(label) { it.copy(state = "stopped") }
        } catch (e: Exception) {
            Log.e(TAG, "$label capture failed", e)
            stats.update(label) { it.copy(state = "failed", detail = e.message ?: "") }
        } finally {
            runCatching { journal?.close() }
            runCatching { audio.unregisterAudioDeviceCallback(deviceWatch) }
        }
    }

    private fun choose() = MicChoice.choose(wanted, headsetAvailable(), headsetFailed, phoneRecordedElsewhere, fallback)

    /** Waits until a microphone comes or goes, or the time is up. */
    private fun pause(ms: Long) {
        synchronized(lock) { if (running && !changed.get()) lock.wait(ms) }
    }

    private fun headsetAvailable(): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) return audio.availableCommunicationDevices.any(::isBluetooth)
        return audio.getDevices(AudioManager.GET_DEVICES_INPUTS).any { it.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO }
    }

    private fun open(mic: Mic): Capture? {
        val device: AudioDeviceInfo?
        if (mic == Mic.HEADSET) {
            audio.mode = AudioManager.MODE_IN_COMMUNICATION
            device = routeToHeadset()
            if (device == null) {
                releaseHeadset()
                return null
            }
        } else {
            // The phone's own microphone, even when a headset is connected.
            device = audio.getDevices(AudioManager.GET_DEVICES_INPUTS).firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_MIC }
        }
        val minBuffer = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val source = if (mic == Mic.HEADSET) MediaRecorder.AudioSource.VOICE_COMMUNICATION else MediaRecorder.AudioSource.MIC
        val record = try {
            AudioRecord(source, RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(minBuffer, FRAME_SAMPLES * 8))
        } catch (e: Exception) {
            Log.w(TAG, "$label: ${mic.input} could not be opened", e)
            if (mic == Mic.HEADSET) releaseHeadset()
            return null
        }
        // Android gives an app silence, without an error, when the microphone switch in Quick Settings is off
        // or another app (a call, a voice assistant) has the microphone. That is told apart from a quiet room.
        val silenced = AtomicBoolean(false)
        val watch = object : AudioManager.AudioRecordingCallback() {
            override fun onRecordingConfigChanged(configs: MutableList<AudioRecordingConfiguration>) {
                configs.firstOrNull { it.clientAudioSessionId == record.audioSessionId }?.let { silenced.set(it.isClientSilenced) }
            }
        }
        val capture = Capture(record, mic, silenced, watch)
        try {
            if (record.state != AudioRecord.STATE_INITIALIZED) throw IllegalStateException("not initialised")
            device?.let { record.setPreferredDevice(it) }
            record.registerAudioRecordingCallback(context.mainExecutor, watch)
            record.addOnRoutingChangedListener(AudioRouting.OnRoutingChangedListener { poke() }, null)
            record.startRecording()
            if (record.recordingState != AudioRecord.RECORDSTATE_RECORDING) throw IllegalStateException("did not start")
            audio.activeRecordingConfigurations.firstOrNull { it.clientAudioSessionId == record.audioSessionId }
                ?.let { silenced.set(it.isClientSilenced) }
            return capture
        } catch (e: Exception) {
            Log.w(TAG, "$label: ${mic.input} did not start", e)
            close(capture)
            return null
        }
    }

    private fun close(capture: Capture) {
        val record = capture.record
        runCatching { record.unregisterAudioRecordingCallback(capture.watch) }
        runCatching { record.stop() }
        runCatching { record.release() }
        if (capture.mic == Mic.HEADSET) releaseHeadset()
    }

    /** Reads frames until stopped, the microphone changes, or the stream stops giving audio. */
    private fun record(capture: Capture, journal: JournalWriter?, timeline: Timeline): End {
        val record = capture.record
        val opened = SystemClock.elapsedRealtime()
        val watch = StallWatch(STALL_MS, HEADSET_DEAD_FRAMES).apply { opened(opened) }
        var routedToHeadset = false
        val actual = "${record.sampleRate} Hz via ${record.routedDevice?.productName ?: capture.mic.input}"
        stats.update(label) { it.copy(state = "recording", detail = actual) }
        val buffer = ByteArray(FRAME_SAMPLES * 2)
        var at = 0
        var lastSync = SystemClock.elapsedRealtime()
        while (running) {
            val now = SystemClock.elapsedRealtime()
            if (changed.getAndSet(false) && choose() != capture.mic) return End.CHANGED
            if (capture.mic == Mic.HEADSET) {
                // When the Bluetooth link drops, Android quietly moves the stream to another microphone.
                val routed = record.routedDevice
                if (isBluetooth(routed)) routedToHeadset = true
                else if (routed != null && (routedToHeadset || now - opened > HEADSET_ROUTE_MS)) return End.DEAD
            }
            val n = record.read(buffer, at, buffer.size - at, AudioRecord.READ_NON_BLOCKING)
            if (n < 0) {
                Log.w(TAG, "$label: AudioRecord error $n")
                return End.DEAD
            }
            if (n == 0) {
                if (watch.stalled(now)) return End.DEAD
                sleep(5)
                continue
            }
            at += n
            if (at < buffer.size) continue
            at = 0
            val (sequence, samples) = timeline.next(nowUs())
            journal?.append(nowUs(), encodeFrame(1, sequence, samples.toInt(), CODEC_RAW_PCM16, buffer, buffer.size))
            val level = meterLevel(buffer, buffer.size)
            watch.frame(now, buffer.all { it == 0.toByte() })
            val androidSilenced = capture.silenced.get()
            if (capture.mic == Mic.HEADSET && fallback && !androidSilenced && watch.silent()) return End.DEAD
            val silenced = androidSilenced || watch.zeroRun >= SILENT_FRAMES
            val own = capture.mic == wanted
            stats.update(label) {
                it.copy(
                    frames = it.frames + 1,
                    ownFrames = if (own) it.ownFrames + 1 else it.ownFrames,
                    level = level,
                    state = if (silenced) "silenced" else "recording",
                    detail = if (silenced) SILENCED_DETAIL else actual,
                )
            }
            if (journal != null && now - lastSync >= 1000) {
                journal.sync()
                lastSync = now
            }
        }
        return End.STOPPED
    }

    private fun routeToHeadset(): AudioDeviceInfo? {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val device = audio.availableCommunicationDevices.firstOrNull(::isBluetooth) ?: return null
            return device.takeIf { audio.setCommunicationDevice(it) }
        }
        val device = audio.getDevices(AudioManager.GET_DEVICES_INPUTS)
            .firstOrNull { it.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO } ?: return null
        @Suppress("DEPRECATION")
        audio.startBluetoothSco()
        return device
    }

    private fun releaseHeadset() {
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                audio.clearCommunicationDevice()
            } else {
                @Suppress("DEPRECATION")
                audio.stopBluetoothSco()
            }
        }
        runCatching { audio.mode = AudioManager.MODE_NORMAL }
    }

    fun shutdown() {
        running = false
        synchronized(lock) { lock.notifyAll() }
    }
}
