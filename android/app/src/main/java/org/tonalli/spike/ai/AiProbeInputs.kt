package org.tonalli.spike.ai

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Process
import android.system.Os
import android.widget.TextView
import java.io.FileOutputStream
import java.io.IOException
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.util.UUID
import java.util.concurrent.Executors
import org.json.JSONObject
import org.tonalli.spike.benchmark.BenchmarkFiles
import org.tonalli.spike.probe.ProbeInputDirectories

/** Debug push preparation runs as the app UID; request IDs bind a fixed model and incoming directory. */
object AiProbeInputs {
    private const val ACTION = "org.tonalli.spike.PREPARE_AI_INPUT"
    private fun request(value: String): String = try {
        UUID.fromString(value).toString().also { if (it != value) throw IOException("Noncanonical AI request") }
    } catch (error: IllegalArgumentException) { throw IOException("Invalid AI request", error) }
    private fun receipt(context: Context, id: String): Path {
        val ready = context.filesDir.toPath().toRealPath().resolve("ai-inputs-ready")
        if (!Files.isDirectory(ready, NOFOLLOW_LINKS) || ready.toRealPath() != ready) throw IOException("Invalid AI receipt directory")
        return ready.resolve("$id.json")
    }

    fun incoming(context: Context, id: String, model: AiModelSpec): Path {
        request(id)
        val path = receipt(context, id)
        if (!Files.isRegularFile(path, NOFOLLOW_LINKS) || Files.size(path) > 4096) throw IOException("AI input request is not prepared")
        val json = JSONObject(Files.readAllBytes(path).toString(Charsets.UTF_8))
        if (json.getString("status") != "ready" || json.getString("request_id") != id ||
            json.getString("model_id") != model.id || json.getString("sha256") != model.sha256) throw IOException("AI input request differs from trusted model")
        val external = context.getExternalFilesDir(null) ?: throw IOException("App-specific incoming storage unavailable")
        val root = external.toPath().toRealPath()
        val incoming = root.resolve("ai-input-$id")
        if (!Files.isDirectory(incoming, NOFOLLOW_LINKS) || incoming.toRealPath() != incoming ||
            Os.stat(incoming.toString()).st_uid != Process.myUid()) throw IOException("AI input directory is not app-owned")
        return incoming
    }

    fun route(activity: Activity, intent: Intent): Boolean {
        if (intent.action != ACTION) return false
        if (activity.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE == 0) { activity.finish(); return true }
        val checked = runCatching {
            request(intent.getStringExtra("request_id") ?: throw IOException("Missing AI request")) to
                AiModelCatalog.model(intent.getStringExtra("model_id") ?: throw IOException("Missing AI model"))
        }.getOrElse { activity.finish(); return true }
        val (id, model) = checked
        val context = activity.applicationContext
        activity.setContentView(TextView(activity).apply { text = "Preparing app-owned AI model intake; no model is loaded." })
        val worker = Executors.newSingleThreadExecutor()
        worker.execute {
            try {
                val readyRoot = context.filesDir.toPath().toRealPath().resolve("ai-inputs-ready")
                if (!Files.exists(readyRoot, NOFOLLOW_LINKS)) Files.createDirectory(readyRoot)
                if (!Files.isDirectory(readyRoot, NOFOLLOW_LINKS) || readyRoot.toRealPath() != readyRoot) throw IOException("Invalid AI receipt directory")
                val output = BenchmarkFiles.claimResult(readyRoot, id)
                val json = JSONObject().put("version", 1).put("request_id", id).put("model_id", model.id).put("sha256", model.sha256)
                    .put("bytes", model.bytes).put("role", model.role)
                try {
                    val external = context.getExternalFilesDir(null) ?: throw IOException("App-specific incoming storage unavailable")
                    val available = external.usableSpace
                    val additional = AiModelFiles.requiredAdditionalBytes(model)
                    json.put("incoming_additional_bytes", additional).put("available_bytes", available)
                        .put("internal_additional_stage_bytes", additional)
                    // usableSpace already excludes existing incoming/installed files; internal staging checks again later.
                    if (available < additional) throw IOException("Insufficient incoming space")
                    ProbeInputDirectories.prepare(external.toPath().toRealPath(), listOf("ai-input-$id")) {
                        Os.stat(it.toString()).st_uid == Process.myUid()
                    }
                    json.put("status", "ready").put("incoming_name", "ai-input-$id").put("file_name", "model.gguf")
                } catch (error: Exception) { json.put("status", "failed").put("error", error.javaClass.simpleName) }
                val temp = Files.createTempFile(readyRoot, ".ai-ready-", ".json")
                try {
                    FileOutputStream(temp.toFile()).use { it.write(json.toString(2).toByteArray()); it.fd.sync() }
                    Files.move(temp, output, ATOMIC_MOVE)
                } finally { Files.deleteIfExists(temp) }
            } catch (_: Exception) {
                android.util.Log.e("TonalliM1", "AI input preparation failed")
            } finally {
                worker.shutdown()
                activity.runOnUiThread { if (!activity.isDestroyed) activity.finish() }
            }
        }
        return true
    }
}
