package org.tonalli.spike.benchmark

import java.io.FileOutputStream
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

/** Only explicit preparation reads external payloads; benchmark opens use the verified immutable receipt. */
object BenchmarkFiles {
    data class Entry(val name: String, val bytes: Long, val sha256: String)
    private const val MAX_ZIM = 64L * 1024 * 1024 * 1024
    private const val MAX_TABLE = 2L * 1024 * 1024 * 1024
    private val hashPattern = Regex("[a-f0-9]{64}")

    /** A run ID is single-use even if its Activity is recreated or work is cancelled. */
    fun claimResult(root: Path, runId: String): Path {
        if (!Regex("[A-Za-z0-9][A-Za-z0-9_-]{0,63}").matches(runId) || !Files.isDirectory(root, NOFOLLOW_LINKS)) {
            throw IOException("Invalid benchmark result reservation")
        }
        val output = root.resolve("$runId.json")
        if (Files.exists(output, NOFOLLOW_LINKS)) throw IOException("Benchmark result already exists")
        // Never remove this claim on failure/interruption: a retry needs a new ID.
        FileChannel.open(root.resolve("$runId.claim"), WRITE, CREATE_NEW, NOFOLLOW_LINKS).use { it.force(true) }
        return output
    }

    fun prepare(incoming: Path, internal: Path, hash: String, entries: List<Entry>, cancelled: () -> Boolean,
        space: (Path) -> Long = { it.toFile().usableSpace }, validate: (Path) -> Unit): Path {
        validateManifest(hash, entries)
        return prepareEntries(incoming, internal, hash, entries, cancelled, space, validate)
    }

    /** SAF preparation copies only the folded-title table; validate attaches it to the selected pack. */
    fun prepareTable(incoming: Path, internal: Path, hash: String, entry: Entry, cancelled: () -> Boolean,
        space: (Path) -> Long = { it.toFile().usableSpace }, validate: (Path) -> Unit): Path {
        validateTable(hash, entry)
        return prepareEntries(incoming, internal, hash, listOf(entry), cancelled, space, validate)
    }

