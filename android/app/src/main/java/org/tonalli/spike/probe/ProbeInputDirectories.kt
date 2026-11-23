package org.tonalli.spike.probe

import java.io.IOException
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import org.tonalli.spike.index.IndexSnapshot

/** Only creates transfer directories; never changes ownership, payloads or installed inputs. */
object ProbeInputDirectories {
    fun paths(hash: String, generation: String, entries: List<IndexSnapshot.Entry>, datasets: List<String>): List<String> {
        IndexSnapshot.validateManifest(hash, generation, entries)
        if (datasets.isEmpty() || datasets.size > 32 || datasets.distinct().size != datasets.size ||
            datasets.any { !Regex("[a-z][a-z0-9_-]{0,31}").matches(it) }) throw IOException("Invalid packaged benchmark dataset list")
        val incoming = IndexSnapshot.incomingName(hash)
        return listOf(incoming, "$incoming/$generation", "benchmark") + datasets.sorted().map { "benchmark/$it" }
    }

    fun prepare(root: Path, paths: List<String>, owned: (Path) -> Boolean) {
        fun requireOwned(path: Path) {
            if (!Files.isDirectory(path, NOFOLLOW_LINKS) || !owned(path) || !Files.isReadable(path) || !Files.isWritable(path)) {
                throw IOException("Transfer directory is unavailable or not app-owned; existing contents were preserved")
            }
        }
        requireOwned(root)
        for (relative in paths) {
            if (relative.isEmpty() || relative.split('/').any { it.isEmpty() || it == "." || it == ".." || '\\' in it || ':' in it }) {
                throw IOException("Invalid trusted transfer directory")
            }
            var current = root
            for (part in relative.split('/')) {
                current = current.resolve(part)
                if (!Files.exists(current, NOFOLLOW_LINKS)) Files.createDirectory(current)
                requireOwned(current)
            }
        }
    }
}
