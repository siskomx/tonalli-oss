package org.tonalli.spike.benchmark

import android.content.Context
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.security.MessageDigest
import org.json.JSONObject
import uniffi.tonalli_core.PackHandle
import uniffi.tonalli_core.packClose
import uniffi.tonalli_core.packOpenInternal
import uniffi.tonalli_core.zimAttachTitleIndex
import org.tonalli.spike.saf.SafSelectionStore
import org.tonalli.spike.saf.SafBenchmarkContract
import org.tonalli.spike.saf.SafDocuments
import org.tonalli.spike.saf.SafPackSession

class BenchmarkDataset private constructor(val id: String, val name: String, val manifest: ByteArray,
    val manifestHash: String, val eligible: Boolean, val prefix: String, val articleUrl: String,
    val entries: List<BenchmarkFiles.Entry>) {
    fun dataRoot(context: Context): Path {
        val root = File(context.filesDir, "benchmarks").toPath()
        if (Files.isSymbolicLink(root) || Files.isSymbolicLink(root.resolve("data"))) throw IOException("Linked benchmark storage is not internal immutable input")
        return root.resolve("data")
    }
    fun prepared(context: Context): Path = BenchmarkFiles.prepared(dataRoot(context), manifestHash, entries)

    private fun tableRoot(context: Context): Path {
        val root = File(context.filesDir, "benchmarks").toPath()
        if (Files.isSymbolicLink(root) || Files.isSymbolicLink(root.resolve("tables"))) throw IOException("Linked benchmark table storage")
        return root.resolve("tables")
    }
    fun preparedTable(context: Context): Path = BenchmarkFiles.preparedTable(tableRoot(context), manifestHash,
        entries.first { it.name == "titles.bin" })

    fun prepareTable(context: Context, cancelled: () -> Boolean): Path {
        val store = SafSelectionStore(context)
        val record = store.loadRecord() ?: throw IOException("Select a SAF ZIM first")
        SafBenchmarkContract.requireSelection(record.selection, record.revision, record.fingerprint,
            record.revision ?: throw IOException("Reselect once to create a benchmark revision"), record.fingerprint)
        val external = context.getExternalFilesDir(null) ?: throw IOException("External benchmark table storage unavailable")
        val incoming = File(external, "benchmark/$id").toPath()
        val supplied = Files.newInputStream(incoming.resolve("manifest.json"), NOFOLLOW_LINKS).use(::readManifest)
        if (!supplied.contentEquals(manifest)) throw IOException("External benchmark manifest differs from packaged input")
        return BenchmarkFiles.prepareTable(incoming, tableRoot(context), manifestHash,
            entries.first { it.name == "titles.bin" }, cancelled, validate = { staged ->
                store.withCurrent(record) { }
                SafPackSession.open(SafDocuments(context.contentResolver), record.selection, cancelled).use { pack ->
                    zimAttachTitleIndex(pack.handle, staged.resolve("titles.bin").toString())
                }
                store.withCurrent(record) { }
            })
    }

    fun prepare(context: Context, cancelled: () -> Boolean): Path {
        val external = context.getExternalFilesDir(null) ?: throw IOException("External benchmark storage unavailable")
        val incoming = File(external, "benchmark/$id").toPath()
        val supplied = Files.newInputStream(incoming.resolve("manifest.json"), NOFOLLOW_LINKS).use(::readManifest)
        if (!supplied.contentEquals(manifest)) throw IOException("External benchmark manifest differs from packaged input")
        return BenchmarkFiles.prepare(incoming, dataRoot(context), manifestHash, entries, cancelled, validate = { staged ->
            withPack(staged) { }
        })
    }

    fun <T> withPack(directory: Path, work: (PackHandle) -> T): T {
        val handle = packOpenInternal(directory.resolve("pack.zim").toString())
        return try {
            zimAttachTitleIndex(handle, directory.resolve("titles.bin").toString())
            work(handle)
        } finally { try { packClose(handle) } finally { handle.destroy() } }
    }

    companion object {
        fun ids(context: Context): List<String> = context.assets.list("").orEmpty().mapNotNull {
            Regex("benchmark-([a-z][a-z0-9_-]{0,31})\\.json").matchEntire(it)?.groupValues?.get(1)
        }.sorted()

        fun load(context: Context, id: String): BenchmarkDataset {
            if (id !in ids(context)) throw IOException("Unknown packaged benchmark dataset")
            val bytes = context.assets.open("benchmark-$id.json").use(::readManifest)
            val json = JSONObject(bytes.toString(Charsets.UTF_8))
            if (json.getInt("version") != 1 || json.getString("id") != id) throw IOException("Invalid benchmark identity")
            val prefix = json.getString("prefix")
            val url = json.getString("article_url")
            if (prefix.isBlank() || prefix.toByteArray().size > 4096 || url.isBlank() || url.toByteArray().size > 4096 ||
                url.contains("://") || url.startsWith('/') || url.any { it < ' ' }) throw IOException("Invalid known benchmark query")
            val entries = listOf("zim" to "pack.zim", "title_index" to "titles.bin").map { (key, name) ->
                val entry = json.getJSONObject(key)
                BenchmarkFiles.Entry(name, entry.getLong("bytes"), entry.getString("sha256"))
            }
            val hash = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
            return BenchmarkDataset(id, json.getString("name"), bytes, hash, json.getBoolean("acceptance_eligible"), prefix, url, entries)
        }

        private fun readManifest(input: InputStream): ByteArray {
            val output = ByteArrayOutputStream()
            val buffer = ByteArray(4096)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                if (count == 0 || output.size() > 65536 - count) throw IOException("Invalid benchmark manifest size")
                output.write(buffer, 0, count)
            }
            return output.toByteArray()
        }
    }
}
