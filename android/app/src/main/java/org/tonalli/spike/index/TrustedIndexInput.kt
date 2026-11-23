package org.tonalli.spike.index

import java.io.IOException
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardOpenOption.READ

/** One descriptor at a time; consumers use positioned reads and never retain the channel. */
interface TrustedIndexInput {
    fun checkContents(spec: IndexTransferSpec)
    fun read(path: String, consume: (FileChannel) -> Unit)

    companion object {
        fun requirePaths(spec: IndexTransferSpec, paths: List<String>) {
            val expected = spec.entries.map { it.path }.toSet() + setOf("snapshot.json", spec.generation)
            if (paths.size > IndexTransferSpec.MAX_FILES + 2 || paths.size != expected.size || paths.toSet() != expected) {
                throw IOException("Index folder differs from the trusted file list")
            }
        }
        fun path(root: Path): TrustedIndexInput = object : TrustedIndexInput {
            override fun checkContents(spec: IndexTransferSpec) {
                if (!Files.isDirectory(root, NOFOLLOW_LINKS)) throw IOException("Invalid incoming index directory")
                val paths = mutableListOf<String>()
                Files.walk(root).use { walk -> walk.forEach { current ->
                    if (current != root) {
                        if (paths.size >= IndexTransferSpec.MAX_FILES + 2 || Files.isSymbolicLink(current)) throw IOException("Invalid index entry")
                        val relative = root.relativize(current).toString().replace('\\', '/')
                        if (Files.isDirectory(current, NOFOLLOW_LINKS) != (relative == spec.generation) ||
                            (!Files.isDirectory(current, NOFOLLOW_LINKS) && !Files.isRegularFile(current, NOFOLLOW_LINKS))) throw IOException("Invalid index entry type")
                        paths.add(relative)
                    }
                } }
                requirePaths(spec, paths)
            }
            override fun read(path: String, consume: (FileChannel) -> Unit) {
                FileChannel.open(root.resolve(path), READ, NOFOLLOW_LINKS).use(consume)
            }
        }
    }
}
