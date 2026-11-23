package org.tonalli.spike.index

import android.content.Context
import android.os.Build
import android.os.Process
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject
import org.tonalli.spike.benchmark.BenchmarkDataset
import org.tonalli.spike.benchmark.BenchmarkFiles
import org.tonalli.spike.benchmark.BenchmarkSeries
import org.tonalli.spike.saf.SafBenchmarkBinding
import org.tonalli.spike.saf.SafManualReceipt

/** Private timing metadata only: no SAF URIs, credentials or source content. */
class IndexQueryReport(context: Context, private val spec: IndexTransferSpec, catalog: BenchmarkDataset,
    binding: SafBenchmarkBinding) {
    val results = spec.queries.map { IndexQuerySeries.Result(it) }
    val setup = JSONObject().put("copy_and_checksum", 0.0).put("source_open", 0.0).put("index_open", 0.0)
    var parts: JSONArray = binding.metadata()
    private val root = File(context.filesDir, "index-handoff-results").toPath()
    private val output: Path
    val relativePath: String get() = "files/index-handoff-results/${output.fileName}"
    private val report = JSONObject().put("version", 1).put("benchmark_kind", "warm_native_index_search")
        .put("dataset", spec.dataset).put("manifest_sha256", spec.manifestHash)
        .put("benchmark_manifest_sha256", catalog.manifestHash).put("source", "saf_pread")
        .put("index_storage", "app_internal_verified_snapshot")
        .put("selection", binding.selection()).put("external_manual", SafManualReceipt.projection(binding.manual, binding.receiptHash))
        .put("expected_zim_sha256", spec.source.expectedSha256).put("expected_zim_bytes", spec.source.bytes)
        .put("percentile_method", "nearest_rank").put("expected_samples_per_query", 30).put("expected_priming_per_query", 1)
        .put("os_cache", "untouched").put("cache_state", "uncontrolled_os_cache")
        .put("excluded_setup", JSONArray(listOf("transfer", "copy", "checksum_verification", "source_open", "index_open")))
        .put("api_level", Build.VERSION.SDK_INT).put("pid", Process.myPid())
        .put("process_started_elapsed_ms", Process.getStartElapsedRealtime())
        .put("dataset_acceptance_eligible", catalog.eligible).put("eligibility_scope", "dataset_only")
        .put("phone_acceptance", "not_evaluated")
        .put("measurement_role", if (catalog.eligible) "warm_query_measurement" else "functional_smoke_only")

    init {
        Files.createDirectories(root)
        if (!Files.isDirectory(root, NOFOLLOW_LINKS)) throw IOException("Invalid private index report directory")
        output = BenchmarkFiles.claimResult(root, UUID.randomUUID().toString())
    }

    fun publish(success: Boolean, phase: String, error: Throwable? = null) {
        if (success && results.any { !it.completed || it.primingCalls != 1 || it.samplesMs.size != 30 }) throw IOException("Incomplete index query report")
        val queries = JSONArray()
        for (result in results) {
            val value = JSONObject().put("query", result.query.query).put("priming_calls", result.primingCalls)
                .put("samples_ms", JSONArray(result.samplesMs)).put("series_complete", success && result.completed)
            if (success) value.put("p50_ms", BenchmarkSeries.nearestRank(result.samplesMs, .5))
                .put("p95_ms", BenchmarkSeries.nearestRank(result.samplesMs, .95))
            queries.put(value)
        }
        report.put("status", if (success) "complete" else "failed").put("phase", phase).put("queries", queries)
            .put("ordered_title_url_parity", if (success) "verified" else "not_completed").put("setup_ms", setup).put("parts", parts)
        if (error != null) report.put("error_class", error.javaClass.simpleName.take(128))
        val bytes = report.toString(2).toByteArray(Charsets.UTF_8)
        if (bytes.size > 65536) throw IOException("Index query report exceeds 64 KiB")
        val staged = Files.createTempFile(root, ".result-", ".json")
        try {
            FileOutputStream(staged.toFile()).use { stream -> stream.write(bytes); stream.fd.sync() }
            Files.move(staged, output, ATOMIC_MOVE)
        } finally { Files.deleteIfExists(staged) }
    }
}
