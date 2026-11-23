package org.tonalli.spike.ai

import java.io.IOException
import java.util.concurrent.atomic.AtomicReference
import kotlin.math.abs
import kotlin.math.sqrt

/** One explicit native embedding request; Core owns actual token admission. */
object AiEmbeddingProbe {
    data class Assessment(val dimensions: Int, val norm: Double)
    fun input(text: String): List<String> {
        if (text.isBlank() || '\u0000' in text || text.toByteArray(Charsets.UTF_8).size > 16384)
            throw IOException("Embedding text must be nonblank, contain no NUL and fit 16 KiB UTF-8; Core checks actual token bounds")
        return listOf(text)
    }
    fun assess(vectors: List<List<Float>>, dimensions: Int): Assessment {
        if (dimensions !in 1..4096 || vectors.size != 1 || vectors[0].size != dimensions || vectors[0].any { !it.isFinite() })
            throw IOException("Invalid owned embedding vector count, dimensions or finite values")
        val norm = sqrt(vectors[0].sumOf { it.toDouble() * it.toDouble() })
        if (!norm.isFinite() || abs(norm - 1.0) > 1e-3) throw IOException("Embedding vector is not L2 normalized")
        return Assessment(dimensions, norm)
    }
}

/** Native close may run independently; worker disposal waits for that exact close. */
class AiModelClosure(private val nativeClose: () -> Unit, private val destroy: () -> Unit) {
    private val lock = Any()
    private var nativeClosed = false
    private var destroyed = false
    fun requestClose() = synchronized(lock) {
        if (!nativeClosed) { nativeClosed = true; nativeClose() }
    }
    fun dispose() = synchronized(lock) {
        try { requestClose() } finally {
            if (!destroyed) { destroyed = true; destroy() }
        }
    }
}

/** Captures the exact native target; an older queued cancellation cannot close a new model. */
class AiEmbeddingRun(private val cancelled: (Long) -> Boolean) {
    private data class Target(val epoch: Long, val close: () -> Unit)
    private val current = AtomicReference<Target?>()
    fun publish(epoch: Long, close: () -> Unit): Boolean {
        check(current.compareAndSet(null, Target(epoch, close))) { "Embedding target already owned" }
        return !cancelled(epoch)
    }
    fun cancellation(): (() -> Unit)? = current.get()?.close
    fun clear(epoch: Long) {
        val target = current.get()
        if (target?.epoch == epoch) current.compareAndSet(target, null)
    }
}
