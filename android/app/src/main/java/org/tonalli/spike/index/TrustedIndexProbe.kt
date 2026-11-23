package org.tonalli.spike.index

import android.content.Context
import android.os.SystemClock
import java.io.File
import java.io.IOException
import java.nio.file.Files
import java.util.Locale
import org.tonalli.spike.benchmark.BenchmarkDataset
import org.tonalli.spike.benchmark.BenchmarkSeries
import org.tonalli.spike.saf.SafBenchmarkBinding
import org.tonalli.spike.saf.SafBenchmarkErrors
import org.tonalli.spike.saf.SafSelectionStore
import uniffi.tonalli_core.indexOpen
import uniffi.tonalli_core.indexSearch

/** Validates a transferred completed index against a freshly resolved bound SAF pack. */
class TrustedIndexProbe(private val context: Context) {
    fun run(dataset: String, cancelled: () -> Boolean): String {
        val spec = PackagedIndexTransfer.load(context, dataset)
        val catalog = BenchmarkDataset.load(context, dataset)
        val binding = SafBenchmarkBinding.load(context, catalog)
        val store = SafSelectionStore(context)
        val external = context.getExternalFilesDir(null) ?: throw IOException("External index transfer storage unavailable")
        val root = File(context.filesDir, "index-handoff").toPath()
        val internal = root.resolve(dataset)
        if (Files.isSymbolicLink(root) || Files.isSymbolicLink(internal)) throw IOException("Linked private index storage")
        val report = IndexQueryReport(context, spec, catalog, binding)
        var phase = "copy_and_checksum"
        var failure: Throwable? = null
        val started = SystemClock.elapsedRealtimeNanos()
        try {
            TrustedIndexInstaller.install(external.toPath().resolve(spec.incomingName), internal, spec, cancelled,
                withSelection = { commit ->
                    phase = "activation"
                    store.withCurrent(binding.record) { commit() }
                }, validate = { staged ->
                    report.setup.put("copy_and_checksum", milliseconds(started))
                    phase = "source_open"
                    val sourceStarted = SystemClock.elapsedRealtimeNanos()
                    binding.open(context, catalog, cancelled).use { pack ->
                        report.setup.put("source_open", milliseconds(sourceStarted))
                        report.parts = binding.metadata(pack.partSizes)
                        phase = "index_open"
                        val indexStarted = SystemClock.elapsedRealtimeNanos()
                        val index = indexOpen(staged.toString(), pack.handle)
                        try {
                            report.setup.put("index_open", milliseconds(indexStarted))
                            phase = "warm_queries"
                            IndexQuerySeries.run(report.results, cancelled,
                                withSelection = { check -> store.withCurrent(binding.record) { check() } },
                                clock = SystemClock::elapsedRealtimeNanos,
                                search = { query -> indexSearch(index, query.query, 5u) },
                                verify = { query, hits -> IndexTransferSpec.requireHits(query,
                                    hits.map { IndexTransferSpec.Hit(it.title, it.url) }) })
                        } finally { index.destroy() }
                    }
                })
        } catch (error: Throwable) { failure = error }
        val error = failure
        if (error != null) {
            report.publish(false, phase, error)
            return SafBenchmarkErrors.message(error) + "\nFailed report: ${report.relativePath}; partial samples are not a completed series."
        }
        val summary = report.results.joinToString("\n") { result ->
            String.format(Locale.ROOT, "%s: 30 warm samples, p50 %.3f ms, p95 %.3f ms", result.query.query,
                BenchmarkSeries.nearestRank(result.samplesMs, .5), BenchmarkSeries.nearestRank(result.samplesMs, .95))
        }
        report.publish(true, "complete")
        return "Trusted $dataset index ready after ordered title/URL parity.\n$summary\nReport: ${report.relativePath}\n" +
            "OS cache untouched; setup excluded. Source evidence external_manual_not_rechecked. Phone acceptance is not assessed."
    }

    private fun milliseconds(start: Long): Double = (SystemClock.elapsedRealtimeNanos() - start) / 1_000_000.0
}
