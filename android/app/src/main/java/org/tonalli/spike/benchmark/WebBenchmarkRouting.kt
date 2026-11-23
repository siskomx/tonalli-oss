package org.tonalli.spike.benchmark

import android.app.Activity
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.util.Log
import android.webkit.WebView
import java.io.IOException
import org.tonalli.spike.web.WebProbeActivity

/** Called by Main before any provider/normal WebView startup, in a fresh debug process. */
object WebBenchmarkRouting {
    private var configured: WebBenchmarkRequest? = null

    fun route(activity: Activity, intent: Intent): Boolean {
        if (intent.action !in setOf(WebBenchmarkRequest.ACTION, WebBenchmarkRequest.LOAD)) return false
        if ((activity.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) == 0) return true
        runCatching {
            val request = WebBenchmarkRequest.checked(intent.getStringExtra("dataset_id"),
                intent.getStringExtra("manifest_sha256"), intent.getStringExtra("profile"))
            if (intent.action == WebBenchmarkRequest.ACTION) {
                if (configured == null) {
                    WebView.setDataDirectorySuffix(request.profile)
                    configured = request
                } else if (configured != request) throw IOException("Restart required for another WebView profile")
            } else if (configured != request) throw IOException("No matching prepared WebView benchmark")
            activity.startActivity(Intent(activity, WebProbeActivity::class.java).setAction(intent.action)
                .putExtra("dataset_id", request.dataset).putExtra("manifest_sha256", request.manifestHash)
                .putExtra("profile", request.profile))
        }.onFailure { Log.i("TonalliM1", "WebView benchmark request rejected: ${it.javaClass.simpleName}") }
        return true
    }
}
