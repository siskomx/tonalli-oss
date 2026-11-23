package org.tonalli.spike.probe

import android.app.Activity
import android.content.Intent
import android.content.Context
import android.content.pm.ApplicationInfo
import android.os.Process
import android.system.Os
import android.widget.TextView
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.Executors
import org.json.JSONArray
import org.json.JSONObject
import org.tonalli.spike.benchmark.BenchmarkDataset
import org.tonalli.spike.benchmark.BenchmarkFiles
import org.tonalli.spike.index.IndexSnapshot
import org.tonalli.spike.index.IndexTransferSpec
import org.tonalli.spike.index.PackagedIndexTransfer

/** Debug transfer setup executes as the app UID, unlike adb-created external directories. */
object ProbeInputRouting {
    private const val ACTION = "org.tonalli.spike.PREPARE_PROBE_INPUTS"
    fun route(activity: Activity, intent: Intent): Boolean {
        if (intent.action != ACTION) return false
        if (activity.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE == 0) { activity.finish(); return true }
        val request = intent.getStringExtra("request_id")
        val expectedHash = intent.getStringExtra("manifest_sha256")
        val dataset = intent.getStringExtra("dataset_id")
        if (request == null || runCatching { UUID.fromString(request).toString() != request }.getOrDefault(true) ||
            expectedHash == null || !Regex("[a-f0-9]{64}").matches(expectedHash) ||
            (dataset != null && dataset !in setOf("small", "full", "medicine"))) {
            activity.finish(); return true
        }
        activity.setContentView(TextView(activity).apply { text = "Preparing trusted app-owned probe transfer directories…" })
        val context = activity.applicationContext
        val worker = Executors.newSingleThreadExecutor()
        worker.execute {
            val output = runCatching {
                val root = File(context.filesDir, "probe-inputs-ready").toPath()
                Files.createDirectories(root)
                if (!Files.isDirectory(root, NOFOLLOW_LINKS)) throw IOException("Invalid private receipt directory")
                BenchmarkFiles.claimResult(root, request)
            }.getOrElse {
                android.util.Log.e("TonalliM1", "Probe transfer request rejected: ${it.javaClass.simpleName}")
                worker.shutdown()
                activity.runOnUiThread { if (!activity.isDestroyed) activity.finish() }
                return@execute
            }
            val receipt = JSONObject().put("version", 1).put("request_id", request).put("manifest_sha256", expectedHash)
            if (dataset != null) receipt.put("dataset_id", dataset)
            try {
                if (dataset != null) prepareV2(context, dataset, expectedHash, receipt)
                else {
                    val manifest = context.assets.open("index-probe-manifest.json").use { input ->
                        input.readNBytes(65537).also { if (it.size > 65536) throw IOException("Oversized trusted index manifest") }
                    }
                    val hash = MessageDigest.getInstance("SHA-256").digest(manifest).joinToString("") { "%02x".format(it) }
                    if (hash != expectedHash) throw IOException("Trusted index manifest mismatch")
                    val json = JSONObject(manifest.toString(Charsets.UTF_8))
                    if (json.getInt("version") != 1) throw IOException("Unsupported trusted index manifest")
                    val records = json.getJSONArray("files")
                    if (records.length() !in 2..128) throw IOException("Invalid trusted index file count")
                    val entries = (0 until records.length()).map { index -> records.getJSONObject(index).let {
                        IndexSnapshot.Entry(it.getString("path"), it.getLong("bytes"), it.getString("sha256"))
                    } }
                    val generation = json.getString("generation")
                    val datasets = BenchmarkDataset.ids(context)
                    val paths = ProbeInputDirectories.paths(hash, generation, entries, datasets)
                    val external = context.getExternalFilesDir(null) ?: throw IOException("App-specific external storage unavailable")
                    ProbeInputDirectories.prepare(external.toPath(), paths) { path -> Os.stat(path.toString()).st_uid == Process.myUid() }
                    receipt.put("status", "ready").put("incoming_name", IndexSnapshot.incomingName(hash))
                        .put("generation", generation).put("benchmark_ids", JSONArray(datasets))
                }
            } catch (error: Throwable) {
                receipt.put("status", "failed").put("error", error.javaClass.simpleName)
            }
            try {
                val staged = Files.createTempFile(output.parent, ".ready-", ".json")
                try {
                    FileOutputStream(staged.toFile()).use { stream -> stream.write(receipt.toString(2).toByteArray(Charsets.UTF_8)); stream.fd.sync() }
                    Files.move(staged, output, ATOMIC_MOVE)
                } finally { Files.deleteIfExists(staged) }
            } catch (error: Throwable) {
                android.util.Log.e("TonalliM1", "Probe transfer preparation failed: ${error.javaClass.simpleName}")
            } finally {
                worker.shutdown()
                activity.runOnUiThread { if (!activity.isDestroyed) activity.finish() }
            }
        }
        return true
    }

    private fun prepareV2(context: Context, dataset: String, expectedHash: String, receipt: JSONObject) {
        val spec = PackagedIndexTransfer.load(context, dataset)
        if (spec.manifestHash != expectedHash) throw IOException("Trusted index descriptor mismatch")
        val external = context.getExternalFilesDir(null) ?: throw IOException("App-specific external storage unavailable")
        val available = minOf(external.usableSpace, context.filesDir.usableSpace)
        receipt.put("incoming_bytes", spec.incomingBytes).put("internal_stage_bytes", spec.payloadBytes)
            .put("reserve_bytes", IndexTransferSpec.RESERVE_BYTES).put("required_bytes", spec.requiredBytes)
            .put("available_bytes", available)
        if (available < spec.requiredBytes) throw IOException("Insufficient space for incoming and internal staged index")
        ProbeInputDirectories.prepare(external.toPath(), listOf(spec.incomingName, "${spec.incomingName}/${spec.generation}")) {
            Os.stat(it.toString()).st_uid == Process.myUid()
        }
        receipt.put("status", "ready").put("incoming_name", spec.incomingName).put("generation", spec.generation)
    }
}
