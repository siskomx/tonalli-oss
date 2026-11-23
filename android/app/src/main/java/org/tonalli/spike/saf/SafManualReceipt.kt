package org.tonalli.spike.saf

import java.io.IOException
import java.time.OffsetDateTime
import org.json.JSONObject
import org.tonalli.spike.benchmark.BenchmarkDataset

/** Checks an operator statement; deliberately does not read/hash current provider content. */
object SafManualReceipt {
    fun checked(bytes: ByteArray, dataset: BenchmarkDataset, record: SafSelectionStore.Record): JSONObject {
        if (bytes.size > 16384) throw IOException("Manual receipt exceeds 16 KiB")
        val json = JSONObject(bytes.toString(Charsets.UTF_8))
        val expected = setOf("version", "dataset_id", "manifest_sha256", "selection_revision", "selection_fingerprint",
            "observed_logical_zim_sha256", "observed_logical_zim_bytes", "observed_at_utc", "medium", "card_class", "filesystem", "evidence")
        if (json.keys().asSequence().toSet() != expected || json.get("version") != 1 ||
            expected.filter { it !in setOf("version", "observed_logical_zim_bytes") }.any { json.get(it) !is String } ||
            (json.get("observed_logical_zim_bytes") !is Int && json.get("observed_logical_zim_bytes") !is Long) ||
            json.getString("dataset_id") != dataset.id || json.getString("manifest_sha256") != dataset.manifestHash ||
            json.getString("observed_logical_zim_sha256") != dataset.entries.first { it.name == "pack.zim" }.sha256 ||
            json.getLong("observed_logical_zim_bytes") != dataset.entries.first { it.name == "pack.zim" }.bytes ||
            json.getString("evidence") != "external_manual_not_rechecked") throw IOException("Manual statement differs from the trusted dataset or evidence boundary")
        SafBenchmarkContract.requireSelection(record.selection, record.revision, record.fingerprint,
            json.getString("selection_revision"), json.getString("selection_fingerprint"))
        for (key in listOf("observed_at_utc", "medium", "card_class", "filesystem")) {
            val text = json.getString(key)
            if (text.isEmpty() || text.toByteArray(Charsets.UTF_8).size > 128 || text.any(Char::isISOControl)) throw IOException("Invalid manual observation text")
        }
        val observed = runCatching { OffsetDateTime.parse(json.getString("observed_at_utc")) }.getOrElse { throw IOException("Manual receipt requires explicit UTC observation time") }
        if (observed.offset.totalSeconds != 0) throw IOException("Manual receipt observation time must be UTC")
        return json
    }

    fun projection(manual: JSONObject, receiptHash: String): JSONObject {
        val value = JSONObject().put("receipt_sha256", receiptHash).put("verification", "external_manual_not_rechecked")
        for (key in listOf("observed_logical_zim_sha256", "observed_logical_zim_bytes", "observed_at_utc", "medium", "card_class", "filesystem")) value.put(key, manual.get(key))
        return value
    }
}
