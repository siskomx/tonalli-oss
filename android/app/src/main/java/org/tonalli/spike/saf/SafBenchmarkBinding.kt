package org.tonalli.spike.saf

import android.content.Context
import android.util.AtomicFile
import android.util.Base64
import java.io.File
import java.io.IOException
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.security.MessageDigest
import org.json.JSONArray
import org.json.JSONObject
import org.tonalli.spike.benchmark.BenchmarkDataset

class SafBenchmarkBinding private constructor(val record: SafSelectionStore.Record, val receiptHash: String,
    val manual: JSONObject, val parts: List<Long?>) {
    fun open(context: Context, dataset: BenchmarkDataset, cancelled: () -> Boolean): SafPackSession {
        val store = SafSelectionStore(context)
        store.withCurrent(record) { }
        return SafPackSession.open(SafDocuments(context.contentResolver), record.selection, cancelled) { sizes ->
            SafBenchmarkContract.requireLengths(parts, sizes, dataset.entries.first { it.name == "pack.zim" }.bytes)
            store.withCurrent(record) { }
        }
    }

    fun selection(): JSONObject = record.summary().also { it.remove("version") }
    fun metadata(sizes: List<Long?> = parts): JSONArray = JSONArray().also { array -> sizes.forEachIndexed { index, bytes ->
        array.put(JSONObject().put("ordinal", index).put("bytes", bytes ?: JSONObject.NULL))
    } }

    companion object {
        private val bindingLock = Any()
        fun bind(context: Context, dataset: BenchmarkDataset, request: SafBindingRequest?, cancelled: () -> Boolean): SafBenchmarkBinding {
            val store = SafSelectionStore(context)
            val record = store.loadRecord() ?: throw IOException("Select a SAF ZIM first")
            requireRequest(dataset, record, request)
            val external = context.getExternalFilesDir(null) ?: throw IOException("External manual receipt storage unavailable")
            val bytes = Files.newInputStream(File(external, "benchmark/saf-receipt.json").toPath(), NOFOLLOW_LINKS).use { bounded(it, 16384) }
            val hash = sha(bytes)
            if (request != null && hash != request.receiptHash) throw IOException("Manual receipt SHA differs from explicit binding request")
            val manual = SafManualReceipt.checked(bytes, dataset, record)
            if (cancelled()) throw IOException("Manual binding cancelled")
            val opened = SafPackSession.open(SafDocuments(context.contentResolver), record.selection, cancelled)
            val sizes = try { opened.partSizes } finally { opened.close() }
            SafBenchmarkContract.requireLengths(sizes, sizes, dataset.entries.first { it.name == "pack.zim" }.bytes)
            val binding = SafBenchmarkBinding(record, hash, manual, sizes)
            synchronized(bindingLock) {
                store.withCurrent(record) {
                    if (cancelled()) throw IOException("Manual binding cancelled")
                    val file = file(context)
                    val json = JSONObject().put("version", 1).put("receipt_sha256", hash)
                        .put("manual_receipt_base64", Base64.encodeToString(bytes, Base64.NO_WRAP)).put("parts", binding.metadata())
                    val output = file.startWrite()
                    try {
                        output.write(json.toString().toByteArray(Charsets.UTF_8))
                        if (cancelled()) throw IOException("Manual binding cancelled")
                        file.finishWrite(output)
                    } catch (error: Throwable) { file.failWrite(output); throw error }
                }
            }
            return binding
        }

        fun load(context: Context, dataset: BenchmarkDataset, request: SafBindingRequest? = null): SafBenchmarkBinding = synchronized(bindingLock) {
            val record = SafSelectionStore(context).loadRecord() ?: throw IOException("Select a SAF ZIM first")
            requireRequest(dataset, record, request)
            val json = JSONObject(file(context).openRead().use { bounded(it, 32768) }.toString(Charsets.UTF_8))
            if (json.getInt("version") != 1) throw IOException("Unsupported private SAF binding")
            val bytes = Base64.decode(json.getString("manual_receipt_base64"), Base64.NO_WRAP)
            val hash = sha(bytes)
            if (hash != json.getString("receipt_sha256") || (request != null && hash != request.receiptHash)) throw IOException("Private manual receipt identity changed")
            val manual = SafManualReceipt.checked(bytes, dataset, record)
            val array = json.getJSONArray("parts")
            if (array.length() != record.selection.documents.size) throw IOException("Private SAF part count differs")
            val parts = (0 until array.length()).map { index -> array.getJSONObject(index).let {
                if (it.getInt("ordinal") != index) throw IOException("Invalid private SAF part ordinal")
                if (it.isNull("bytes")) null else it.getLong("bytes")
            } }
            SafBenchmarkContract.requireLengths(parts, parts, dataset.entries.first { it.name == "pack.zim" }.bytes)
            SafBenchmarkBinding(record, hash, manual, parts)
        }

        private fun requireRequest(dataset: BenchmarkDataset, record: SafSelectionStore.Record, request: SafBindingRequest?) {
            if (request != null && (request.dataset != dataset.id || request.manifestHash != dataset.manifestHash)) throw IOException("SAF dataset manifest differs")
            SafBenchmarkContract.requireSelection(record.selection, record.revision, record.fingerprint,
                request?.revision ?: record.revision ?: throw IOException("Reselect once to create a benchmark revision"), request?.fingerprint ?: record.fingerprint)
        }
        private fun file(context: Context): AtomicFile {
            val root = File(context.filesDir, "benchmarks")
            if ((!root.isDirectory && !root.mkdirs()) || Files.isSymbolicLink(root.toPath())) throw IOException("Private benchmark binding directory unavailable")
            return AtomicFile(File(root, "saf-binding.json"))
        }
        private fun bounded(input: java.io.InputStream, limit: Int): ByteArray = input.readNBytes(limit + 1).also {
            if (it.size > limit) throw IOException("Oversized manual SAF receipt/binding")
        }
        private fun sha(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
    }
}
