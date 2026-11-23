package org.tonalli.spike.benchmark

import android.app.Activity
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.util.Log
import org.tonalli.spike.saf.SafBindingRequest

/** The existing exported launcher is the only external automation entry point. */
object BenchmarkRouting {
    fun route(activity: Activity, intent: Intent): Boolean {
        if (intent.action !in setOf(BenchmarkRequest.ACTION, SafBindingRequest.ACTION, SafBindingRequest.BIND)) return false
        if ((activity.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) == 0) return true
        if (intent.action != BenchmarkRequest.ACTION) return routeSaf(activity, intent)
        val request = runCatching {
            BenchmarkRequest.checked(intent.getStringExtra("dataset_id"), intent.getStringExtra("manifest_sha256"),
                intent.getStringExtra("run_id"), intent.getStringExtra("mode"),
                if (intent.hasExtra("sample_count")) intent.getIntExtra("sample_count", -1) else null)
        }.getOrElse { Log.i("TonalliM1", "Invalid benchmark request rejected"); return true }
        activity.startActivity(Intent(activity, BenchmarkActivity::class.java).setAction(BenchmarkRequest.ACTION)
            .putExtra("dataset_id", request.dataset).putExtra("manifest_sha256", request.manifestHash)
            .putExtra("run_id", request.runId).putExtra("mode", request.mode)
            .putExtra("sample_count", if (request.mode == "warm") 30 else 1))
        return true
    }

    private fun routeSaf(activity: Activity, intent: Intent): Boolean {
        runCatching {
            val binding = SafBindingRequest.checked(intent.getStringExtra("dataset_id"), intent.getStringExtra("manifest_sha256"),
                intent.getStringExtra("selection_revision"), intent.getStringExtra("selection_fingerprint"), intent.getStringExtra("manual_receipt_sha256"))
            val target = Intent(activity, BenchmarkActivity::class.java).setAction(intent.action)
                .putExtra("dataset_id", binding.dataset).putExtra("manifest_sha256", binding.manifestHash)
                .putExtra("selection_revision", binding.revision).putExtra("selection_fingerprint", binding.fingerprint)
                .putExtra("manual_receipt_sha256", binding.receiptHash)
            if (intent.action == SafBindingRequest.ACTION) {
                val request = BenchmarkRequest.checked(binding.dataset, binding.manifestHash, intent.getStringExtra("run_id"),
                    intent.getStringExtra("mode"), if (intent.hasExtra("sample_count")) intent.getIntExtra("sample_count", -1) else null)
                target.putExtra("run_id", request.runId).putExtra("mode", request.mode)
                    .putExtra("sample_count", if (request.mode == "warm") 30 else 1)
            }
            activity.startActivity(target)
        }.onFailure { Log.i("TonalliM1", "Invalid SAF benchmark request rejected") }
        return true
    }
}
