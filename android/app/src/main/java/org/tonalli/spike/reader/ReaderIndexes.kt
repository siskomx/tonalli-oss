package org.tonalli.spike.reader

import android.content.Context
import java.io.IOException
import org.tonalli.spike.index.PackagedIndexTransfer
import org.tonalli.spike.saf.SafPackSession
import uniffi.tonalli_core.LoopbackHandle
import uniffi.tonalli_core.indexOpen
import uniffi.tonalli_core.serverRegisterIndex

/** Optional body search. The registry retains its own index/source references after registration. */
internal object ReaderIndexes {
    fun attach(context: Context, server: LoopbackHandle, entry: ReaderEntry, pack: SafPackSession, cancelled: () -> Boolean) {
        // These are the only packaged authorities today; arbitrary manifests/directories are not trusted.
        for (dataset in listOf("small", "full", "medicine")) {
            if (cancelled()) throw IOException("Reader closed")
            try {
                val spec = PackagedIndexTransfer.load(context, dataset)
                val path = ReaderIndexPolicy.candidate(context.filesDir.toPath(), spec, entry.identity, pack.partSizes) ?: continue
                val index = indexOpen(path.toString(), pack.handle)
                try {
                    if (cancelled()) throw IOException("Reader closed")
                    serverRegisterIndex(server, entry.id, index)
                    return
                } finally { index.destroy() }
            } catch (error: Exception) {
                if (cancelled()) throw IOException("Reader closed", error)
                // Missing or incompatible optional indexes never withdraw the registered library.
            }
        }
    }
}
