package org.tonalli.spike.index

import java.io.IOException
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.nio.file.StandardOpenOption.CREATE_NEW
import java.nio.file.StandardOpenOption.WRITE
import java.security.MessageDigest

/** Incoming reads are positioned; only complete verified private directories are activated. */
object TrustedIndexInstaller {
    fun install(incoming: Path, internal: Path, spec: IndexTransferSpec, cancelled: () -> Boolean,
        space: (Path) -> Long = { it.toFile().usableSpace },
        withSelection: (() -> Unit) -> Unit = { it() }, validate: (Path) -> Unit): Path =
        install(TrustedIndexInput.path(incoming), internal, spec, cancelled, space, withSelection, validate)

    fun install(incoming: TrustedIndexInput, internal: Path, spec: IndexTransferSpec, cancelled: () -> Boolean,
        space: (Path) -> Long = { it.toFile().usableSpace },
        withSelection: (() -> Unit) -> Unit = { it() }, validate: (Path) -> Unit): Path {
        fun active() { if (cancelled()) throw IOException("Index handoff cancelled") }
        active()
        val root = internal.toAbsolutePath().normalize()
        Files.createDirectories(root)
        directory(root)
        val installed = root.resolve("snapshot-${spec.manifestHash}")
        if (Files.exists(installed, NOFOLLOW_LINKS)) {
            verify(installed, spec, cancelled)
            validate(installed)
            withSelection { active() }
            return installed
        }
        incoming.checkContents(spec)
        // Only one private staged copy; provider bytes are never copied to an intermediate incoming folder.
        if (space(root) < spec.stageRequiredBytes) throw IOException("Insufficient space for private staged trusted index")
        requireDescriptor(incoming, spec.descriptor)
        val staged = Files.createTempDirectory(root, ".stage-")
        var published = false
        try {
            Files.createDirectory(staged.resolve(spec.generation))
            val buffer = ByteBuffer.allocate(65536)
            for (entry in spec.entries) {
                active()
                incoming.read(entry.path) { input ->
                    if (input.size() != entry.bytes) throw IOException("Trusted index file length differs")
                    FileChannel.open(staged.resolve(entry.path), WRITE, CREATE_NEW).use { output ->
                        var position = 0L
                        while (position < entry.bytes) {
                            active()
                            buffer.clear().limit(minOf(buffer.capacity().toLong(), entry.bytes - position).toInt())
                            val n = input.read(buffer, position)
                            if (n <= 0) throw IOException("Incoming index truncated or unavailable")
                            position += n; buffer.flip()
                            while (buffer.hasRemaining()) if (output.write(buffer) <= 0) throw IOException("Index staging write made no progress")
                        }
                        if (input.size() != entry.bytes) throw IOException("Incoming index changed during copy")
                        output.force(true)
                    }
                }
            }
            verify(staged, spec, cancelled)
            validate(staged)
            withSelection { active(); Files.move(staged, installed, ATOMIC_MOVE); published = true }
            return installed
        } finally {
            if (!published) {
                if (staged.parent != root || !staged.fileName.toString().startsWith(".stage-")) throw IOException("Invalid trusted index cleanup path")
                Files.walk(staged).use { it.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists) }
            }
        }
    }

    private fun verify(root: Path, spec: IndexTransferSpec, cancelled: () -> Boolean) {
        directory(root); directory(root.resolve(spec.generation))
        val buffer = ByteArray(65536)
        for (entry in spec.entries) {
            val path = root.resolve(entry.path)
            file(path, entry.bytes)
            val digest = MessageDigest.getInstance("SHA-256")
            var read = 0L
            Files.newInputStream(path, NOFOLLOW_LINKS).use { input -> while (true) {
                if (cancelled()) throw IOException("Index verification cancelled")
                val count = input.read(buffer)
                if (count < 0) break
                if (count == 0 || read > entry.bytes - count) throw IOException("Staged index changed")
                digest.update(buffer, 0, count); read += count
            } }
            if (read != entry.bytes || digest.digest().joinToString("") { "%02x".format(it) } != entry.sha256) throw IOException("Staged index checksum mismatch")
        }
    }
    private fun requireDescriptor(incoming: TrustedIndexInput, expected: ByteArray) {
        val actual = ByteBuffer.allocate(expected.size)
        incoming.read("snapshot.json") { input ->
            if (input.size() != expected.size.toLong()) throw IOException("Incoming descriptor length differs")
            while (actual.hasRemaining()) if (input.read(actual, actual.position().toLong()) <= 0) throw IOException("Truncated incoming descriptor")
            if (input.size() != expected.size.toLong()) throw IOException("Incoming descriptor changed")
        }
        if (!actual.array().contentEquals(expected)) throw IOException("Incoming descriptor differs from packaged trusted bytes")
    }
    private fun directory(path: Path) { if (!Files.isDirectory(path, NOFOLLOW_LINKS)) throw IOException("Invalid trusted index directory") }
    private fun file(path: Path, bytes: Long) {
        if (!Files.isRegularFile(path, NOFOLLOW_LINKS) || Files.size(path) != bytes) throw IOException("Trusted index file length differs")
    }
}
