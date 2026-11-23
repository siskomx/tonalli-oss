package org.tonalli.spike.benchmark

import android.content.Context
import android.os.Process
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.StandardCopyOption.ATOMIC_MOVE
import java.nio.file.StandardCopyOption.REPLACE_EXISTING
import org.json.JSONObject

/** Safe process/session binding for the external driver; never exports launch credentials. */
object WebSessionReceipt {
    @Synchronized fun write(context: Context, session: String, request: WebBenchmarkRequest, uiId: String) {
        val root = File(context.filesDir, "benchmarks").toPath()
        Files.createDirectories(root)
        if (!Files.isDirectory(root, NOFOLLOW_LINKS)) throw IOException("Invalid private benchmark receipt directory")
        val value = JSONObject().put("version", 1).put("state", "ready").put("session_id", session)
            .put("pid", Process.myPid()).put("process_started_elapsed_ms", Process.getStartElapsedRealtime())
            .put("profile", request.profile).put("dataset_id", request.dataset).put("manifest_sha256", request.manifestHash)
            .put("source", "internal_mmap").put("ui_id", uiId)
        val staged = Files.createTempFile(root, ".web-session-", ".json")
        try {
            FileOutputStream(staged.toFile()).use { file -> file.write(value.toString(2).toByteArray(Charsets.UTF_8)); file.fd.sync() }
            Files.move(staged, root.resolve("webview-session.json"), ATOMIC_MOVE, REPLACE_EXISTING)
        } finally { Files.deleteIfExists(staged) }
    }

    @Synchronized fun clear(context: Context, session: String? = null) {
        val root = File(context.filesDir, "benchmarks").toPath()
        if (Files.isSymbolicLink(root)) throw IOException("Linked benchmark receipt directory")
        val file = root.resolve("webview-session.json")
        if (!Files.exists(file, NOFOLLOW_LINKS)) return
        if (session == null) { Files.delete(file); return }
        if (!Files.isRegularFile(file, NOFOLLOW_LINKS) || Files.size(file) > 4096) throw IOException("Invalid WebView session receipt")
        if (JSONObject(Files.readAllBytes(file).toString(Charsets.UTF_8)).getString("session_id") == session) {
            Files.delete(file)
        }
    }
}
