package org.tonalli.spike.saf

import android.os.ParcelFileDescriptor
import java.io.IOException
import uniffi.tonalli_core.PackHandle
import uniffi.tonalli_core.PackPart
import uniffi.tonalli_core.TitleHit
import uniffi.tonalli_core.isoOpenFd
import uniffi.tonalli_core.packClose
import uniffi.tonalli_core.packOpenFds
import uniffi.tonalli_core.zimArticle
import uniffi.tonalli_core.zimSuggest

/** Worker-owned source. A removed/closed source is never retried with old descriptors. */
class SafPackSession private constructor(internal val handle: PackHandle, val partSizes: List<Long?>) : AutoCloseable {
    private var closed = false
    fun suggest(prefix: String): List<TitleHit> { checkOpen(); return zimSuggest(handle, prefix, 10u) }
    fun article(url: String): String {
        checkOpen()
        val article = zimArticle(handle, url)
        return "Article read through provider positioned reads: ${article.bytes.size} bytes (${article.mimeType})."
    }
    private fun checkOpen() { if (closed) throw IOException("Source is closed; explicitly reopen") }
    override fun close() {
        if (closed) return
        closed = true
        try { packClose(handle) } finally { handle.destroy() }
    }

    companion object {
        fun open(documents: SafDocuments, selection: SafSelection, cancelled: () -> Boolean = { false },
            validateParts: (List<Long?>) -> Unit = {}): SafPackSession {
            val resolved = documents.resolve(selection)
            val sizes = mutableListOf<Long?>()
            val handle = DescriptorTransfer.open(resolved, cancelled,
                opener = { document ->
                    val descriptor = documents.open(document)
                    try { sizes.add(descriptor.statSize.takeIf { it >= 0 }) }
                    catch (error: Throwable) { descriptor.close(); throw error }
                    object : TransferDescriptor {
                        override fun detachFd(): Int = descriptor.detachFd()
                        override fun close() = descriptor.close()
                    }
                },
                closeDetached = { fd -> ParcelFileDescriptor.adoptFd(fd).close() },
                nativeCall = { detached, transfer ->
                    validateParts(sizes.toList())
                    val parts = detached.map { PackPart(it.name, it.fd) }
                    if (selection.kind == SafKind.ISO) {
                        val part = parts.single()
                        val path = selection.innerPath ?: throw IOException("Explicit ISO inner path is required")
                        transfer()
                        isoOpenFd(part, path)
                    } else {
                        transfer()
                        packOpenFds(parts)
                    }
                })
            return SafPackSession(handle, sizes.toList())
        }
    }
}
