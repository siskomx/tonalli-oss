package org.tonalli.spike

import android.os.ParcelFileDescriptor
import java.io.File
import uniffi.tonalli_core.PackHandle
import uniffi.tonalli_core.PackPart
import uniffi.tonalli_core.packClose
import uniffi.tonalli_core.packOpenFds

internal inline fun <T> withPack(file: File, block: (PackHandle) -> T): T {
    val descriptor = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
    // Rust consumes this detached descriptor on success and failure.
    val fd = descriptor.use { it.detachFd() }
    val pack = packOpenFds(listOf(PackPart(file.name, fd)))
    return try { block(pack) } finally {
        try { packClose(pack) } finally { pack.destroy() }
    }
}
