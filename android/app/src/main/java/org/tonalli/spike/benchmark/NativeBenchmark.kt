package org.tonalli.spike.benchmark

import android.app.ActivityManager
import android.content.Context
import android.os.Build
import android.os.Process
import android.os.SystemClock
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import org.json.JSONArray
import org.json.JSONObject
import uniffi.tonalli_core.packClose
import uniffi.tonalli_core.packOpenInternal
import uniffi.tonalli_core.ramTier
import uniffi.tonalli_core.zimArticle
import uniffi.tonalli_core.zimAttachTitleIndex
import uniffi.tonalli_core.zimSuggest
import org.tonalli.spike.saf.SafBenchmarkBinding
import org.tonalli.spike.saf.SafManualReceipt

object NativeBenchmark {
    fun run(context: Context, dataset: BenchmarkDataset, request: BenchmarkRequest, cancelled: () -> Boolean,
        saf: SafBenchmarkBinding? = null): File {
        if (request.manifestHash != dataset.manifestHash || request.dataset != dataset.id) throw IOException("Benchmark manifest mismatch")
        val resultRoot = File(context.filesDir, "benchmarks").toPath()
        Files.createDirectories(resultRoot)
        if (!Files.isDirectory(resultRoot, NOFOLLOW_LINKS)) throw IOException("Invalid private benchmark result directory")
        val output = BenchmarkFiles.claimResult(resultRoot, request.runId)
        val samples = BenchmarkSeries.Samples()
        val setup = JSONObject().put("native_init", 0.0).put("open", 0.0).put("attach", 0.0).put("total", 0.0)
        val ram = ActivityManager.MemoryInfo()
        context.getSystemService(ActivityManager::class.java).getMemoryInfo(ram)
        val report = JSONObject().put("version", 1).put("run_id", request.runId).put("dataset", dataset.id)
            .put("manifest_sha256", dataset.manifestHash).put("mode", request.mode).put("source", if (saf == null) "internal_mmap" else "saf_pread")
            .put("pid", Process.myPid()).put("process_started_elapsed_ms", Process.getStartElapsedRealtime())
            .put("api_level", Build.VERSION.SDK_INT).put("cache_state", "uncontrolled_os_cache")
            .put("supported_abis", JSONArray(Build.SUPPORTED_ABIS.toList()))
            .put("ram", JSONObject().put("advertised_bytes", ram.advertisedMem.coerceAtLeast(0))
                .put("total_bytes", ram.totalMem.coerceAtLeast(0)).put("available_bytes", ram.availMem.coerceAtLeast(0)))
            .put("title_index_sha256", dataset.entries.first { it.name == "titles.bin" }.sha256)
            .put("prefix", dataset.prefix).put("article_url", dataset.articleUrl).put("acceptance_eligible", dataset.eligible)
            .put("percentile_method", "nearest_rank").put("setup_ms", setup)
            .put("cache_note", "Title attachment scans its checksum during setup; fresh process is not OS-cold.")
        val expectedPack = dataset.entries.first { it.name == "pack.zim" }
        if (saf == null) report.put("zim_sha256", expectedPack.sha256)
        else report.put("expected_zim_sha256", expectedPack.sha256).put("expected_zim_bytes", expectedPack.bytes)
            .put("selection", saf.selection()).put("parts", saf.metadata())
            .put("external_manual", SafManualReceipt.projection(saf.manual, saf.receiptHash))
        val setupStart = SystemClock.elapsedRealtimeNanos()
        var phase = "prepared_receipt"
        try {
            if (cancelled()) throw IOException("Benchmark cancelled")
            val directory = if (saf == null) dataset.prepared(context) else dataset.preparedTable(context)
            phase = "native_init"
            var started = SystemClock.elapsedRealtimeNanos()
            val tier = ramTier(ram.totalMem.coerceAtLeast(0).toULong())
            setup.put("native_init", milliseconds(started))
            report.put("ram_tier", tier.toString())
            phase = "open"
            started = SystemClock.elapsedRealtimeNanos()
            val source = saf?.open(context, dataset, cancelled)
            val handle = source?.handle ?: packOpenInternal(directory.resolve("pack.zim").toString())
            setup.put("open", milliseconds(started))
            if (source != null) report.put("parts", saf.metadata(source.partSizes))
            try {
                phase = "attach"
                started = SystemClock.elapsedRealtimeNanos()
                zimAttachTitleIndex(handle, directory.resolve("titles.bin").toString())
                setup.put("attach", milliseconds(started))
                setup.put("total", milliseconds(setupStart))
                phase = "samples"
                BenchmarkSeries.run(request.mode, samples, cancelled, SystemClock::elapsedRealtimeNanos,
                    { zimSuggest(handle, dataset.prefix, 10u).size },
                    { zimArticle(handle, dataset.articleUrl).bytes.size })
            } finally {
                if (source != null) source.close()
                else try { packClose(handle) } finally { handle.destroy() }
            }
            if (cancelled()) throw IOException("Benchmark cancelled")
            report.put("status", "complete")
        } catch (error: Throwable) {
            if (setup.getDouble("total") == 0.0) setup.put("total", milliseconds(setupStart))
            report.put("status", "failed").put("error", if (cancelled()) "Cancelled" else error.javaClass.simpleName)
                .put("error_stage", phase)
        }
        report.put("priming", JSONObject().put("suggest", samples.primedSuggest).put("article", samples.primedArticle))
            .put("samples", JSONObject().put("suggest_ms", JSONArray(samples.suggest)).put("article_ms", JSONArray(samples.article)))
        if (report.getString("status") == "complete") {
            val percentiles = JSONObject()
            for ((name, values) in listOf("suggest_ms" to samples.suggest, "article_ms" to samples.article)) {
                if (values.isNotEmpty()) percentiles.put(name, JSONObject().put("p50", BenchmarkSeries.nearestRank(values, .5))
                    .put("p95", BenchmarkSeries.nearestRank(values, .95)))
            }
            report.put("percentiles", percentiles)
        }
        val staged = Files.createTempFile(resultRoot, ".result-", ".json")
        try {
            FileOutputStream(staged.toFile()).use { file -> file.write(report.toString(2).toByteArray(Charsets.UTF_8)); file.fd.sync() }
            if (Files.exists(output, NOFOLLOW_LINKS)) throw IOException("Benchmark result collision")
            Files.move(staged, output, ATOMIC_MOVE)
        } finally { Files.deleteIfExists(staged) }
        return output.toFile()
    }

    private fun milliseconds(started: Long) = (SystemClock.elapsedRealtimeNanos() - started) / 1_000_000.0
}
