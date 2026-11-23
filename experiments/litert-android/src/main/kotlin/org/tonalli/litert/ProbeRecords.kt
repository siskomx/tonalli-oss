package org.tonalli.litert

import android.os.SystemClock
import org.json.JSONObject
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardOpenOption.CREATE_NEW
import java.nio.file.StandardOpenOption.WRITE
import java.nio.channels.FileChannel
import java.nio.channels.Channels
import java.nio.ByteBuffer
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.nio.file.StandardCopyOption.REPLACE_EXISTING
import java.util.UUID
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.atomic.AtomicInteger

/** Callbacks enqueue bounded data only; one worker writes private JSONL. */
internal class ProbeRecords(privateRoot: Path) : AutoCloseable {
    class CapacityException : java.io.IOException("Probe record capacity reached; preserve existing evidence before another run")
    private data class Event(val nanos: Long, val kind: String, val text: String, val channels: Map<String, String>, val fields: Map<String, String>)
    private val queue = ConcurrentLinkedQueue<Event>()
    private val queued = AtomicInteger()
    private val dropped = AtomicInteger()
    private val truncated = AtomicInteger()
    private var omittedAtFileLimit = 0
    private var writeFailures = 0
    private var written = 0L
    private var failed = false
    val id = UUID.randomUUID().toString()
    private val writer: java.io.BufferedWriter
    private val channel: FileChannel
    private val statusPath: Path

    init {
        val directory = privateRoot.resolve("litert-probe-records")
        if (!Files.exists(directory, NOFOLLOW_LINKS)) Files.createDirectory(directory)
        check(Files.isDirectory(directory, NOFOLLOW_LINKS) && directory.toRealPath() == directory) { "Record directory must be canonical and unlinked" }
        Files.list(directory).use { files ->
            val count = files.filter { Files.isRegularFile(it, NOFOLLOW_LINKS) && it.fileName.toString().matches(Regex("[a-f0-9-]{36}\\.jsonl")) }.count()
            if (count >= 32) throw CapacityException()
        }
        statusPath = directory.resolve("$id.status.json")
        channel = FileChannel.open(directory.resolve("$id.jsonl"), CREATE_NEW, WRITE)
        writer = Channels.newWriter(channel, Charsets.UTF_8.newEncoder(), -1).buffered()
    }

    fun offer(kind: String, text: String, channels: Map<String, String> = emptyMap(), fields: Map<String, String> = emptyMap()) {
        if (queued.incrementAndGet() > 256) { queued.decrementAndGet(); dropped.incrementAndGet(); return }
        if (kind.length > 80 || text.length > 32768 || channels.size > 8 || fields.size > 24 ||
            channels.any { it.key.length > 80 || it.value.length > 32768 } || fields.any { it.key.length > 80 || it.value.length > 1024 }) truncated.incrementAndGet()
        queue.add(Event(SystemClock.elapsedRealtimeNanos(), kind.take(80), text.take(32768),
            channels.entries.take(8).associate { it.key.take(80) to it.value.take(32768) }, fields.entries.take(24).associate { it.key.take(80) to it.value.take(1024) }))
    }

    fun flush(display: (String) -> Unit, durable: Boolean = false) {
        while (true) {
            val event = queue.poll() ?: break
            queued.decrementAndGet()
            display("${event.kind}: ${event.text}")
            if (failed) continue
            val line = JSONObject().put("run_id", id).put("monotonic_ns", event.nanos)
                .put("event", event.kind).put("text", event.text).put("channels", JSONObject(event.channels))
                .put("fields", JSONObject(event.fields)).toString() + "\n"
            val length = line.toByteArray(Charsets.UTF_8).size
            if (written + length > 2L * 1024 * 1024) {
                omittedAtFileLimit++
                continue
            }
            try { writer.write(line); written += length }
            catch (error: java.io.IOException) { failed = true; writeFailures++; display("JSONL write failed: ${error.message}") }
        }
        if (!failed) try { writer.flush(); if (durable) channel.force(true) }
        catch (error: java.io.IOException) { failed = true; writeFailures++; display("JSONL flush failed: ${error.message}") }
        persistStatus(false, display)
    }

    fun finish(completed: Boolean, display: (String) -> Unit) {
        flush(display, durable = true)
        persistStatus(completed, display)
    }

    fun checkpoint(display: (String) -> Unit) {
        flush(display, durable = true)
        check(!failed) { "Durable evidence checkpoint failed; inference refused" }
    }

    private fun persistStatus(completed: Boolean, display: (String) -> Unit) {
        val eligible = completed && !failed && dropped.get() == 0 && truncated.get() == 0 && omittedAtFileLimit == 0 && writeFailures == 0
        val status = JSONObject().put("run_id", id).put("monotonic_ns", SystemClock.elapsedRealtimeNanos())
            .put("completed", completed).put("comparison_eligible", eligible).put("dropped_records", dropped.get())
            .put("truncated_records", truncated.get()).put("file_limit_omissions", omittedAtFileLimit).put("write_failures", writeFailures)
        val temp = statusPath.resolveSibling("$id.status-stage")
        try {
            FileChannel.open(temp, CREATE_NEW, WRITE).use { file ->
                val bytes = ByteBuffer.wrap((status.toString() + "\n").toByteArray(Charsets.UTF_8))
                while (bytes.hasRemaining()) check(file.write(bytes) > 0)
                file.force(true)
            }
            Files.move(temp, statusPath, ATOMIC_MOVE, REPLACE_EXISTING)
        } catch (error: Exception) {
            failed = true; writeFailures++
            display("Evidence status write failed; comparison ineligible: ${error.message}")
        } finally { try { Files.deleteIfExists(temp) } catch (_: java.io.IOException) { failed = true; writeFailures++ } }
    }

    override fun close() { writer.close() }
}
