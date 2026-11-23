package org.tonalli.spike.benchmark

import android.app.Activity
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.util.Log
import android.view.WindowManager
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import org.tonalli.spike.saf.SafBenchmarkBinding
import org.tonalli.spike.saf.SafBindingRequest
import org.tonalli.spike.saf.SafBenchmarkErrors
import org.tonalli.spike.index.TrustedIndexProbe
import org.tonalli.spike.index.IndexTransferUnavailable

class BenchmarkActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private val destroyed = AtomicBoolean(false)
    private val active = AtomicBoolean(false)
    private val current = AtomicReference<AtomicBoolean?>()
    private var busy = false
    private data class Pending(val request: BenchmarkRequest?, val saf: SafBindingRequest?, val bindOnly: Boolean = false)
    private var pending: Pending? = null
    private lateinit var dataset: Spinner
    private lateinit var status: TextView
    private val actions = mutableListOf<Button>()

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        dataset = Spinner(this).apply {
            val ids = BenchmarkDataset.ids(this@BenchmarkActivity)
            adapter = ArrayAdapter(this@BenchmarkActivity, android.R.layout.simple_spinner_dropdown_item, ids)
            setSelection(ids.indexOf("small").coerceAtLeast(0))
        }
        status = TextView(this).apply { setTextIsSelectable(true) }
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(24, 16, 24, 16)
            addView(TextView(this@BenchmarkActivity).apply {
                text = "Internal mmap benchmark. Prepare explicitly copies and verifies the selected dataset; full input needs over 53 GB of additional internal space. Small is functional validation only. Fresh process is not OS-cold."
            })
            addView(dataset)
            addView(button("Prepare selected dataset (copy to internal)") {
                val id = dataset.selectedItem?.toString() ?: return@button
                job { cancelled ->
                    val input = BenchmarkDataset.load(applicationContext, id)
                    input.prepare(applicationContext, cancelled)
                    "Prepared $id; SHA-256 checked and native title table validated. No performance result recorded."
                }
            })
            addView(button("Run 30 warm suggest + article samples") {
                val id = dataset.selectedItem?.toString() ?: return@button
                job { cancelled ->
                    val input = BenchmarkDataset.load(applicationContext, id)
                    val request = BenchmarkRequest.checked(id, input.manifestHash, "manual-${UUID.randomUUID()}", "warm", 30)
                    val output = NativeBenchmark.run(applicationContext, input, request, cancelled)
                    "Recorded ${output.name}; no performance gate is inferred.\n${output.readText()}"
                }
            })
            addView(TextView(this@BenchmarkActivity).apply {
                text = "SAF benchmarks use a separately selected standalone/split ZIM. Reselect once to create its revision, prepare only the verified internal title table, then explicitly bind benchmark/saf-receipt.json. Its manual SHA statement is not current Android-byte verification or proof of physical SD."
            })
            addView(button("Prepare SAF benchmark title table only") {
                val id = dataset.selectedItem?.toString() ?: return@button
                job { cancelled ->
                    BenchmarkDataset.load(applicationContext, id).prepareTable(applicationContext, cancelled)
                    "Prepared the verified internal title table and validated attachment to the selected SAF pack."
                }
            })
            addView(button("Bind external manual SAF source receipt") {
                val id = dataset.selectedItem?.toString() ?: return@button
                job { cancelled ->
                    val bound = SafBenchmarkBinding.bind(applicationContext, BenchmarkDataset.load(applicationContext, id), null, cancelled)
                    "Bound manual receipt ${bound.receiptHash}; source evidence remains external_manual_not_rechecked."
                }
            })
            addView(button("Run 30 warm samples using bound SAF source") {
                val id = dataset.selectedItem?.toString() ?: return@button
                job { cancelled ->
                    val input = BenchmarkDataset.load(applicationContext, id)
                    val bound = SafBenchmarkBinding.load(applicationContext, input)
                    val request = BenchmarkRequest.checked(id, input.manifestHash, "saf-${UUID.randomUUID()}", "warm", 30)
                    val output = NativeBenchmark.run(applicationContext, input, request, cancelled, bound)
                    "Recorded ${output.name}; source saf_pread, manual content identity not rechecked.\n${output.readText()}"
                }
            })
            addView(button("Validate bound SAF index handoff (selected dataset)") {
                val id = dataset.selectedItem?.toString() ?: return@button
                job { cancelled -> TrustedIndexProbe(applicationContext).run(id, cancelled) }
            })
            addView(Button(this@BenchmarkActivity).apply {
                text = "Cancel current work"
                setOnClickListener { current.get()?.set(true); status.text = "Cancellation requested; installed versions are preserved." }
            })
            addView(Button(this@BenchmarkActivity).apply { text = "Close benchmark"; setOnClickListener { finish() } })
            addView(status)
        }
        setContentView(ScrollView(this).apply { addView(column) })
        pending = automation(intent)
    }

    private fun button(label: String, click: () -> Unit) = Button(this).apply {
        text = label; setOnClickListener { click() }; actions.add(this)
    }

    private fun job(work: (cancelled: () -> Boolean) -> String) {
        if (busy || destroyed.get()) { status.text = "Benchmark is busy; retry after it finishes."; return }
        val cancel = AtomicBoolean(false)
        current.set(cancel)
        busy = true
        actions.forEach { it.isEnabled = false }
        dataset.isEnabled = false
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        status.text = "Working on selected benchmark input; leaving this screen requests cancellation."
        worker.execute {
            val cancelled = { cancel.get() || destroyed.get() || !active.get() }
            val result = runCatching {
                if (cancelled()) throw java.io.IOException("Cancelled")
                work(cancelled)
            }.getOrElse { if (it is IndexTransferUnavailable) it.userMessage else SafBenchmarkErrors.message(it) }
            runOnUiThread {
                if (!destroyed.get() && current.get() === cancel) {
                    busy = false
                    actions.forEach { it.isEnabled = true }
                    dataset.isEnabled = true
                    window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                    status.text = result
                }
            }
        }
    }

    private fun automation(intent: Intent): Pending? {
        if (intent.action !in setOf(BenchmarkRequest.ACTION, SafBindingRequest.ACTION, SafBindingRequest.BIND)) return null
        if ((applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) == 0) return null
        return runCatching {
            val saf = if (intent.action == BenchmarkRequest.ACTION) null else SafBindingRequest.checked(
                intent.getStringExtra("dataset_id"), intent.getStringExtra("manifest_sha256"), intent.getStringExtra("selection_revision"),
                intent.getStringExtra("selection_fingerprint"), intent.getStringExtra("manual_receipt_sha256"))
            if (intent.action == SafBindingRequest.BIND) return@runCatching Pending(null, saf, true)
            val request = BenchmarkRequest.checked(intent.getStringExtra("dataset_id"), intent.getStringExtra("manifest_sha256"),
                intent.getStringExtra("run_id"), intent.getStringExtra("mode"),
                if (intent.hasExtra("sample_count")) intent.getIntExtra("sample_count", -1) else null)
            Pending(request, saf)
        }.getOrElse { status.text = "Invalid benchmark request rejected."; null }
    }

    private fun runPending() {
        val item = pending ?: return
        pending = null
        val id = item.request?.dataset ?: item.saf?.dataset ?: return
        if (!busy) {
            (0 until dataset.adapter.count).firstOrNull { dataset.adapter.getItem(it) == id }?.let(dataset::setSelection)
        }
        job { cancelled ->
            val input = BenchmarkDataset.load(applicationContext, id)
            if (item.bindOnly) {
                val bound = SafBenchmarkBinding.bind(applicationContext, input, item.saf, cancelled)
                return@job "Bound manual receipt ${bound.receiptHash}; current provider bytes were not SHA-verified."
            }
            val request = item.request ?: throw java.io.IOException("Missing benchmark request")
            val bound = item.saf?.let { SafBenchmarkBinding.load(applicationContext, input, it) }
            val output = NativeBenchmark.run(applicationContext, input, request, cancelled, bound)
            Log.i("TonalliM1", "BENCH_RESULT ${request.runId}")
            "Benchmark result recorded: ${output.name}; read its status and raw samples."
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        pending = automation(intent)
        if (active.get()) runPending()
    }

    override fun onResume() { super.onResume(); active.set(true); runPending() }
    override fun onPause() {
        active.set(false); current.get()?.set(true)
        window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        super.onPause()
    }
    override fun onDestroy() { destroyed.set(true); current.get()?.set(true); worker.shutdown(); super.onDestroy() }
}
