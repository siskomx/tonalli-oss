package org.tonalli.spike.ai

import android.content.Context
import android.app.ActivityManager
import android.os.Process
import android.os.SystemClock
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.nio.file.StandardCopyOption.REPLACE_EXISTING
import java.security.MessageDigest
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject
import org.tonalli.spike.benchmark.BenchmarkDataset
import org.tonalli.spike.benchmark.BenchmarkFiles
import org.tonalli.spike.index.IndexTransferSpec
import org.tonalli.spike.index.PackagedIndexTransfer
import org.tonalli.spike.index.TrustedIndexInstaller
import org.tonalli.spike.saf.SafBenchmarkBinding
import org.tonalli.spike.saf.SafPackSession
import org.tonalli.spike.saf.SafSelectionStore
import uniffi.tonalli_core.AnswerOutcome
import uniffi.tonalli_core.AnswerRequest
import uniffi.tonalli_core.AnswerSource
import uniffi.tonalli_core.AnswerTimings
import uniffi.tonalli_core.GenerationEvent
import uniffi.tonalli_core.GenerationStats
import uniffi.tonalli_core.IndexHandle
import uniffi.tonalli_core.TokenSink
import uniffi.tonalli_core.aiCancel
import uniffi.tonalli_core.indexOpen
import uniffi.tonalli_core.indexSearch
import uniffi.tonalli_core.ramTier

/** Explicit SMALL fixture only; all actual retrieval, prompting and citations remain in Core. */
object AiAnswerProbe {
    private const val MAX_REPORT_BYTES = 1024 * 1024
    class Prepared internal constructor(internal val spec: IndexTransferSpec, internal val binding: SafBenchmarkBinding,
        internal val pack: SafPackSession, internal val index: IndexHandle) : AutoCloseable {
        private var closed = false // worker only
        fun requireCurrent(context: Context) {
            if (closed) throw IOException("Answer source is closed; prepare it again")
            SafSelectionStore(context).withCurrent(binding.record) { }
        }
        override fun close() {
            if (closed) return
            closed = true
            try { index.destroy() } finally { pack.close() }
        }
    }

    fun prepare(context: Context, cancelled: () -> Boolean): Prepared {
        val spec = PackagedIndexTransfer.load(context, "small")
        val catalog = BenchmarkDataset.load(context, "small")
        val binding = SafBenchmarkBinding.load(context, catalog)
        val store = SafSelectionStore(context)
        val root = File(context.filesDir, "index-handoff").toPath()
        val internal = root.resolve("small")
        if (Files.isSymbolicLink(root) || Files.isSymbolicLink(internal)) throw IOException("Linked answer index storage")
        val external = context.getExternalFilesDir(null) ?: throw IOException("Missing incoming index storage")
        val installed = TrustedIndexInstaller.install(external.toPath().resolve(spec.incomingName), internal, spec, cancelled,
            withSelection = { commit -> store.withCurrent(binding.record) { commit() } },
            validate = { staged ->
                binding.open(context, catalog, cancelled).use { pack ->
                    val index = indexOpen(staged.toString(), pack.handle)
                    try {
                        for (query in spec.queries) {
                            if (cancelled()) throw IOException("Answer preparation cancelled")
                            IndexTransferSpec.requireHits(query, indexSearch(index, query.query, 5u).map { IndexTransferSpec.Hit(it.title, it.url) })
                        }
                    } finally { index.destroy() }
                }
            })
        if (cancelled()) throw IOException("Answer preparation cancelled")
        val pack = binding.open(context, catalog, cancelled)
        try {
            val index = indexOpen(installed.toString(), pack.handle)
            return Prepared(spec, binding, pack, index)
        } catch (error: Throwable) {
            try { pack.close() } catch (cleanup: Throwable) { if (cleanup !== error) error.addSuppressed(cleanup) }
            throw error
        }
    }

    // No Activity, model, pack, index, Context or UI closures may be retained by this callback.
    private class GateSink(private val gate: AiRunGate, private val stamp: Long) : TokenSink {
        override fun onEvent(event: GenerationEvent) {
            when (event) {
                is GenerationEvent.Started -> gate.started(stamp, event.id).cancelId?.let { aiCancel(it) }
                is GenerationEvent.Text -> Unit // Core returns owned raw/final text on the worker.
                is GenerationEvent.Finished -> { gate.finished(stamp, event.id) }
            }
        }
    }

