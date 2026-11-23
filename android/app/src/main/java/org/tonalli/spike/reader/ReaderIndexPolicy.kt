package org.tonalli.spike.reader

import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import org.tonalli.spike.index.IndexTransferSpec

/** Selects only an installed trusted handoff bound to this freshly reopened archive. */
object ReaderIndexPolicy {
    fun candidate(filesRoot: Path, spec: IndexTransferSpec, identity: ReaderIdentity, partSizes: List<Long?>): Path? {
        if (!matches(spec, identity, partSizes)) return null
        val root = filesRoot.toAbsolutePath().normalize()
        val handoff = root.resolve("index-handoff")
        val dataset = handoff.resolve(spec.dataset)
        val snapshot = dataset.resolve("snapshot-${spec.manifestHash}")
        if (listOf(root, handoff, dataset, snapshot).any { !Files.isDirectory(it, NOFOLLOW_LINKS) || Files.isSymbolicLink(it) }) return null
        return snapshot
    }

    fun matches(spec: IndexTransferSpec, identity: ReaderIdentity, partSizes: List<Long?>): Boolean {
        if (identity.uuid != spec.source.uuid || identity.storedMd5 != spec.source.md5 || partSizes.isEmpty()) return false
        var bytes = 0L
        for (size in partSizes) {
            if (size == null || size < 0 || bytes > Long.MAX_VALUE - size) return false
            bytes += size
        }
        return bytes == spec.source.bytes
    }
}
