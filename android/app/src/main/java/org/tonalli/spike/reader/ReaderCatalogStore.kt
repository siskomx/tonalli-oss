package org.tonalli.spike.reader

import android.content.Context
import android.util.AtomicFile
import java.io.File
import java.io.IOException
import java.util.UUID
import org.json.JSONArray
import org.json.JSONObject
import org.tonalli.spike.saf.*

/** Product references only. Never writes the diagnostic SAF selection or benchmark binding. */
class ReaderCatalogStore(context: Context) {
    private val file = AtomicFile(File(context.filesDir, "reader/catalog.json"))

    fun load(): List<ReaderEntry> = synchronized(lock) {
        if (!file.baseFile.exists() && !File(file.baseFile.path + ".bak").exists()) return@synchronized emptyList()
        val bytes = file.openRead().use { it.readNBytes(MAX_BYTES + 1) }
        if (bytes.size > MAX_BYTES) throw IOException("Reader catalog exceeds storage bound")
        val root = JSONObject(bytes.toString(Charsets.UTF_8))
        if (root.getInt("version") != 1) throw IOException("Unsupported reader catalog")
        val rows = root.getJSONArray("sources")
        if (rows.length() > 64) throw IOException("Reader catalog is full")
        ReaderCatalogPolicy.checked((0 until rows.length()).map { index ->
            val row = rows.getJSONObject(index)
            val docs = row.getJSONArray("documents")
            if (docs.length() !in 1..128) throw IOException("Invalid reader document count")
            ReaderEntry(row.getString("id"), SafSelection(SafKind.valueOf(row.getString("kind")),
                (0 until docs.length()).map { i -> docs.getJSONObject(i).let { SafDocument(it.getString("uri"), it.getString("name")) } },
                if (row.isNull("tree_uri")) null else row.getString("tree_uri"),
                if (row.isNull("inner_path")) null else row.getString("inner_path")),
                ReaderIdentity(row.getString("uuid"), row.getString("stored_md5")))
        })
    }

    /** A short atomic publication may run only while the exact native source reference is current. */
    fun withCurrent(expected: ReaderEntry, publish: () -> Unit): Unit = synchronized(lock) {
        if (load().none { it == expected }) throw IOException("Reader selection changed")
        publish()
    }

    /** Called only after Core validation and on the active Activity's main looper. */
    fun add(selection: SafSelection, verified: ReaderIdentity): Unit = synchronized(lock) {
        val entries = ReaderCatalogPolicy.add(load(), selection, verified, "reader-${UUID.randomUUID()}")
        val rows = JSONArray()
        entries.forEach { entry ->
            val docs = JSONArray()
            entry.selection.documents.forEach { docs.put(JSONObject().put("uri", it.uri).put("name", it.name)) }
            rows.put(JSONObject().put("id", entry.id).put("uuid", entry.identity.uuid).put("stored_md5", entry.identity.storedMd5)
                .put("kind", entry.selection.kind.name).put("documents", docs)
                .put("tree_uri", entry.selection.treeUri ?: JSONObject.NULL).put("inner_path", entry.selection.innerPath ?: JSONObject.NULL))
        }
        val bytes = JSONObject().put("version", 1).put("sources", rows).toString().toByteArray(Charsets.UTF_8)
        if (bytes.size > MAX_BYTES) throw IOException("Reader catalog exceeds storage bound")
        val parent = file.baseFile.parentFile ?: throw IOException("Reader storage unavailable")
        if (!parent.isDirectory && !parent.mkdirs()) throw IOException("Reader storage unavailable")
        val stream = file.startWrite()
        try { stream.write(bytes); file.finishWrite(stream) }
        catch (error: Throwable) { file.failWrite(stream); throw error }
    }

    companion object { private const val MAX_BYTES = 4 * 1024 * 1024; private val lock = Any() }
}
