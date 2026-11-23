package org.tonalli.spike.music

import android.media.MediaDataSource
import java.io.IOException
import uniffi.tonalli_core.AudioSource

/** Decoder reads remain positioned and bounded; the handle outlives the reader screen. */
internal class ManagedAudioSource(private val source: AudioSource) : MediaDataSource() {
    private var closed = false
    @Synchronized override fun getSize(): Long {
        if (closed) throw IOException("Audio closed")
        return source.size().toLong()
    }
    @Synchronized override fun readAt(position: Long, buffer: ByteArray, offset: Int, size: Int): Int {
        if (closed || position < 0 || offset < 0 || size < 0 || offset > buffer.size - size)
            throw IOException("Invalid audio read")
        if (size == 0) return 0
        try {
            val bytes = source.read(position.toULong(), size.coerceAtMost(65536).toUInt())
            if (bytes.isEmpty()) return -1
            bytes.copyInto(buffer, offset)
            return bytes.size
        } catch (error: Exception) { throw IOException("Audio unavailable", error) }
    }
    @Synchronized override fun close() {
        if (!closed) { closed = true; source.destroy() }
    }
}
