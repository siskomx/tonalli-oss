package org.tonalli.spike.index

import android.content.Context
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.security.MessageDigest
import org.json.JSONObject
import org.tonalli.spike.withPack
import uniffi.tonalli_core.PackException
import uniffi.tonalli_core.indexOpen
import uniffi.tonalli_core.indexSearch

/** Exact small Windows fixture checks, not a general-purpose index installer. */
class IndexProbe(private val context: Context) {
    fun run(): String {
        val external = context.getExternalFilesDir(null) ?: throw IOException("External app storage unavailable")
        val expected = context.assets.open("index-probe-manifest.json").use(::readManifest)
        val hash = MessageDigest.getInstance("SHA-256").digest(expected).joinToString("") { "%02x".format(it) }
        val incoming = File(external, IndexSnapshot.incomingName(hash)).toPath()
        val supplied = Files.newInputStream(incoming.resolve("snapshot.json"), NOFOLLOW_LINKS).use(::readManifest)
        if (!supplied.contentEquals(expected)) throw IOException("snapshot.json differs from the packaged Windows fixture manifest")
        val manifest = JSONObject(expected.toString(Charsets.UTF_8))
        if (manifest.getInt("version") != 1) throw IOException("Unsupported index snapshot version")
        val records = manifest.getJSONArray("files")
        if (records.length() !in 2..128) throw IOException("Invalid snapshot file count")
        val entries = (0 until records.length()).map { position ->
            val entry = records.getJSONObject(position)
            IndexSnapshot.Entry(entry.getString("path"), entry.getLong("bytes"), entry.getString("sha256"))
        }
        return withPack(File(external, "probe.zim")) { correct ->
            withPack(File(external, "wrong-probe.zim")) { wrong ->
                val results = StringBuilder()
                val installed = IndexSnapshot.install(incoming, File(context.filesDir, "index-probe").toPath(),
                    hash, manifest.getString("generation"), entries) { staged ->
                    val index = indexOpen(staged.toString(), correct)
                    try {
                        for ((query, expectedTitles) in windowsQueries) {
                            val hits = indexSearch(index, query, 5u)
                            val titles = hits.map { it.title }
                            if (titles != expectedTitles) throw IOException("Windows parity failed for $query: $titles")
                            results.append("MATCH $query\n")
                            hits.forEach { results.append("  ${it.title} | ${it.url}\n") }
                        }
                    } finally { index.destroy() }
                    try {
                        val unexpected = indexOpen(staged.toString(), wrong)
                        unexpected.destroy()
                        throw IOException("Wrong ZIM was unexpectedly accepted")
                    } catch (_: PackException.Mismatch) {
                        results.append("MATCH wrong Spanish ZIM: PackException.Mismatch\n")
                    }
                }
                "SUCCESS — small index portability probe\nInternal snapshot: $installed\n$results"
            }
        }
    }

    private fun readManifest(input: InputStream): ByteArray {
        val output = ByteArrayOutputStream()
        val buffer = ByteArray(4096)
        while (true) {
            val count = input.read(buffer)
            if (count < 0) break
            if (count == 0 || output.size() > 65536 - count) throw IOException("Invalid or oversized snapshot manifest")
            output.write(buffer, 0, count)
        }
        return output.toByteArray()
    }

    private val windowsQueries = listOf(
        "\"nontrivial knot\"" to listOf("2-bridge knot", "Gordon\u2013Luecke theorem", "Trefoil knot", "Satellite knot", "Ropelength"),
        "trefoil" to listOf("Trefoil knot", "Square knot (mathematics)", "Granny knot (mathematics)", "Stick number", "Crossing number (knot theory)"),
        "\"overhand knot\"" to listOf("Overhand knot", "Double overhand knot", "Overhand knot with draw-loop", "Arbor knot", "Slip knot"),
    )
}
