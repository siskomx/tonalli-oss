package org.tonalli.spike.ai

import android.app.ActivityManager
import android.content.Context
import android.os.Debug
import android.os.Process
import android.os.SystemClock
import java.io.FileOutputStream
import java.io.IOException
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.nio.file.StandardCopyOption.REPLACE_EXISTING
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import org.json.JSONArray
import org.json.JSONObject
import org.tonalli.spike.benchmark.BenchmarkFiles
import uniffi.tonalli_core.ramTier

/** Disposable, already-loaded BGE fixture probe. It never opens source URLs or model paths. */
object AiRerankProbe {
    private fun hash(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
    private data class Prepared(val authority: AiRerankInput.Authority, val verified: AiRerankInput.Verified,
                                val decoded: AiRerankInput.Decoded, val assetSha256: String, val prefixSource: String)

    private fun prepare(context: Context, query: String, rawContext: String, spec: AiModelSpec): Prepared {
        val asset = context.assets.open("ai-rerank-inputs-v1.json").use { it.readBytes() }
        if (asset.size !in 1..8192) throw IOException("Invalid rerank authority asset size")
        val source = JSONObject(asset.toString(Charsets.UTF_8))
        val model = source.getJSONObject("embedding_model")
        val pins = source.getJSONArray("questions")
        if (pins.length() != 10 || source.getInt("catalog_entries_verified") != 54) throw IOException("Invalid rerank authority entries")
        val authority = AiRerankInput.Authority(source.getInt("version"), source.getString("kind"),
            source.getString("catalog_sha256"), source.getString("source_pack_sha256"), source.getString("source_index_sha256"),
            source.getString("questions_file_sha256"), AiRerankInput.ModelPin(model.getString("id"), model.getLong("bytes"), model.getString("sha256")),
            source.getString("query_prefix"), (0 until pins.length()).map {
                pins.getJSONObject(it).let { pin -> AiRerankInput.QuestionPin(pin.getString("id"), pin.getInt("raw_question_json_bytes"), pin.getString("sha256")) }
            })
        if (rawContext.length > 65536) throw IOException("Rerank context exceeds bounded raw JSON size")
        val raw = rawContext.toByteArray(Charsets.UTF_8)
        // Authenticate exact bytes before parsing; do not canonicalize or rewrite the payload.
        val pin = AiRerankInput.verifyRaw(authority, raw)
        val json = JSONObject(rawContext)
        val question = json.getJSONObject("question")
        val dataset = json.getJSONObject("dataset")
        val passages = json.getJSONArray("passages")
        if (passages.length() != 20) throw IOException("Rerank fixture requires twenty unchanged passages")
        val decoded = AiRerankInput.Decoded(json.getInt("version"), question.getString("id"), question.getString("question_en"),
            question.getString("retrieval_query_en"), dataset.getString("pack_sha256"), dataset.getString("index_manifest_sha256"),
            json.getString("retrieval_language"), json.getBoolean("spanish_retrieval_quality_evaluated"),
            (0 until passages.length()).map { passages.getJSONObject(it).let { passage ->
                AiRerankInput.Passage(passage.getInt("bm25_rank"), passage.getString("url"), passage.getString("text"), passage.getString("passage_sha256"))
            } })
        val verified = AiRerankInput.verifyParsed(authority, pin, query, decoded,
            AiRerankInput.ModelPin(spec.id, spec.bytes, spec.sha256, spec.role))
        return Prepared(authority, verified, decoded, hash(asset), source.getString("query_prefix_source_url"))
    }

    fun run(context: Context, model: AiNativeSession, query: String, rawContext: String,
            cancelled: () -> Boolean, release: () -> Unit): String {
        var report: Report? = null
        var released = false
        var stage = "rerank_input"
        fun releaseOnce() { if (!released) { released = true; release() } }
        try {
            if (cancelled()) throw IOException("Rerank cancelled before submission")
            if (model.spec.role != "embedding") throw IOException("Rerank requires the verified BGE embedding model")
            val input = prepare(context, query, rawContext, model.spec)
            val activeReport = Report(context, model.spec, input)
            report = activeReport
            stage = "rerank_sampling_start"
            activeReport.startSamples()
            stage = "rerank_core_call"
            AiRerankInput.series(cancelled, SystemClock::elapsedRealtimeNanos, {
                val hits = model.rerank(input.verified.query, input.verified.documents)
                AiRerankInput.Ranking(hits.map { it.index }, hits.map { it.score })
            }, activeReport::row)
            stage = "rerank_sampling_stop"
            activeReport.stopSamples()
            stage = "rerank_model_release"
            releaseOnce()
            if (cancelled()) throw IOException("Rerank cancelled; completed rows are retained")
            stage = "rerank_complete_write"
            activeReport.complete()
            return "Three rerank Core calls recorded: ai-measurements/${activeReport.directory.fileName}/rerank.json. Model closed. This is not a gate PASS."
        } catch (error: Exception) {
            runCatching { report?.failure(error, stage) }
            throw error
        } finally {
            try { report?.stopSamples() } catch (error: Exception) { runCatching { report?.failure(error, "rerank_sampling_stop") } }
            try { releaseOnce() } catch (error: Exception) {
                runCatching { report?.failure(error, "rerank_model_release") }
                throw error
            }
        }
    }

    private class Report(private val context: Context, spec: AiModelSpec, private val input: Prepared) {
        private val root = context.filesDir.toPath().toRealPath().resolve("ai-measurements")
        val directory = run {
            if (!Files.exists(root, NOFOLLOW_LINKS)) Files.createDirectory(root)
            check(Files.isDirectory(root, NOFOLLOW_LINKS) && root.toRealPath() == root)
            Files.createDirectory(root.resolve(UUID.randomUUID().toString()))
        }
        private val output = BenchmarkFiles.claimResult(directory, "rerank")
        private val rows = JSONArray()
        private val samples = mutableListOf<JSONObject>()
        private val sampleLock = Any()
        private val sampler = Executors.newSingleThreadScheduledExecutor()
        private var scheduled: java.util.concurrent.ScheduledFuture<*>? = null
        private var samplesStopped = false
        private var omittedSamples = 0
        private var peakObservedPssKb: Int? = null
        private val cadenceNs = 500_000_000L
        private val workerStartNs = SystemClock.elapsedRealtimeNanos()
        private var samplingStartNs = 0L
        private var sampleOrdinal = 0L
        private val report = JSONObject().put("version", 1).put("scope", "three synchronous Core/FFI rerank calls; no automatic quality/performance gate PASS")
            .put("fixture_id", input.verified.fixtureId).put("authority_asset_sha256", input.assetSha256)
            .put("catalog_sha256", input.authority.catalogSha256).put("source_pack_sha256", input.authority.packSha256)
            .put("source_index_sha256", input.authority.indexSha256).put("questions_file_sha256", input.authority.questionsSha256)
            .put("raw_context_sha256", input.verified.rawContextSha256).put("query_sha256", hash(input.verified.query.toByteArray(Charsets.UTF_8)))
            .put("query_prefix", input.authority.queryPrefix).put("query_prefix_source_url", input.prefixSource)
            .put("model_id", spec.id).put("model_sha256", spec.sha256).put("model_bytes", spec.bytes)
            .put("uid", Process.myUid()).put("pid", Process.myPid()).put("worker_started_elapsed_ns", workerStartNs)
            .put("model_load_included", false).put("staging_included", false).put("retrieval_included", false).put("extraction_included", false)
            .put("call_timer_scope", "synchronous public Core/FFI call including marshalling/owned-result copy; validation/record I/O excluded")
            .put("input_tokens", JSONObject.NULL).put("input_tokens_missing_reason", "rerank API returns rankings, not native token counts")
            .put("configured_thread_ceiling", 4).put("actual_native_threads", JSONObject.NULL)
            .put("pss_cadence_ns", cadenceNs).put("pss_scope", "this app process; observed maximum only; no true-peak guarantee; sampling may perturb timings")
            .put("spanish_retrieval_quality_evaluated", false)
        init {
            val memory = ActivityManager.MemoryInfo()
            context.getSystemService(ActivityManager::class.java).getMemoryInfo(memory)
            report.put("total_ram_bytes", memory.totalMem).put("initial_available_ram_bytes", memory.availMem)
                .put("actual_ram_tier", ramTier(memory.totalMem.coerceAtLeast(0).toULong()).name)
            write("running", false)
        }
        fun startSamples() {
            samplingStartNs = SystemClock.elapsedRealtimeNanos()
            report.put("sampling_started_elapsed_ns", samplingStartNs)
            sample(samplingStartNs, "before_calls")
            scheduled = sampler.scheduleAtFixedRate({
                sampleOrdinal++
                sample(samplingStartNs + sampleOrdinal * cadenceNs, "cadence")
            }, 500, 500, TimeUnit.MILLISECONDS)
        }
        private fun sample(scheduledNs: Long, phase: String) {
            val row = JSONObject().put("phase", phase).put("scheduled_elapsed_ns", scheduledNs).put("observed_elapsed_ns", SystemClock.elapsedRealtimeNanos())
            var pss: Int? = null
            try {
                pss = Debug.MemoryInfo().also { Debug.getMemoryInfo(it) }.totalPss.takeIf { it > 0 }
                row.put("pss_kb", pss ?: JSONObject.NULL).put("missing_reason", if (pss == null) "zero_or_unavailable_pss" else JSONObject.NULL)
            } catch (error: Exception) {
                val failure = AiMeasurementRun.failure(error, "rerank_pss_sample")
                row.put("pss_kb", JSONObject.NULL).put("missing_reason", "${failure.errorClass}: ${failure.reason ?: "unavailable"}")
            }
            synchronized(sampleLock) {
                if (samples.size >= 1024) { omittedSamples++; return }
                samples.add(row)
                pss?.let { peakObservedPssKb = maxOf(peakObservedPssKb ?: 0, it) }
            }
        }
        fun stopSamples() {
            if (samplesStopped) return
            scheduled?.cancel(false)
            sampler.shutdown()
            if (!sampler.awaitTermination(2, TimeUnit.SECONDS)) throw IOException("PSS sampler stop timed out; observations incomplete")
            samplesStopped = true
            sample(SystemClock.elapsedRealtimeNanos(), "after_calls")
        }
        fun row(value: AiRerankInput.Measurement) {
            val ranking = value.ranking.indices.indices.map { position ->
                val index = value.ranking.indices[position].toInt()
                val passage = input.decoded.passages[index]
                JSONObject().put("position", position + 1).put("index", index).put("score", value.ranking.scores[position])
                    .put("bm25_rank", passage.rank).put("url", passage.url).put("passage_sha256", passage.sha256)
            }
            rows.put(JSONObject().put("repeat", value.repeat).put("duration_ns", value.durationNs).put("ranking", JSONArray(ranking)))
            write("running", true)
        }
        fun complete() {
            check(rows.length() == 3 && samplesStopped)
            write("complete", true)
        }
        fun failure(error: Throwable, stage: String) {
            val value = AiMeasurementRun.failure(error, stage)
            report.put("failure", JSONObject().put("class", value.errorClass).put("reason", value.reason ?: JSONObject.NULL).put("stage", value.stage))
            write("incomplete", true)
        }
        private fun write(status: String, replace: Boolean) {
            report.put("status", status).put("rows", rows)
            synchronized(sampleLock) {
                report.put("pss_samples", JSONArray(samples.toList())).put("pss_omitted_samples", omittedSamples)
                    .put("pss_observed_max_kb", peakObservedPssKb ?: JSONObject.NULL)
                    .put("pss_missing_reason", if (peakObservedPssKb == null) "no successful app-process PSS observation" else JSONObject.NULL)
                    .put("trace_complete", status == "complete" && omittedSamples == 0)
            }
            val bytes = report.toString(2).toByteArray(Charsets.UTF_8)
            if (bytes.size > 2 * 1024 * 1024) throw IOException("Rerank receipt exceeds bounded record size")
            val temp = Files.createTempFile(directory, ".rerank-", ".json")
            try {
                FileOutputStream(temp.toFile()).use { it.write(bytes); it.fd.sync() }
                if (replace) {
                    check(Files.isRegularFile(output, NOFOLLOW_LINKS))
                    Files.move(temp, output, ATOMIC_MOVE, REPLACE_EXISTING)
                } else Files.move(temp, output, ATOMIC_MOVE)
            } finally { Files.deleteIfExists(temp) }
        }
    }
}
