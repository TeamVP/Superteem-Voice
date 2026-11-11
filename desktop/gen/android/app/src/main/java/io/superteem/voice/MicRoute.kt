package io.superteem.voice

/** The two microphones an Android recording can use. */
enum class Mic(val input: String, val spoken: String) {
    HEADSET(INPUT_HEADSET, "the Bluetooth headset"),
    PHONE(INPUT_PHONE, "the phone microphone"),
}

/**
 * Which microphone a stream records from, decided again whenever a microphone comes or goes or the one in use
 * stops giving audio. Kept apart from Android so it can be tested.
 */
object MicChoice {
    /**
     * The microphone to open now, or null to wait. A stream asked for the phone records the phone's own microphone,
     * even with a headset connected. One asked for the headset falls back to the phone, unless the phone has a
     * stream of its own in this recording (it would only record the same thing twice) or falling back is off
     * (a microphone test, which is about that microphone).
     */
    fun choose(
        wanted: Mic,
        headsetAvailable: Boolean,
        headsetFailed: Boolean,
        phoneRecordedElsewhere: Boolean,
        fallback: Boolean,
    ): Mic? = when {
        wanted == Mic.PHONE -> Mic.PHONE
        headsetAvailable && !headsetFailed -> Mic.HEADSET
        !fallback || phoneRecordedElsewhere -> null
        else -> Mic.PHONE
    }

    /** What to tell the person when the microphone changes, or null when there's nothing to say. */
    fun notice(wanted: Mic, from: Mic?, to: Mic?): String? = when {
        to == null || from == to -> null
        // Starting on another microphone than the one chosen.
        from == null -> if (to == wanted) null else "${wanted.spoken.replaceFirstChar { it.uppercase() }} isn't available, so ${to.spoken} is recording"
        else -> "Switched to ${to.spoken}"
    }
}

/**
 * The sample counter written into each frame. After a gap while the microphone was switched, it moves on by the
 * time that passed, so the rest of the recording stays in step with the clock and with other streams.
 */
class Timeline(private val rate: Int, private val frameSamples: Int) {
    var sequence = 0
        private set
    var samples = 0L
        private set
    private var lastFrameUs: Long? = null

    /** Advances past a gap that ended at `nowUs`, in whole frames. Nothing happens before the first frame. */
    fun resumeAt(nowUs: Long) {
        val last = lastFrameUs ?: return
        val expected = last + frameSamples * 1_000_000L / rate
        val gapFrames = (nowUs - expected) * rate / 1_000_000L / frameSamples
        if (gapFrames > 0) samples += gapFrames * frameSamples
    }

    /** The counters for the frame read at `nowUs`, then moves on by one frame. */
    fun next(nowUs: Long): Pair<Int, Long> {
        val at = sequence to samples
        sequence++
        samples += frameSamples
        lastFrameUs = nowUs
        return at
    }
}

/**
 * Tells a stream that has stopped giving audio from one that's quiet: no frames at all for `stallMs`, or a headset
 * giving exact digital silence (its link dropped) for `zeroFrames` frames in a row.
 */
class StallWatch(private val stallMs: Long, private val zeroFrames: Int) {
    private var lastDataMs = 0L
    /** Frames in a row of exact digital silence, which a working microphone never gives. */
    var zeroRun = 0
        private set

    fun opened(nowMs: Long) {
        lastDataMs = nowMs
        zeroRun = 0
    }

    fun frame(nowMs: Long, allZero: Boolean) {
        lastDataMs = nowMs
        zeroRun = if (allZero) zeroRun + 1 else 0
    }

    fun stalled(nowMs: Long) = nowMs - lastDataMs >= stallMs

    fun silent() = zeroRun >= zeroFrames
}
