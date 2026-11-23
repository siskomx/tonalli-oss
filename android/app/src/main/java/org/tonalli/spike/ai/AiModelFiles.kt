package org.tonalli.spike.ai

import java.io.IOException
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.nio.file.StandardOpenOption.CREATE_NEW
import java.nio.file.StandardOpenOption.READ
import java.nio.file.StandardOpenOption.WRITE
import java.security.MessageDigest

/** Publishes one verified internal GGUF; activation and lifetime belong to its caller. */
object AiModelFiles {
    const val RESERVE_BYTES = 32L * 1024 * 1024

    /** Installed-only access: never creates directories, stages input or repairs a failed pin. */
    fun installed(appFilesDir: Path, spec: AiModelSpec, cancelled: () -> Boolean = { false }): Path {
        if (cancelled()) throw IOException("AI model verification cancelled")
        val root = canonicalDirectory(appFilesDir.toRealPath())
        val ai = canonicalDirectory(root.resolve("ai"))
        val models = canonicalDirectory(ai.resolve("models"))
        val published = canonicalDirectory(models.resolve(spec.sha256))
        return published.resolve("model.gguf").also { verify(it, spec, cancelled) }
    }

    fun requiredAdditionalBytes(spec: AiModelSpec): Long = try {
        Math.addExact(spec.bytes, RESERVE_BYTES)
    } catch (error: ArithmeticException) {
        throw IOException("AI staging space exceeds supported size", error)
    }

    fun prepare(incoming: Path, appFilesDir: Path, spec: AiModelSpec,
                cancelled: () -> Boolean = { false },
                availableBytes: (Path) -> Long = { it.toFile().usableSpace }): Path {
        fun checkCancelled() { if (cancelled()) throw IOException("AI model staging cancelled") }
        checkCancelled()
        // Context supplies these roots; Android may spell them through OS-owned aliases.
        val root = canonicalDirectory(appFilesDir.toRealPath())
        val models = directory(directory(root.resolve("ai")).resolve("models"))
        val published = models.resolve(spec.sha256)
        val destination = published.resolve("model.gguf")
        if (Files.exists(published, NOFOLLOW_LINKS)) {
            canonicalDirectory(published)
            verify(destination, spec, cancelled)
            return destination
        }
        val source = canonicalDirectory(incoming.toRealPath()).resolve("model.gguf")
        if (!Files.isRegularFile(source, NOFOLLOW_LINKS)) throw IOException("AI incoming model is not a regular file")
        if (availableBytes(root) < requiredAdditionalBytes(spec)) throw IOException("Insufficient additional AI staging space")
        val stage = Files.createTempDirectory(models, ".stage-")
        val staged = stage.resolve("model.gguf")
        try {
            FileChannel.open(source, READ, NOFOLLOW_LINKS).use { input ->
                if (input.size() != spec.bytes) throw IOException("AI model length differs from trusted catalog")
                FileChannel.open(staged, CREATE_NEW, WRITE).use { output ->
                    val hash = MessageDigest.getInstance("SHA-256")
                    val buffer = ByteBuffer.allocate(64 * 1024)
                    var position = 0L
                    while (position < spec.bytes) {
                        checkCancelled()
                        buffer.clear().limit(minOf(buffer.capacity().toLong(), spec.bytes - position).toInt())
                        val read = input.read(buffer, position)
                        if (read <= 0) throw IOException("AI model ended during staging")
                        buffer.flip()
                        hash.update(buffer.asReadOnlyBuffer())
                        var written = 0L
                        while (buffer.hasRemaining()) {
                            val count = output.write(buffer, position + written)
                            if (count <= 0) throw IOException("AI model staging made no progress")
                            written += count
                        }
                        position += read
                    }
                    if (input.size() != spec.bytes || hex(hash.digest()) != spec.sha256) throw IOException("AI model checksum or length mismatch")
                    output.force(true)
                }
            }
            checkCancelled()
            // Nonempty published directories cannot be replaced by an atomic directory rename.
            if (Files.exists(published, NOFOLLOW_LINKS)) throw IOException("AI model publication already exists")
            Files.move(stage, published, ATOMIC_MOVE)
            return destination
        } finally {
            Files.deleteIfExists(staged)
            Files.deleteIfExists(stage)
        }
    }

    private fun verify(path: Path, spec: AiModelSpec, cancelled: () -> Boolean) {
        if (!Files.isRegularFile(path, NOFOLLOW_LINKS)) throw IOException("Published AI model is not a regular file")
        FileChannel.open(path, READ, NOFOLLOW_LINKS).use { input ->
            if (input.size() != spec.bytes) throw IOException("Published AI model length mismatch")
            val hash = MessageDigest.getInstance("SHA-256")
            val buffer = ByteBuffer.allocate(64 * 1024)
            var position = 0L
            while (position < spec.bytes) {
                if (cancelled()) throw IOException("AI model verification cancelled")
                buffer.clear().limit(minOf(buffer.capacity().toLong(), spec.bytes - position).toInt())
                val count = input.read(buffer, position)
                if (count <= 0) throw IOException("Published AI model ended during verification")
                buffer.flip()
                hash.update(buffer)
                position += count
            }
            if (input.size() != spec.bytes || hex(hash.digest()) != spec.sha256) throw IOException("Published AI model checksum mismatch")
            if (cancelled()) throw IOException("AI model verification cancelled")
        }
    }

    private fun canonicalDirectory(path: Path): Path {
        val normalized = path.toAbsolutePath().normalize()
        if (!Files.isDirectory(normalized, NOFOLLOW_LINKS) || normalized.toRealPath() != normalized) throw IOException("AI directory must be canonical and unlinked")
        return normalized
    }

    private fun directory(path: Path): Path {
        if (!Files.exists(path, NOFOLLOW_LINKS)) Files.createDirectory(path)
        return canonicalDirectory(path)
    }

    private fun hex(bytes: ByteArray): String = bytes.joinToString("") { "%02x".format(it) }
}