    fun run(context: Context, prepared: Prepared, model: AiNativeSession, selected: AiAnswerCase,
        gate: AiRunGate, stamp: Long): String {
        if (model.spec.role != "generation") throw IOException("Load a generation model for the cited answer")
        val raw = context.assets.open("ai-rerank-questions-v1.json").use { it.readNBytes(65537) }
        if (raw.size !in 1..65536) throw IOException("Invalid question catalog size")
        val authorityRaw = context.assets.open("ai-rerank-inputs-v1.json").use { it.readNBytes(8193) }
        if (authorityRaw.size !in 1..8192) throw IOException("Invalid answer authority size")
        val authority = JSONObject(authorityRaw.toString(Charsets.UTF_8))
        if (authority.getInt("version") != 1 || authority.getString("kind") != "tonalli-rerank-inputs") throw IOException("Invalid answer authority identity")
        val questionsHash = hash(raw)
        AiAnswerCase.requireAuthority(questionsHash, authority.getString("questions_file_sha256"),
            prepared.spec.source.expectedSha256, authority.getString("source_pack_sha256"))
        val catalog = JSONObject(raw.toString(Charsets.UTF_8))
        if (catalog.getInt("version") != 1 || catalog.getString("retrieval_language") != "en") throw IOException("Invalid question catalog identity")
        val rows = catalog.getJSONArray("questions")
        if (rows.length() != 10) throw IOException("Expected the ten fixed questions")
        val input = selected.select((0 until rows.length()).map { number -> rows.getJSONObject(number).let {
            AiAnswerCase.Question(it.getString("id"), it.getString("question_en"), it.getString("question_es"), it.getString("retrieval_query_en"))
        } })
        val directory = File(context.filesDir, "ai-answer").toPath()
        Files.createDirectories(directory)
        if (!Files.isDirectory(directory, NOFOLLOW_LINKS)) throw IOException("Invalid private answer evidence directory")
        val output = BenchmarkFiles.claimResult(directory, "answer-${UUID.randomUUID()}")
        val memory = ActivityManager.MemoryInfo()
        context.getSystemService(ActivityManager::class.java).getMemoryInfo(memory)
        val report = JSONObject().put("version", 1).put("status", "running").put("dataset_id", "small")
            .put("case_id", selected.id).put("question_language", selected.language).put("question", input.question)
            .put("retrieval_query_en", input.retrievalQuery).put("spanish_retrieval_evaluated", false)
            .put("retrieval_mode", "bm25_only").put("embedding_model", JSONObject.NULL).put("max_tokens", 128)
            .put("question_catalog_sha256", questionsHash).put("authority_asset_sha256", hash(authorityRaw))
            .put("index_manifest_sha256", prepared.spec.manifestHash)
            .put("source_pack_sha256", prepared.spec.source.expectedSha256)
            .put("source_evidence", "bound SAF manual receipt; current content not re-SHA-scanned")
            .put("model_id", model.spec.id).put("model_sha256", model.spec.sha256)
            .put("model_bytes", model.spec.bytes).put("training_context_tokens", model.info.trainingContextTokens.toLong())
            .put("pid", Process.myPid()).put("uid", Process.myUid())
            .put("total_ram_bytes", memory.totalMem).put("initial_available_ram_bytes", memory.availMem)
            .put("actual_ram_tier", ramTier(memory.totalMem.coerceAtLeast(0).toULong()).name)
            .put("ram_sample_scope", "after model load and SMALL preparation, before ai_answer")
            .put("model_retained", true)
            .put("session_scope", "worker retains generation weights and prepared SMALL handles for subsequent cases until explicit close/departure; failure releases source/index only; returned status makes no cleanup or LMK claim")
            .put("model_load_included", false).put("preparation_included", false)
            .put("timing_scope", "request_wall_us covers the synchronous JNI call; Core first_raw_received_us is raw callback receipt, not visible answer latency; generation_phase_us includes post-prefill sampling/UTF8/enqueue time")
            .put("scope", "explicit fixed-question functional evidence; no automatic quality/performance gate PASS")
        publish(output, report, replace = false)
        var phase = "source_selection"
        try {
            if (gate.cancelled(stamp)) throw IOException("Cited answer cancelled before submission")
            prepared.requireCurrent(context) // The selection lock ends before Core or callbacks can run.
            phase = "core_answer"
            val started = SystemClock.elapsedRealtimeNanos()
            val outcome = model.answer(prepared.index, prepared.pack.handle,
                AnswerRequest(input.question, input.retrievalQuery, null, selected.maxTokens), GateSink(gate, stamp))
            report.put("request_wall_us", (SystemClock.elapsedRealtimeNanos() - started) / 1000)
            val summary = when (outcome) {
                is AnswerOutcome.NoSources -> {
                    report.put("outcome", "no_sources").put("timings", timings(outcome.timings))
                    "No sources found; no generated answer."
                }
                is AnswerOutcome.Observed -> {
                    val answer = outcome.answer
                    val flags = answer.flags
                    val flagRecord = JSONObject().put("invalid_citations", flags.invalidCitations).put("uncited", flags.uncited)
                        .put("nonempty_thinking", flags.nonemptyThinking).put("unfinished_thinking", flags.unfinishedThinking)
                        .put("truncated", flags.truncated).put("incomplete", flags.incomplete)
                    report.put("outcome", "observed").put("id", outcome.id.toString()).put("raw_text", answer.rawText)
                        .put("text", answer.text).put("sources", sources(answer.sources))
                        .put("citations", JSONArray(answer.citations.map { it.toLong() })).put("flags", flagRecord)
                        .put("end", answer.end.toString()).put("error", outcome.error ?: JSONObject.NULL)
                        .put("terminal_forwarded", outcome.terminalForwarded).put("stats", stats(outcome.stats))
                        .put("timings", timings(outcome.timings))
                    "${answer.text}\nFlags: $flagRecord\nEnd: ${answer.end}; error_present=${outcome.error != null}\n" +
                        answer.sources.joinToString("\n") { "[${it.number}] ${it.title} — ${it.url}" }
                }
                is AnswerOutcome.Deadline -> {
                    report.put("outcome", "deadline").put("id", outcome.id?.toString() ?: JSONObject.NULL)
                        .put("raw_text", outcome.rawText).put("sources", sources(outcome.sources))
                        .put("terminal_forwarded", outcome.terminalForwarded).put("timings", timings(outcome.timings))
                    val terminal = outcome.terminal
                    if (terminal is GenerationEvent.Finished) report.put("terminal", JSONObject().put("reason", terminal.reason.toString())
                        .put("error", terminal.error ?: JSONObject.NULL).put("stats", stats(terminal.stats)))
                    else report.put("terminal", JSONObject.NULL)
                    "Core answer deadline reached; incomplete raw evidence only. No completed answer is claimed."
                }
            }
            phase = "source_selection_after_answer"
            report.put("status", "outcome_received")
            publish(output, report)
            prepared.requireCurrent(context)
            report.put("status", "returned").put("cancelled_or_departed", gate.cancelled(stamp))
            publish(output, report)
            return "Fixed ${selected.id}/${selected.language}, SMALL, BM25 only, cap128.\n$summary\nEvidence: ai-answer/${output.fileName}"
        } catch (error: Throwable) {
            report.put("status", "failed").put("phase", phase).put("error_type", error.javaClass.simpleName)
                .put("error", error.message?.take(1024) ?: JSONObject.NULL)
            try { publish(output, report) } catch (cleanup: Throwable) { if (cleanup !== error) error.addSuppressed(cleanup) }
            throw error
        }
    }

