package org.tonalli.spike.web

import java.io.ByteArrayOutputStream
import java.io.FileOutputStream
import java.io.IOException
import java.io.InputStream
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.Paths
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.security.MessageDigest

/** Immutable packaged UI versions stay separate from the private core data directory. */
object UiAssets {
    data class Entry(val path: String, val bytes: Long, val sha256: String)
    private const val MAX_BYTES = 32L * 1024 * 1024
    private val hashPattern = Regex("[a-f0-9]{64}")

    fun readManifest(input: InputStream): ByteArray {
        val output = ByteArrayOutputStream()
        val buffer = ByteArray(4096)
        while (true) {
            val count = input.read(buffer)
            if (count < 0) break
            if (count == 0 || output.size() > 512 * 1024 - count) throw IOException("Oversized or invalid UI manifest")
            output.write(buffer, 0, count)
        }
        return output.toByteArray()
    }

    @Synchronized // Recreated activities can overlap publication of the same verified version.
    fun install(internal: Path, id: String, entries: List<Entry>, open: (String) -> InputStream): Path {
        if (!hashPattern.matches(id) || entries.size !in 1..2000 || entries.none { it.path == "index.html" } ||
            entries.map { it.path }.distinct().size != entries.size) throw IOException("Invalid UI manifest")
        var total = 0L
        for (entry in entries) {
            if (entry.path.length !in 1..512 || entry.path.any { it <= '\u001f' || it == '\u007f' || it == '\\' || it == ':' } ||
                entry.path.split('/').any { it in setOf("", ".", "..") } || !hashPattern.matches(entry.sha256) ||
                entry.bytes < 0 || entry.bytes > MAX_BYTES || total > MAX_BYTES - entry.bytes) {
                throw IOException("Invalid UI asset: ${entry.path}")
            }
            total += entry.bytes
        }
        val root = internal.toAbsolutePath().normalize()
        Files.createDirectories(root)
        if (!Files.isDirectory(root, NOFOLLOW_LINKS)) throw IOException("Invalid internal UI directory")
        val installed = root.resolve("ui-$id")
        if (Files.exists(installed, NOFOLLOW_LINKS)) {
            verify(installed, entries)
            return installed
        }
        if (root.toFile().usableSpace < total + 1024 * 1024) throw IOException("Insufficient internal UI storage")
        val staged = Files.createTempDirectory(root, ".stage-")
        var published = false
        try {
            val buffer = ByteArray(64 * 1024)
            for (entry in entries) {
                val target = staged.resolve(entry.path)
                Files.createDirectories(target.parent)
                Files.createFile(target)
                open(entry.path).use { input ->
                    FileOutputStream(target.toFile()).use { output ->
                        var copied = 0L
                        while (true) {
                            val count = input.read(buffer)
                            if (count < 0) break
                            if (count == 0 || copied > entry.bytes - count) throw IOException("UI asset grew: ${entry.path}")
                            output.write(buffer, 0, count)
                            copied += count
                        }
                        if (copied != entry.bytes) throw IOException("Truncated UI asset: ${entry.path}")
                        output.fd.sync()
                    }
                }
            }
            verify(staged, entries)
            Files.move(staged, installed, ATOMIC_MOVE)
            published = true
            return installed
        } finally {
            if (!published) {
                if (staged.parent != root || !staged.fileName.toString().startsWith(".stage-")) throw IOException("Invalid UI cleanup path")
                Files.walk(staged).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists) }
            }
        }
    }

    private fun verify(root: Path, entries: List<Entry>) {
        if (!Files.isDirectory(root, NOFOLLOW_LINKS)) throw IOException("Invalid installed UI directory")
        val files = entries.map { it.path }.toSet()
        val directories = mutableSetOf("")
        for (entry in entries) {
            var parent = Paths.get(entry.path).parent
            while (parent != null) {
                directories.add(parent.toString().replace('\\', '/'))
                parent = parent.parent
            }
        }
        Files.walk(root).use { paths -> paths.forEach { path ->
            val relative = root.relativize(path).toString().replace('\\', '/')
            if (Files.isSymbolicLink(path) ||
                (Files.isDirectory(path, NOFOLLOW_LINKS) && relative !in directories) ||
                (!Files.isDirectory(path, NOFOLLOW_LINKS) && (!Files.isRegularFile(path, NOFOLLOW_LINKS) || relative !in files))) {
                throw IOException("Unexpected installed UI entry: $relative")
            }
        } }
        val buffer = ByteArray(64 * 1024)
        for (entry in entries) {
            val source = root.resolve(entry.path)
            if (!Files.isRegularFile(source, NOFOLLOW_LINKS) || Files.size(source) != entry.bytes) throw IOException("UI length mismatch: ${entry.path}")
            val hash = MessageDigest.getInstance("SHA-256")
            Files.newInputStream(source, NOFOLLOW_LINKS).use { input ->
                var read = 0L
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    if (count == 0 || read > entry.bytes - count) throw IOException("UI asset changed: ${entry.path}")
                    hash.update(buffer, 0, count)
                    read += count
                }
                if (read != entry.bytes) throw IOException("UI asset changed: ${entry.path}")
            }
            if (hash.digest().joinToString("") { "%02x".format(it) } != entry.sha256) throw IOException("UI checksum mismatch: ${entry.path}")
        }
    }
}
