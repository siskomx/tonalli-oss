package org.tonalli.spike.saf

import android.content.Context
import android.util.AtomicFile
import java.io.File
import java.io.IOException
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject

/** Stores URI identities only; reopening always obtains new provider descriptors. */
class SafSelectionStore(context: Context) {
    private val file = AtomicFile(File(context.filesDir, "saf-selection.json"))
    private val summary = AtomicFile(File(context.filesDir, "benchmarks/saf-selection.json"))

    data class Record(val selection: SafSelection, val revision: String?, val fingerprint: String) {
        fun summary(): JSONObject = JSONObject().put("version", 1).put("revision", revision ?: JSONObject.NULL)
            .put("fingerprint", fingerprint).put("kind", selection.kind.name).put("part_count", selection.documents.size)
    }

    fun save(selection: SafSelection): Unit = synchronized(lock) {
        SafSelections.checked(selection)
        val record = Record(selection, UUID.randomUUID().toString(), SafBenchmarkContract.fingerprint(selection))
        val documents = JSONArray()
        selection.documents.forEach { documents.put(JSONObject().put("uri", it.uri).put("name", it.name)) }
        val json = JSONObject().put("version", 2).put("kind", selection.kind.name).put("documents", documents)
            .put("revision", record.revision).put("fingerprint", record.fingerprint)
            .put("tree_uri", selection.treeUri ?: JSONObject.NULL).put("inner_path", selection.innerPath ?: JSONObject.NULL)
        val bytes = json.toString().toByteArray(Charsets.UTF_8)
        if (bytes.size > MAX_BYTES) throw IOException("Selection metadata exceeds private storage bound")
        val output = file.startWrite()
        try { output.write(bytes); file.finishWrite(output) }
        catch (error: Throwable) { file.failWrite(output); throw error }
        // A missing/stale safe summary makes automation fail closed; the selection is already durable.
        runCatching { writeSummary(record) }.onFailure {
            android.util.Log.e("TonalliM1", "SAF benchmark identity unavailable: ${it.javaClass.simpleName}")
        }
        Unit
    }

    fun load(): SafSelection? = loadRecord()?.selection

    fun loadRecord(): Record? = synchronized(lock) {
        if (!file.baseFile.exists() && !File(file.baseFile.path + ".bak").exists()) return@synchronized null
        val bytes = file.openRead().use { input ->
            val bytes = input.readNBytes(MAX_BYTES + 1)
            if (bytes.size > MAX_BYTES) throw IOException("Oversized stored SAF selection")
            bytes
        }
        val json = JSONObject(bytes.toString(Charsets.UTF_8))
        val version = json.getInt("version")
        if (version !in 1..2) throw IOException("Unsupported stored SAF selection")
        val records = json.getJSONArray("documents")
        if (records.length() !in 1..128) throw IOException("Invalid stored SAF document count")
        val selection = SafSelections.checked(SafSelection(SafKind.valueOf(json.getString("kind")),
            (0 until records.length()).map { index -> records.getJSONObject(index).let { SafDocument(it.getString("uri"), it.getString("name")) } },
            if (json.isNull("tree_uri")) null else json.getString("tree_uri"),
            if (json.isNull("inner_path")) null else json.getString("inner_path")))
        val fingerprint = SafBenchmarkContract.fingerprint(selection)
        val revision = if (version == 2) json.getString("revision") else null
        if (version == 2 && (json.getString("fingerprint") != fingerprint ||
                revision == null || runCatching { UUID.fromString(revision).toString() != revision }.getOrDefault(true))) {
            throw IOException("Stored SAF selection identity differs")
        }
        Record(selection, revision, fingerprint)
    }

    fun <T> withCurrent(record: Record, work: () -> T): T = synchronized(lock) {
        val current = loadRecord() ?: throw IOException("No saved SAF selection")
        SafBenchmarkContract.requireSelection(current.selection, current.revision, current.fingerprint,
            record.revision ?: throw IOException("Reselect once to create a benchmark revision"), record.fingerprint)
        work()
    }

    private fun writeSummary(record: Record) {
        val parent = summary.baseFile.parentFile ?: throw IOException("Private benchmark directory unavailable")
        if (!parent.isDirectory && !parent.mkdirs()) throw IOException("Private benchmark directory unavailable")
        val output = summary.startWrite()
        try { output.write(record.summary().toString(2).toByteArray(Charsets.UTF_8)); summary.finishWrite(output) }
        catch (error: Throwable) { summary.failWrite(output); throw error }
    }

    companion object {
        private const val MAX_BYTES = 512 * 1024
        private val lock = Any()
    }
}