    private fun sources(values: List<AnswerSource>) = JSONArray(values.map {
        JSONObject().put("number", it.number.toLong()).put("title", it.title).put("url", it.url).put("excerpt", it.excerpt)
    })
    private fun timings(value: AnswerTimings) = JSONObject().put("retrieval_us", value.retrievalUs.toString())
        .put("extraction_us", value.extractionUs.toString()).put("rerank_us", value.rerankUs?.toString() ?: JSONObject.NULL)
        .put("first_raw_received_us", value.firstRawReceivedUs?.toString() ?: JSONObject.NULL).put("total_us", value.totalUs.toString())
    private fun stats(value: GenerationStats) = JSONObject().put("input_tokens", value.inputTokens.toLong())
        .put("output_tokens", value.outputTokens.toLong()).put("setup_us", value.setupUs.toString()).put("prefill_us", value.prefillUs.toString())
        .put("first_utf8_ready_us", value.firstTokenUs?.toString() ?: JSONObject.NULL)
        .put("generation_phase_us", value.decodeUs.toString()).put("total_us", value.totalUs.toString())
    private fun hash(value: ByteArray) = MessageDigest.getInstance("SHA-256").digest(value).joinToString("") { "%02x".format(it) }
    private fun publish(output: Path, report: JSONObject, replace: Boolean = true) {
        if (if (replace) !Files.isRegularFile(output, NOFOLLOW_LINKS) else Files.exists(output, NOFOLLOW_LINKS)) throw IOException("Invalid owned answer report target")
        val bytes = report.toString(2).toByteArray(Charsets.UTF_8)
        if (bytes.size > MAX_REPORT_BYTES) throw IOException("Answer report exceeds bounded receipt size")
        val staged = Files.createTempFile(output.parent, ".answer-", ".json")
        try {
            FileOutputStream(staged.toFile()).use { it.write(bytes); it.fd.sync() }
            if (replace) Files.move(staged, output, ATOMIC_MOVE, REPLACE_EXISTING)
            else Files.move(staged, output, ATOMIC_MOVE)
        } finally { Files.deleteIfExists(staged) }
    }
}
