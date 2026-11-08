package io.superteem.voice

import org.junit.Assert.assertEquals
import org.junit.Test
import java.io.File

class JournalTest {
    /** Also leaves a sample for the Rust core's reader: see core/tests/replay.rs. */
    @Test
    fun writesSampleJournal() {
        val file = File("build/journal-sample.stvj").apply { delete() }
        JournalWriter(file).use { journal ->
            for (i in 0 until 5) {
                val payload = ByteArray(640) { (i and 0x7F).toByte() }
                journal.append(1000L + i, encodeFrame(9, i, i * 320, CODEC_RAW_PCM16, payload, payload.size))
            }
        }
        assertEquals(5L + 5 * (8 + 8 + 15 + 640), file.length())
    }

    @Test
    fun headerFieldsReadBack() {
        val frame = encodeFrame(1, 42, 640, CODEC_RAW_PCM16, byteArrayOf(1, 2, 3), 3)
        assertEquals(42, frameSequence(frame))
        assertEquals(3, framePayloadLength(frame))
    }
}
