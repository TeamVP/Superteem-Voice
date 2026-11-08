package io.superteem.voice

import java.io.BufferedOutputStream
import java.io.File
import java.io.FileOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.zip.CRC32

/** Writes the journal format read by core/src/journal.rs. */
class JournalWriter(file: File) : AutoCloseable {
    private val fileStream = FileOutputStream(file)
    private val out = BufferedOutputStream(fileStream)

    init {
        check(file.length() == 0L) { "journal already exists: $file" }
        out.write("STVJ".toByteArray())
        out.write(1)
        sync()
    }

    fun append(arrivalUs: Long, frame: ByteArray) {
        val body = ByteBuffer.allocate(8 + frame.size).order(ByteOrder.LITTLE_ENDIAN)
            .putLong(arrivalUs).put(frame).array()
        val crc = CRC32().apply { update(body) }.value.toInt()
        val head = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).putInt(body.size).putInt(crc).array()
        out.write(head)
        out.write(body)
    }

    fun sync() {
        out.flush()
        fileStream.fd.sync()
    }

    override fun close() {
        sync()
        out.close()
    }
}

const val FRAME_HEADER_LEN = 15
const val CODEC_RAW_PCM16 = 0

/** Builds a frame in the layout from protocol/PROTOCOL-v0.md. */
fun encodeFrame(sessionId: Int, sequence: Int, sampleCounter: Int, codec: Int, payload: ByteArray, length: Int): ByteArray =
    ByteBuffer.allocate(FRAME_HEADER_LEN + length).order(ByteOrder.LITTLE_ENDIAN)
        .put(0)
        .put(0)
        .putShort(sessionId.toShort())
        .putInt(sequence)
        .putInt(sampleCounter)
        .put(codec.toByte())
        .putShort(length.toShort())
        .put(payload, 0, length)
        .array()

/** Reads the sequence number from a frame header. */
fun frameSequence(header: ByteArray): Int =
    ByteBuffer.wrap(header).order(ByteOrder.LITTLE_ENDIAN).getInt(4)

/** Reads the payload length from a frame header. */
fun framePayloadLength(header: ByteArray): Int =
    ByteBuffer.wrap(header).order(ByteOrder.LITTLE_ENDIAN).getShort(13).toInt() and 0xFFFF
