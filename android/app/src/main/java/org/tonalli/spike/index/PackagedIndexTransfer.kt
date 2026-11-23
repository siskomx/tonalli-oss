package org.tonalli.spike.index

import android.content.Context
import java.io.IOException
import org.json.JSONObject
import org.tonalli.spike.benchmark.BenchmarkDataset

class IndexTransferUnavailable(val dataset: String) : IOException() {
    val userMessage: String get() = "Trusted $dataset index descriptor is unavailable. Its completed export has not been packaged."
}

/** Resolves only named packaged inputs; incoming metadata never defines trusted limits. */
object PackagedIndexTransfer {
    fun load(context: Context, dataset: String): IndexTransferSpec {
        if (dataset !in setOf("small", "full", "medicine")) throw IOException("Unknown index handoff dataset")
        val name = "index-probe-$dataset.json"
        if (name !in context.assets.list("").orEmpty()) throw IndexTransferUnavailable(dataset)
        val raw = context.assets.open(name).use { it.readNBytes(IndexTransferSpec.MAX_DESCRIPTOR_BYTES + 1) }
        if (raw.size > IndexTransferSpec.MAX_DESCRIPTOR_BYTES) throw IOException("Oversized trusted index descriptor")
        val json = JSONObject(raw.toString(Charsets.UTF_8))
        keys(json, setOf("version", "dataset_id", "generation", "source", "files", "queries"))
        if (json.get("version") != 2 || string(json, "dataset_id") != dataset) throw IOException("Invalid trusted index descriptor version or dataset")
        val source = json.getJSONObject("source")
        keys(source, setOf("uuid", "bytes", "md5", "expected_sha256"))
        val files = json.getJSONArray("files")
        val queries = json.getJSONArray("queries")
        if (files.length() !in 3..IndexTransferSpec.MAX_FILES || queries.length() != IndexTransferSpec.QUERY_NAMES.size) throw IOException("Invalid trusted index descriptor counts")
        val entries = (0 until files.length()).map { index -> files.getJSONObject(index).let {
            keys(it, setOf("path", "bytes", "sha256"))
            IndexTransferSpec.Entry(string(it, "path"), long(it, "bytes"), string(it, "sha256"))
        } }
        val baselines = (0 until queries.length()).map { index -> queries.getJSONObject(index).let { query ->
            keys(query, setOf("query", "hits"))
            val hits = query.getJSONArray("hits")
            if (hits.length() > 5) throw IOException("Oversized trusted index hit baseline")
            IndexTransferSpec.Query(string(query, "query"), (0 until hits.length()).map { ordinal -> hits.getJSONObject(ordinal).let {
                keys(it, setOf("title", "url")); IndexTransferSpec.Hit(string(it, "title"), string(it, "url"))
            } })
        } }
        val spec = IndexTransferSpec.checked(dataset, raw, string(json, "generation"),
            IndexTransferSpec.Source(string(source, "uuid"), long(source, "bytes"), string(source, "md5"), string(source, "expected_sha256")), entries, baselines)
        val pack = BenchmarkDataset.load(context, dataset).entries.first { it.name == "pack.zim" }
        if (spec.source.bytes != pack.bytes || spec.source.expectedSha256 != pack.sha256) throw IOException("Trusted index source differs from packaged dataset catalog")
        return spec
    }
    private fun keys(json: JSONObject, expected: Set<String>) {
        if (json.keys().asSequence().toSet() != expected) throw IOException("Trusted index descriptor fields differ")
    }
    private fun string(json: JSONObject, name: String): String = json.get(name) as? String ?: throw IOException("Invalid trusted index text field")
    private fun long(json: JSONObject, name: String): Long {
        val value = json.get(name)
        if (value !is Int && value !is Long) throw IOException("Invalid trusted index integer field")
        return (value as Number).toLong()
    }
}
