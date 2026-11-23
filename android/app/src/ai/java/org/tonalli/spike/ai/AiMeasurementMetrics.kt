package org.tonalli.spike.ai

import android.app.ActivityManager
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.BatteryManager
import android.os.Debug
import android.os.PowerManager
import android.os.Process
import java.io.FileOutputStream
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.nio.file.StandardCopyOption.REPLACE_EXISTING
import java.nio.file.StandardOpenOption.CREATE_NEW
import java.util.UUID
import org.json.JSONObject
import org.json.JSONArray
import android.os.SystemClock
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import uniffi.tonalli_core.GenerationEvent
import org.tonalli.spike.benchmark.BenchmarkFiles

/** Private app-process measurements, with missing platform readings left unavailable. */
class AiMeasurementMetrics(private val context: Context, spec: AiModelSpec) : AutoCloseable {
    private val root = context.filesDir.toPath().toRealPath().resolve("ai-measurements")
    val directory = run {
        if (!Files.exists(root, NOFOLLOW_LINKS)) Files.createDirectory(root)
        check(Files.isDirectory(root, NOFOLLOW_LINKS) && root.toRealPath() == root)
        Files.createDirectory(root.resolve(UUID.randomUUID().toString()))
    }
    private val result = BenchmarkFiles.claimResult(directory, "run")
    private val snapshots = Files.newBufferedWriter(directory.resolve("snapshots.csv"), CREATE_NEW)
    private val requests = Files.newBufferedWriter(directory.resolve("requests.csv"), CREATE_NEW)
    private val calibration = Files.newBufferedWriter(directory.resolve("calibration.csv"), CREATE_NEW)
    private var peakPssKb: Int? = null
    private val report = JSONObject().put("version", 1).put("status", "running")
        .put("model_id", spec.id).put("model_sha256", spec.sha256).put("model_bytes", spec.bytes)
        .put("pid", Process.myPid()).put("process_started_elapsed_ms", Process.getStartElapsedRealtime())
        .put("input_tokens", 2000).put("output_cap", 128).put("minimum_completed_request_wall_us", 600_000_000L)
        .put("scope", "foreground Android app process; sampled peak PSS; no screen-lock or other RAM-tier proof")
    init {
        requests.write("id,start_us,end_us,wall_us,input_tokens,output_tokens,setup_us,prefill_us,first_utf8_ready_us,first_sink_receipt_us,generation_phase_us,total_us,wall_tokens_s,generation_phase_tokens_s,reason,error_present\n")
        snapshots.write("scheduled_us,observed_us,latest_completed_id,latest_completed_wall_tokens_s,pss_kb,battery_temperature_tenths_c,plugged,thermal_status,available_ram_bytes,metrics_error\n")
        calibration.write("probe,repeats,input_tokens,reason,error_present,wall_us,prefill_us,first_utf8_ready_us,first_sink_receipt_us\n")
        val ram = ActivityManager.MemoryInfo()
        context.getSystemService(ActivityManager::class.java).getMemoryInfo(ram)
        report.put("total_ram_bytes", ram.totalMem).put("initial_available_ram_bytes", ram.availMem)
        writeReport(false)
    }
    fun calibration(probe: Int, repeats: Int, input: UInt, reason: String, error: Boolean,
                    wallUs: Long, prefillUs: ULong, firstReadyUs: ULong?, firstReceiptUs: Long?) {
        calibration.append("$probe,$repeats,$input,$reason,$error,$wallUs,$prefillUs,${firstReadyUs ?: ""},${firstReceiptUs ?: ""}\n")
        calibration.flush()
    }
    fun request(row: String) { requests.append(row).append('\n'); requests.flush() }
    fun snapshot(sample: AiMeasurementRun.Sample, latestId: ULong?, latestTps: Double?) {
        val errors = mutableListOf<String>()
        fun <T> reading(name: String, work: () -> T): T? = try { work() } catch (_: Exception) { errors.add(name); null }
        val pss = reading("pss") { Debug.MemoryInfo().also { Debug.getMemoryInfo(it) }.totalPss }
        if (pss != null) peakPssKb = maxOf(peakPssKb ?: 0, pss)
        val battery = reading("battery") { context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED)) }
        if (battery == null) errors.add("battery_unavailable") else {
            if (!battery.hasExtra(BatteryManager.EXTRA_TEMPERATURE)) errors.add("battery_temperature_unavailable")
            if (!battery.hasExtra(BatteryManager.EXTRA_PLUGGED)) errors.add("battery_plugged_unavailable")
        }
        val temperature = battery?.takeIf { it.hasExtra(BatteryManager.EXTRA_TEMPERATURE) }?.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, 0)
        val plugged = battery?.takeIf { it.hasExtra(BatteryManager.EXTRA_PLUGGED) }?.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0)
        val thermal = reading("thermal") { context.getSystemService(PowerManager::class.java).currentThermalStatus }
        val available = reading("ram") { ActivityManager.MemoryInfo().also { context.getSystemService(ActivityManager::class.java).getMemoryInfo(it) }.availMem }
        snapshots.append(listOf(sample.scheduledUs, sample.observedUs, latestId, latestTps, pss, temperature, plugged,
            thermal, available, errors.joinToString(";")).joinToString(",") { it?.toString() ?: "" }).append('\n')
        snapshots.flush()
    }
    fun finish(status: String, completedWallUs: Long, error: AiMeasurementRun.Failure? = null) {
        report.put("status", status).put("completed_request_wall_us", completedWallUs)
            .put("sampled_peak_pss_kb", peakPssKb ?: JSONObject.NULL).put("error", error?.errorClass ?: JSONObject.NULL)
            .put("failure", error?.let { JSONObject().put("class", it.errorClass)
                .put("reason", it.reason ?: JSONObject.NULL).put("stage", it.stage) } ?: JSONObject.NULL)
        writeReport(true)
    }
    private fun writeReport(replace: Boolean) {
        val temp = Files.createTempFile(directory, ".run-", ".json")
        try {
            FileOutputStream(temp.toFile()).use { it.write(report.toString(2).toByteArray()); it.fd.sync() }
            if (replace) {
                check(Files.isRegularFile(result, NOFOLLOW_LINKS))
                Files.move(temp, result, ATOMIC_MOVE, REPLACE_EXISTING)
            } else {
                check(!Files.exists(result, NOFOLLOW_LINKS))
                Files.move(temp, result, ATOMIC_MOVE)
            }
        } finally { Files.deleteIfExists(temp) }
    }
    override fun close() { try { snapshots.close() } finally { try { requests.close() } finally { calibration.close() } } }
}

