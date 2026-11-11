package io.superteem.voice

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MicRouteTest {
    private fun choose(
        wanted: Mic,
        headset: Boolean,
        failed: Boolean = false,
        phoneElsewhere: Boolean = false,
        fallback: Boolean = true,
    ) = MicChoice.choose(wanted, headset, failed, phoneElsewhere, fallback)

    @Test
    fun headsetIsUsedWhileItWorks() {
        assertEquals(Mic.HEADSET, choose(Mic.HEADSET, headset = true))
    }

    @Test
    fun missingOrFailedHeadsetFallsBackToThePhone() {
        assertEquals(Mic.PHONE, choose(Mic.HEADSET, headset = false))
        assertEquals(Mic.PHONE, choose(Mic.HEADSET, headset = true, failed = true))
    }

    @Test
    fun headsetWaitsWhenThePhoneHasItsOwnStreamOrForAMicrophoneTest() {
        assertNull(choose(Mic.HEADSET, headset = false, phoneElsewhere = true))
        assertNull(choose(Mic.HEADSET, headset = false, fallback = false))
    }

    @Test
    fun phoneStaysOnThePhoneWithAHeadsetConnected() {
        assertEquals(Mic.PHONE, choose(Mic.PHONE, headset = true))
        assertEquals(Mic.PHONE, choose(Mic.PHONE, headset = false))
    }

    @Test
    fun switchesAreToldOnce() {
        assertNull(MicChoice.notice(Mic.HEADSET, null, Mic.HEADSET))
        assertEquals(
            "The Bluetooth headset isn't available, so the phone microphone is recording",
            MicChoice.notice(Mic.HEADSET, null, Mic.PHONE),
        )
        assertEquals("Switched to the phone microphone", MicChoice.notice(Mic.HEADSET, Mic.HEADSET, Mic.PHONE))
        assertEquals("Switched to the Bluetooth headset", MicChoice.notice(Mic.HEADSET, Mic.PHONE, Mic.HEADSET))
        assertNull(MicChoice.notice(Mic.HEADSET, Mic.PHONE, Mic.PHONE))
        assertNull(MicChoice.notice(Mic.HEADSET, Mic.HEADSET, null))
    }

    @Test
    fun timelineSkipsTheGapOfASwitch() {
        val t = Timeline(16_000, 320)
        t.resumeAt(5_000_000) // before the first frame: nothing to skip
        assertEquals(0 to 0L, t.next(1_000_000))
        assertEquals(1 to 320L, t.next(1_020_000))
        // Reopened 1 s after the last frame: 49 frames' worth of samples skipped, the sequence carries on.
        t.resumeAt(2_020_000)
        assertEquals(2 to 640L + 49 * 320, t.next(2_020_000))
        // Reopened straight away: no gap.
        t.resumeAt(2_040_000)
        assertEquals(3 to 640L + 50 * 320, t.next(2_040_000))
    }

    @Test
    fun stallWatchTellsADeadStreamFromAQuietOne() {
        val w = StallWatch(1500, 3)
        w.opened(0)
        assertFalse(w.stalled(1000))
        assertTrue(w.stalled(1500))
        w.frame(1600, allZero = true)
        w.frame(1620, allZero = true)
        assertFalse(w.silent())
        w.frame(1640, allZero = true)
        assertTrue(w.silent())
        w.frame(1660, allZero = false)
        assertFalse(w.silent())
        assertFalse(w.stalled(1700))
    }
}