    private fun prepareEntries(incoming: Path, internal: Path, hash: String, entries: List<Entry>, cancelled: () -> Boolean,
        space: (Path) -> Long, validate: (Path) -> Unit): Path {
        fun checkActive() { if (cancelled()) throw IOException("Preparation cancelled") }
        checkActive()
        val root = internal.toAbsolutePath().normalize()
        Files.createDirectories(root)
        if (!Files.isDirectory(root, NOFOLLOW_LINKS)) throw IOException("Invalid internal benchmark directory")
        val installed = root.resolve(hash)
        if (Files.exists(installed, NOFOLLOW_LINKS)) {
            preparedEntries(root, hash, entries)
            verify(installed, entries, cancelled)
            validate(installed)
            checkActive()
            return installed
        }
        if (!Files.isDirectory(incoming, NOFOLLOW_LINKS)) throw IOException("Missing external benchmark input")
        if (space(root) < entries.sumOf { it.bytes } + 32L * 1024 * 1024) throw IOException("Insufficient internal benchmark storage")
        val staged = Files.createTempDirectory(root, ".stage-")
        var published = false
        try {
            val buffer = ByteBuffer.allocate(64 * 1024)
            for (entry in entries) {
                val source = incoming.resolve(entry.name)
                if (!Files.isRegularFile(source, NOFOLLOW_LINKS) || Files.size(source) != entry.bytes) throw IOException("Benchmark input length mismatch")
                FileChannel.open(source, READ, NOFOLLOW_LINKS).use { input ->
                    FileChannel.open(staged.resolve(entry.name), WRITE, CREATE_NEW).use { output ->
                        var position = 0L
                        while (position < entry.bytes) {
                            checkActive()
                            buffer.clear().limit(minOf(buffer.capacity().toLong(), entry.bytes - position).toInt())
                            val count = input.read(buffer, position)
                            if (count <= 0) throw IOException("Truncated or unavailable external benchmark input")
                            position += count
                            buffer.flip()
                            while (buffer.hasRemaining()) if (output.write(buffer) <= 0) throw IOException("Benchmark write made no progress")
                        }
                        if (input.size() != entry.bytes) throw IOException("Benchmark input changed during preparation")
                        output.force(true)
                    }
                }
            }
            verify(staged, entries, cancelled)
            validate(staged)
            checkActive()
            FileOutputStream(staged.resolve("PREPARED").toFile()).use { output ->
                output.write("$hash\n".toByteArray(Charsets.US_ASCII)); output.fd.sync()
            }
            checkActive()
            Files.move(staged, installed, ATOMIC_MOVE)
            published = true
            return installed
        } finally {
            if (!published) {
                if (staged.parent != root || !staged.fileName.toString().startsWith(".stage-")) throw IOException("Invalid benchmark cleanup path")
                Files.walk(staged).use { it.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists) }
            }
        }
    }

    fun prepared(internal: Path, hash: String, entries: List<Entry>): Path {
        validateManifest(hash, entries)
        return preparedEntries(internal, hash, entries)
    }

    fun preparedTable(internal: Path, hash: String, entry: Entry): Path {
        validateTable(hash, entry)
        return preparedEntries(internal, hash, listOf(entry))
    }

    private fun preparedEntries(internal: Path, hash: String, entries: List<Entry>): Path {
        val root = internal.toAbsolutePath().normalize()
        val installed = root.resolve(hash)
        if (!Files.isDirectory(root, NOFOLLOW_LINKS) || !Files.isDirectory(installed, NOFOLLOW_LINKS)) throw IOException("Dataset is not prepared")
        val receipt = installed.resolve("PREPARED")
        if (!Files.isRegularFile(receipt, NOFOLLOW_LINKS) || Files.size(receipt) != 65L ||
            Files.readAllBytes(receipt).toString(Charsets.US_ASCII) != "$hash\n") throw IOException("Missing verified preparation receipt")
        for (entry in entries) {
            val file = installed.resolve(entry.name)
            if (!Files.isRegularFile(file, NOFOLLOW_LINKS) || Files.size(file) != entry.bytes) throw IOException("Prepared dataset changed")
        }
        return installed
    }

    private fun validateManifest(hash: String, entries: List<Entry>) {
        if (!hashPattern.matches(hash) || entries.size != 2 || entries.map { it.name }.toSet() != setOf("pack.zim", "titles.bin") ||
            entries.any { !hashPattern.matches(it.sha256) || it.bytes <= 0 || it.bytes > if (it.name == "pack.zim") MAX_ZIM else MAX_TABLE }) {
            throw IOException("Invalid trusted benchmark manifest")
        }
    }

    private fun validateTable(hash: String, entry: Entry) {
        if (!hashPattern.matches(hash) || entry.name != "titles.bin" || !hashPattern.matches(entry.sha256) ||
            entry.bytes <= 0 || entry.bytes > MAX_TABLE) throw IOException("Invalid trusted benchmark table manifest")
    }

    private fun verify(root: Path, entries: List<Entry>, cancelled: () -> Boolean) {
        val buffer = ByteArray(64 * 1024)
        for (entry in entries) {
            val file = root.resolve(entry.name)
            if (!Files.isRegularFile(file, NOFOLLOW_LINKS) || Files.size(file) != entry.bytes) throw IOException("Benchmark copy length mismatch")
            val digest = MessageDigest.getInstance("SHA-256")
            var read = 0L
            Files.newInputStream(file, NOFOLLOW_LINKS).use { input ->
                while (true) {
                    if (cancelled()) throw IOException("Preparation cancelled")
                    val count = input.read(buffer)
                    if (count < 0) break
                    if (count == 0 || read > entry.bytes - count) throw IOException("Benchmark copy changed")
                    digest.update(buffer, 0, count); read += count
                }
            }
            if (read != entry.bytes || digest.digest().joinToString("") { "%02x".format(it) } != entry.sha256) throw IOException("Benchmark checksum mismatch")
        }
    }
}
