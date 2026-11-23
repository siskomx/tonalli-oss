package org.tonalli.spike

import android.app.Activity
import android.app.ActivityManager
import android.content.Intent
import android.os.Bundle
import android.util.Log
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import java.io.File
import java.util.concurrent.Executors
import org.tonalli.spike.benchmark.BenchmarkActivity
import org.tonalli.spike.benchmark.BenchmarkRouting
import org.tonalli.spike.benchmark.WebBenchmarkRouting
import org.tonalli.spike.index.IndexProbe
import org.tonalli.spike.notes.NotesProbeActivity
import org.tonalli.spike.saf.RemovableProbeActivity
import org.tonalli.spike.probe.ProbeInputRouting
import org.tonalli.spike.ai.AiProbeInputs
import org.tonalli.spike.ai.AiProbeActivity
import org.tonalli.spike.web.WebProbeActivity
import uniffi.tonalli_core.zimSuggest
import uniffi.tonalli_core.ramTier
import uniffi.tonalli_core.RamTier

/** Normal launches enter the shared Library; explicit advanced actions retain diagnostic tools. */
class MainActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private lateinit var output: TextView
    private lateinit var run: Button
    private lateinit var runIndex: Button

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        if (AiProbeInputs.route(this, intent)) return
        if (ProbeInputRouting.route(this, intent)) return
        if (BenchmarkRouting.route(this, intent) || WebBenchmarkRouting.route(this, intent)) { finish(); return }
        if (intent.action != "org.tonalli.spike.DIAGNOSTICS") {
            startActivity(Intent(this, WebProbeActivity::class.java).setAction(WebProbeActivity.OPEN_READER))
            finish()
            return
        }
        output = TextView(this).apply { setTextIsSelectable(true); setPadding(24, 24, 24, 24) }
        run = Button(this).apply {
            text = "Run ZIM title probe"
            setOnClickListener { probe() }
        }
        runIndex = Button(this).apply {
            text = "Run small index portability probe"
            setOnClickListener { runProbe { IndexProbe(this@MainActivity).run() } }
        }
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(Button(this@MainActivity).apply {
                text = "Open optional AI M1 probe"
                setOnClickListener {
                    val destination = Intent(this@MainActivity, AiProbeActivity::class.java)
                    if (applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0) {
                        // onNewIntent updates Main's intent; copy only fixture keys, preserving types/bytes for private validation.
                        this@MainActivity.intent.extras?.let { source ->
                            val forwarded = Bundle(source)
                            for (key in forwarded.keySet().toList()) {
                                if (key !in setOf("output_tokens", "question", "context", "answer_case")) forwarded.remove(key)
                            }
                            destination.putExtras(forwarded)
                        }
                    }
                    startActivity(destination)
                }
            })
            addView(Button(this@MainActivity).apply {
                text = "Open LAN sharing probe"
                setOnClickListener { startActivity(Intent(this@MainActivity, org.tonalli.spike.lan.LanProbeActivity::class.java)) }
            })
            addView(run)
            addView(runIndex)
            addView(Button(this@MainActivity).apply {
                text = "Open removable document probe"
                setOnClickListener { startActivity(Intent(this@MainActivity, RemovableProbeActivity::class.java)) }
            })
            addView(Button(this@MainActivity).apply {
                text = "Open internal benchmark probe"
                setOnClickListener { startActivity(Intent(this@MainActivity, BenchmarkActivity::class.java)) }
            })
            addView(Button(this@MainActivity).apply {
                text = "Open encrypted notes probe"
                setOnClickListener { startActivity(Intent(this@MainActivity, NotesProbeActivity::class.java)) }
            })
            addView(Button(this@MainActivity).apply {
                text = "Open shared offline UI"
                setOnClickListener { startActivity(Intent(this@MainActivity, WebProbeActivity::class.java)) }
            })
            addView(Button(this@MainActivity).apply {
                text = "Open shared AI chat"
                setOnClickListener { startActivity(Intent(this@MainActivity, WebProbeActivity::class.java).setAction(WebProbeActivity.OPEN_AI)) }
            })
            addView(ScrollView(this@MainActivity).apply { addView(output) })
        }
        setContentView(column)
        val info = ActivityManager.MemoryInfo()
        getSystemService(ActivityManager::class.java).getMemoryInfo(info)
        val advertised = info.advertisedMem.coerceAtLeast(0)
        val total = info.totalMem.coerceAtLeast(0)
        // Advertised RAM can exceed the memory actually visible to Android.
        val tier = runCatching {
            when (ramTier(total.toULong())) {
                RamTier.LITE -> "Lite: offline library, maps and notes work. Local AI is off on this phone; small caches and serial indexing."
                RamTier.STANDARD -> "Standard: all offline features work. Local AI uses the 1.7B/4B model class and medium caches."
                RamTier.REFERENCE -> "Reference: all offline features work. Local AI uses the 4B–8B model class and large caches."
            }
        }.getOrElse { "RAM policy unavailable; offline features remain available." }
        output.text = "${applicationInfo.loadLabel(packageManager)} — actual phone gate pending\n" +
            "Android API: ${android.os.Build.VERSION.SDK_INT}\n" +
            "RAM advertised: $advertised bytes; total: $total bytes; available: ${info.availMem.coerceAtLeast(0)} bytes\n" +
            "$tier\n" +
            "External pack folder: ${getExternalFilesDir(null)}\n" +
            "Copy the verified small fixture here as probe.zim, then run the probe.\n" +
            "Internal indexes and encrypted notes belong under $filesDir.\n" +
            "Title/index probes do not start a listener. Open shared offline UI to start loopback only."
        Log.i("TonalliM1", output.text.toString())
    }

    private fun probe() = runProbe {
        val external = getExternalFilesDir(null) ?: error("External app storage is unavailable")
        withPack(File(external, "probe.zim")) { pack ->
            val prefix = intent.getStringExtra("prefix") ?: "Trefoil"
            val titles = zimSuggest(pack, prefix, 10u)
            "SUCCESS — ${titles.size} suggestions for '$prefix'\n" +
                titles.joinToString("\n") { "${it.title}\n  ${it.url}" }
        }
    }

    private fun runProbe(work: () -> String) {
        run.isEnabled = false
        runIndex.isEnabled = false
        worker.execute {
            val result = runCatching(work).fold(onSuccess = { it }, onFailure = { "FAILED — ${it.javaClass.simpleName}: ${it.message}" })
            Log.i("TonalliM1", result)
            runOnUiThread {
                if (!isDestroyed) { output.text = result; run.isEnabled = true; runIndex.isEnabled = true }
            }
        }
    }

    override fun onDestroy() {
        worker.shutdown()
        super.onDestroy()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        if (AiProbeInputs.route(this, intent)) return
        if (ProbeInputRouting.route(this, intent)) return
        if (BenchmarkRouting.route(this, intent) || WebBenchmarkRouting.route(this, intent)) finish()
        else if (intent.action != "org.tonalli.spike.DIAGNOSTICS") {
            startActivity(Intent(this, WebProbeActivity::class.java).setAction(WebProbeActivity.OPEN_READER))
            finish()
        }
    }
}
