package org.tonalli.litert

import java.nio.file.Path
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.StandardOpenOption.*
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.security.MessageDigest

internal data class TrustedModel(val bytes: Long, val sha256: String) {
    companion object {
        val GEMMA = TrustedModel(2_588_147_712L, "181938105e0eefd105961417e8da75903eacda102c4fce9ce90f50b97139a63c")
    }
}

internal object ModelStaging {
    // Derived from Tonalli AiModelFiles; the isolated experiment uses LiteRT's filename.
    fun prepare(incoming: Path, privateDir: Path, spec: TrustedModel = TrustedModel.GEMMA,
                cancelled: () -> Boolean = { false },
                availableBytes: (Path) -> Long = { it.toFile().usableSpace }): Path {
        fun checkCancelled() { if (cancelled()) throw IOException("Model staging cancelled") }
        fun canonical(path: Path): Path {
            val absolute = path.toAbsolutePath().normalize()
            if (!Files.isDirectory(absolute, NOFOLLOW_LINKS) || absolute.toRealPath() != absolute)
                throw IOException("Model directory must be canonical and unlinked")
            return absolute
        }
        checkCancelled()
        if (spec.bytes <= 0 || !spec.sha256.matches(Regex("[a-f0-9]{64}"))) throw IOException("Invalid trusted model metadata")
        val root = canonical(privateDir)
        val published = root.resolve(spec.sha256)
        val destination = published.resolve("model.litertlm")
        if (Files.exists(published, NOFOLLOW_LINKS)) {
            canonical(published)
            verify(destination, spec, cancelled)
            return destination
        }
        val source = canonical(incoming).resolve("model.litertlm")
        if (!Files.isRegularFile(source, NOFOLLOW_LINKS)) throw IOException("Incoming model must be a regular file")
        val required = try { Math.addExact(spec.bytes, 32L * 1024 * 1024) }
            catch (error: ArithmeticException) { throw IOException("Invalid model staging size", error) }
        if (availableBytes(root) < required) throw IOException("Insufficient model staging space")
        val stage = Files.createTempDirectory(root, ".stage-")
        val staged = stage.resolve("model.litertlm")
        try {
            FileChannel.open(source, READ, NOFOLLOW_LINKS).use { input ->
                if (input.size() != spec.bytes) throw IOException("Model length mismatch")
                FileChannel.open(staged, CREATE_NEW, WRITE).use { output ->
                    val hash = MessageDigest.getInstance("SHA-256")
                    val buffer = ByteBuffer.allocate(64 * 1024)
                    var position = 0L
                    while (position < spec.bytes) {
                        checkCancelled()
                        buffer.clear().limit(minOf(buffer.capacity().toLong(), spec.bytes - position).toInt())
                        val count = input.read(buffer, position)
                        if (count <= 0) throw IOException("Model ended during staging")
                        buffer.flip()
                        hash.update(buffer.asReadOnlyBuffer())
                        var written = 0L
                        while (buffer.hasRemaining()) {
                            val n = output.write(buffer, position + written)
                            if (n <= 0) throw IOException("Model staging made no progress")
                            written += n
                        }
                        position += count
                    }
                    if (input.size() != spec.bytes || hex(hash.digest()) != spec.sha256) throw IOException("Model checksum mismatch")
                    output.force(true)
                }
            }
            checkCancelled()
            if (Files.exists(published, NOFOLLOW_LINKS)) throw IOException("Model publication already exists")
            Files.move(stage, published, ATOMIC_MOVE)
            return destination
        } finally {
            Files.deleteIfExists(staged)
            Files.deleteIfExists(stage)
        }
    }

    private fun verify(path: Path, spec: TrustedModel, cancelled: () -> Boolean) {
        if (!Files.isRegularFile(path, NOFOLLOW_LINKS)) throw IOException("Published model must be a regular file")
        FileChannel.open(path, READ, NOFOLLOW_LINKS).use { input ->
            if (input.size() != spec.bytes) throw IOException("Published model length mismatch")
            val hash = MessageDigest.getInstance("SHA-256")
            val buffer = ByteBuffer.allocate(64 * 1024)
            var position = 0L
            while (position < spec.bytes) {
                if (cancelled()) throw IOException("Model verification cancelled")
                buffer.clear().limit(minOf(buffer.capacity().toLong(), spec.bytes - position).toInt())
                val count = input.read(buffer, position)
                if (count <= 0) throw IOException("Published model ended during verification")
                buffer.flip(); hash.update(buffer); position += count
            }
            if (hex(hash.digest()) != spec.sha256 || input.size() != spec.bytes) throw IOException("Published model checksum mismatch")
            if (cancelled()) throw IOException("Model verification cancelled")
        }
    }

    private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }
}

internal class ProbeLease {
    private val state = Object()
    private val nativeOperations = Any()
    private var cancelled = false
    private var terminal = false
    private var closed = false
    private var cancelNative: (() -> Unit)? = null
    private var inFlight = 0

    fun start(cancelNative: () -> Unit): Boolean = synchronized(state) {
        check(this.cancelNative == null && !closed && !terminal)
        if (cancelled) return@synchronized false
        this.cancelNative = cancelNative
        true
    }
    // Native calls serialize separately; callbacks need only the brief state lock.
    fun cancel() {
        synchronized(state) {
            if (cancelled || terminal || closed) return
            cancelled = true
        }
        synchronized(nativeOperations) {
            val action = synchronized(state) {
                if (terminal || closed) null else cancelNative?.also { inFlight++ }
            }
            if (action != null) try { action() } finally { nativeReturned() }
        }
    }
    fun terminal() = synchronized(state) { terminal = true; state.notifyAll() }
    fun isTerminal(): Boolean = synchronized(state) { terminal }
    fun isCancelled(): Boolean = synchronized(state) { cancelled }
    fun sendIfActive(send: () -> Unit): Boolean = synchronized(nativeOperations) operations@{
        synchronized(state) {
            if (cancelled || terminal || closed) return@operations false
            check(cancelNative != null)
            inFlight++
        }
        try { send() } finally { nativeReturned() }
        true
    }
    fun awaitQuiescentTerminal() = synchronized(state) {
        while (!terminal || inFlight != 0) state.wait()
    }
    private fun nativeReturned() = synchronized(state) { inFlight--; state.notifyAll() }
    fun closeIfTerminal(close: () -> Unit): Boolean {
        synchronized(state) {
            if (!terminal || inFlight != 0 || closed) return false
            closed = true
        }
        close()
        return true
    }
}
