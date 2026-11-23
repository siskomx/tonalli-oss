package org.tonalli.spike.index

import java.io.FileOutputStream
import java.io.IOException
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.security.MessageDigest

/** Bounded copy of this spike's immutable index; never maps external files. */
object IndexSnapshot {
    data class Entry(val path: String, val bytes: Long, val sha256: String)
    private const val MAX_BYTES = 32L * 1024 * 1024
    private val hashPattern = Regex("[a-f0-9]{64}")

    fun incomingName(manifestHash: String): String {
        if (!hashPattern.matches(manifestHash)) throw IOException("Invalid trusted manifest hash")
        return "probe-index-${manifestHash.take(12)}"
    }

    fun install(
        incoming: Path,
        internal: Path,
        manifestHash: String,
        generation: String,
        entries: List<Entry>,
        validate: (Path) -> Unit,
    ): Path {
        val total = validateManifest(manifestHash, generation, entries)
        val root = internal.toAbsolutePath().normalize()
        Files.createDirectories(root)
        requireDirectory(root)
        val installed = root.resolve("snapshot-$manifestHash")
        if (Files.exists(installed, NOFOLLOW_LINKS)) {
            verify(installed, generation, entries)
            validate(installed)
            return installed
        }
        requireDirectory(incoming)
        requireDirectory(incoming.resolve(generation))
        val allowed = entries.map { it.path }.toSet() + "snapshot.json"
        Files.walk(incoming).use { paths -> paths.forEach { path ->
            val relative = incoming.relativize(path).toString().replace('\\', '/')
            if (Files.isSymbolicLink(path) ||
                (Files.isDirectory(path, NOFOLLOW_LINKS) && relative !in setOf("", generation)) ||
                (!Files.isDirectory(path, NOFOLLOW_LINKS) && relative !in allowed)) {
                throw IOException("Unexpected snapshot entry: $relative")
            }
        } }
        if (root.toFile().usableSpace < total + 1024 * 1024) {
            throw IOException("Insufficient internal space for the small index")
        }
        val staged = Files.createTempDirectory(root, ".stage-")
        var published = false
        try {
            Files.createDirectory(staged.resolve(generation))
            for (entry in entries) {
                val source = incoming.resolve(entry.path)
                requireFile(source)
                Files.newInputStream(source, NOFOLLOW_LINKS).use { input ->
                    FileOutputStream(staged.resolve(entry.path).toFile()).use { output ->
                        val buffer = ByteArray(64 * 1024)
                        var copied = 0L
                        while (true) {
                            val count = input.read(buffer)
                            if (count < 0) break
                            if (count == 0) throw IOException("Snapshot source made no progress")
                            if (copied > entry.bytes - count) throw IOException("Snapshot file grew: ${entry.path}")
                            output.write(buffer, 0, count)
                            copied += count
                        }
                        if (copied != entry.bytes) throw IOException("Truncated snapshot: ${entry.path}")
                        output.fd.sync()
                    }
                }
            }
            verify(staged, generation, entries)
            validate(staged)
            Files.move(staged, installed, ATOMIC_MOVE)
            published = true
            return installed
        } finally {
            if (!published) {
                if (staged.parent != root || !staged.fileName.toString().startsWith(".stage-")) {
                    throw IOException("Refusing cleanup outside the staging directory")
                }
                Files.walk(staged).use { paths ->
                    paths.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists)
                }
            }
        }
    }

    internal fun validateManifest(hash: String, generation: String, entries: List<Entry>): Long {
        if (!hashPattern.matches(hash) || !Regex("generation-[A-Za-z0-9_-]{1,64}").matches(generation) ||
            entries.size !in 2..128 || entries.map { it.path }.distinct().size != entries.size ||
            entries.none { it.path == "CURRENT" }) throw IOException("Invalid snapshot manifest")
        var total = 0L
        for (entry in entries) {
            val name = entry.path.removePrefix("$generation/")
            if ((entry.path != "CURRENT" && (name == entry.path ||
                    !Regex("[A-Za-z0-9._-]{1,160}").matches(name) || name in setOf(".", ".."))) ||
                name in setOf(".tantivy-meta.lock", ".tantivy-writer.lock") ||
                !hashPattern.matches(entry.sha256) || entry.bytes < 0 ||
                entry.bytes > MAX_BYTES || total > MAX_BYTES - entry.bytes ||
                (entry.path == "CURRENT" && entry.bytes > 256)) {
                throw IOException("Invalid snapshot record: ${entry.path}")
            }
            total += entry.bytes
        }
        return total
    }

    private fun verify(root: Path, generation: String, entries: List<Entry>) {
        requireDirectory(root)
        requireDirectory(root.resolve(generation))
        for (entry in entries) {
            val path = root.resolve(entry.path)
            requireFile(path)
            if (Files.size(path) != entry.bytes) throw IOException("Snapshot length mismatch: ${entry.path}")
            val hash = MessageDigest.getInstance("SHA-256")
            Files.newInputStream(path, NOFOLLOW_LINKS).use { input ->
                val buffer = ByteArray(64 * 1024)
                var read = 0L
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    if (count == 0 || read > entry.bytes - count) throw IOException("Snapshot changed: ${entry.path}")
                    hash.update(buffer, 0, count)
                    read += count
                }
                if (read != entry.bytes) throw IOException("Snapshot changed: ${entry.path}")
            }
            val actual = hash.digest().joinToString("") { "%02x".format(it) }
            if (actual != entry.sha256) throw IOException("Snapshot checksum mismatch: ${entry.path}")
        }
        if (Files.readAllBytes(root.resolve("CURRENT")).toString(Charsets.UTF_8).trim() != generation) {
            throw IOException("Snapshot CURRENT does not select its generation")
        }
    }

    private fun requireDirectory(path: Path) {
        if (!Files.isDirectory(path, NOFOLLOW_LINKS)) throw IOException("Not a regular snapshot directory: $path")
    }

    private fun requireFile(path: Path) {
        if (!Files.isRegularFile(path, NOFOLLOW_LINKS)) throw IOException("Not a regular snapshot file: $path")
    }
}
