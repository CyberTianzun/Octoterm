package ai.eclosion.octoterm.android.wire

import java.nio.ByteBuffer
import java.nio.ByteOrder

object Frame {
    const val CONTROL_CHANNEL: Int = 0
    const val HEADER_SIZE: Int = 5

    fun encode(channel: Int, payload: ByteArray, flags: Int = 0): ByteArray {
        val buf = ByteBuffer.allocate(HEADER_SIZE + payload.size).order(ByteOrder.LITTLE_ENDIAN)
        buf.putInt(channel)
        buf.put(flags.toByte())
        buf.put(payload)
        return buf.array()
    }

    fun decode(data: ByteArray): Decoded {
        if (data.size < HEADER_SIZE) {
            throw IllegalArgumentException("frame shorter than 5-byte header")
        }
        val buf = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN)
        val channel = buf.int
        val flags = buf.get().toInt() and 0xff
        val payload = ByteArray(data.size - HEADER_SIZE)
        buf.get(payload)
        return Decoded(channel, flags, payload)
    }

    data class Decoded(
        val channel: Int,
        val flags: Int,
        val payload: ByteArray,
    ) {
        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (other !is Decoded) return false
            return channel == other.channel && flags == other.flags && payload.contentEquals(other.payload)
        }

        override fun hashCode(): Int {
            var result = channel
            result = 31 * result + flags
            result = 31 * result + payload.contentHashCode()
            return result
        }
    }
}