/** Only explicit paired comparison fixtures persist raw output; callbacks touch memory only. */
class AiFixtureRecord(context: Context, spec: AiModelSpec, trainingContextTokens: UInt,
                      request: AiProbeRequest, actualPrompt: String) {
    val evidence = AiFixtureEvidence(checkNotNull(request.question), checkNotNull(request.context), actualPrompt, request.outputTokens)
    private val terminal = CountDownLatch(1)
    @Volatile private var terminalFailure: AiMeasurementRun.Failure? = null
    private val root = context.filesDir.toPath().toRealPath().resolve("ai-measurements")
    val directory = run {
        if (!Files.exists(root, NOFOLLOW_LINKS)) Files.createDirectory(root)
        check(Files.isDirectory(root, NOFOLLOW_LINKS) && root.toRealPath() == root)
        Files.createDirectory(root.resolve(UUID.randomUUID().toString()))
    }
    private val result = BenchmarkFiles.claimResult(directory, "fixture")
    private val report = JSONObject().put("version", 1).put("scope", "explicit offline comparison fixture; trace completeness is not a design gate PASS")
        .put("model_id", spec.id).put("model_sha256", spec.sha256).put("model_bytes", spec.bytes)
        .put("uid", Process.myUid()).put("pid", Process.myPid()).put("request_started_elapsed_ns", SystemClock.elapsedRealtimeNanos())
        .put("request_started_scope", "fixture worker before generation; not UI press")
        .put("model_load_included", false).put("retrieval_included", false).put("staging_included", false)
        .put("instruction_placement", "user prompt prefix; model-specific chat template and tokenizer")
        .put("configured_context_tokens", minOf(trainingContextTokens, 4096u).toLong())
        .put("configured_thread_ceiling", 4).put("actual_native_threads", JSONObject.NULL)
        .put("sampler_top_k", 20).put("sampler_top_p", 0.8).put("sampler_temperature", 0.7).put("sampler_seed", 42)
        .put("terminal_wait_deadline_seconds", 90).put("useful_answer_timing_scope", "unavailable; inspect raw output; no automatic useful-content classification")
    init { write("running", false) }

    fun started(event: GenerationEvent.Started) { evidence.started(event.id, SystemClock.elapsedRealtimeNanos()) }
    fun text(event: GenerationEvent.Text) { evidence.text(event.id, SystemClock.elapsedRealtimeNanos(), event.text) }
    fun finished(event: GenerationEvent.Finished) {
        val stats = event.stats
        val accepted = evidence.terminal(event.reason.toString(), event.error != null, event.reason.toString().equals("cancelled", true), mapOf(
            "input_tokens" to stats.inputTokens.toString(), "output_tokens" to stats.outputTokens.toString(),
            "setup_us" to stats.setupUs.toString(), "prefill_us" to stats.prefillUs.toString(),
            "first_utf8_ready_us" to stats.firstTokenUs.toString(), "decode_us" to stats.decodeUs.toString(), "total_us" to stats.totalUs.toString()),
            event.id, SystemClock.elapsedRealtimeNanos())
        if (accepted) {
            terminalFailure = event.error?.let { AiMeasurementRun.failure(java.io.IOException(it), "core_terminal") }
            terminal.countDown()
        }
    }
    fun cancellationRequested() { evidence.cancellationRequested() }

    fun awaitTerminal(cancel: () -> Unit) {
        val completed = try { terminal.await(90, TimeUnit.SECONDS) } catch (_: InterruptedException) { false }
        if (!completed) {
            evidence.cancellationRequested()
            var deferred: Exception? = null
            try { write("cancellation_requested_waiting_actual_terminal", true) } catch (error: Exception) { deferred = error }
            try { cancel() } catch (error: Exception) { if (deferred == null) deferred = error else deferred.addSuppressed(error) }
            while (true) {
                try { if (terminal.await(250, TimeUnit.MILLISECONDS)) break } catch (_: InterruptedException) { }
            }
            deferred?.let { throw it }
        }
    }

    fun finish() {
        check(evidence.snapshot().terminal != null) { "Cannot publish terminal evidence without Core terminal" }
        write("terminal", true)
    }
    fun failure(error: Throwable, stage: String = "fixture_submission_or_writer") {
        report.put("failure", failureJson(AiMeasurementRun.failure(error, stage)))
        write("incomplete", true)
    }

    private fun failureJson(error: AiMeasurementRun.Failure): JSONObject = JSONObject()
        .put("class", error.errorClass).put("reason", error.reason ?: JSONObject.NULL).put("stage", error.stage)

    private fun write(status: String, replace: Boolean) {
        val snapshot = evidence.snapshot()
        report.put("status", status).put("question_sha256", snapshot.questionSha256).put("context_sha256", snapshot.contextSha256)
            .put("prompt_sha256", snapshot.promptSha256).put("output_cap", snapshot.outputCap.toLong())
            .put("omitted_chunks", snapshot.omittedChunks).put("cancellation_requested", snapshot.cancellationRequested)
            .put("comparison_eligible", status == "terminal" && snapshot.comparisonEligible)
            .put("first_raw_received_ns", snapshot.firstRawNs ?: JSONObject.NULL).put("first_useful_answer_ns", JSONObject.NULL)
            .put("started", snapshot.started?.let { JSONObject().put("id", it.id.toString()).put("received_ns", it.receivedNs) } ?: JSONObject.NULL)
            .put("terminal", snapshot.terminal?.let { JSONObject().put("id", it.id?.toString() ?: JSONObject.NULL)
                .put("received_ns", it.receivedNs ?: JSONObject.NULL).put("reason", it.reason).put("error_present", it.errorPresent)
                .put("cancelled", it.cancelled).put("error", terminalFailure?.let(::failureJson) ?: JSONObject.NULL)
                .put("stats", JSONObject(it.stats)) } ?: JSONObject.NULL)
            .put("raw_chunks", JSONArray(snapshot.chunks.map { JSONObject().put("id", it.id.toString()).put("received_ns", it.receivedNs).put("text", it.text) }))
        val temp = Files.createTempFile(directory, ".fixture-", ".json")
        try {
            FileOutputStream(temp.toFile()).use { it.write(report.toString(2).toByteArray(Charsets.UTF_8)); it.fd.sync() }
            if (replace) {
                check(Files.isRegularFile(result, NOFOLLOW_LINKS))
                Files.move(temp, result, ATOMIC_MOVE, REPLACE_EXISTING)
            } else Files.move(temp, result, ATOMIC_MOVE)
        } finally { Files.deleteIfExists(temp) }
    }
}
