package org.tonalli.spike.saf

import java.io.IOException

interface TransferDescriptor : AutoCloseable { fun detachFd(): Int }
data class DetachedDocument(val name: String, val fd: Int)

object DescriptorTransfer {
    fun <T> open(
        documents: List<SafDocument>,
        cancelled: () -> Boolean,
        opener: (SafDocument) -> TransferDescriptor,
        closeDetached: (Int) -> Unit,
        nativeCall: (List<DetachedDocument>, () -> Unit) -> T,
    ): T {
        val wrappers = mutableListOf<TransferDescriptor>()
        val detached = mutableListOf<DetachedDocument>()
        var transferred = false
        var failure: Throwable? = null
        fun checkCancelled() { if (cancelled()) throw IOException("Descriptor transfer cancelled") }
        try {
            checkCancelled()
            if (documents.isEmpty() || documents.size > 128) throw IOException("Invalid document count")
            for (document in documents) { checkCancelled(); wrappers.add(opener(document)) }
            for ((index, wrapper) in wrappers.withIndex()) {
                checkCancelled()
                val fd = wrapper.detachFd()
                if (fd < 0) throw IOException("Invalid detached descriptor")
                detached.add(DetachedDocument(documents[index].name, fd))
            }
            // Finish wrapper cleanup before Rust can return a handle that the caller must own.
            while (wrappers.isNotEmpty()) wrappers.removeAt(wrappers.lastIndex).close()
            checkCancelled()
            val result = nativeCall(detached.toList()) {
                if (transferred) throw IOException("Descriptors already transferred")
                checkCancelled()
                // Call only immediately before entering Rust, which consumes all FDs even on error.
                transferred = true
            }
            if (!transferred) throw IOException("Native ownership was not accepted")
            return result
        } catch (error: Throwable) {
            failure = error
            throw error
        } finally {
            var cleanupFailure: Throwable? = null
            fun clean(action: () -> Unit) {
                try { action() } catch (error: Throwable) {
                    val previous = cleanupFailure
                    if (previous == null) cleanupFailure = error else previous.addSuppressed(error)
                }
            }
            if (!transferred) for (fd in detached.map { it.fd }.distinct()) clean { closeDetached(fd) }
            for (wrapper in wrappers.asReversed()) clean { wrapper.close() }
            cleanupFailure?.let { error ->
                val previous = failure
                if (previous == null) throw error else previous.addSuppressed(error)
            }
        }
    }
}
