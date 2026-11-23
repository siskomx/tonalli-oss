package org.tonalli.spike.ai

import android.app.Activity
import android.app.ActivityManager
import android.os.Bundle
import android.os.SystemClock
import android.view.WindowManager
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import org.tonalli.spike.web.WorkerSession
import uniffi.tonalli_core.GenerationEvent
import uniffi.tonalli_core.PackException
import uniffi.tonalli_core.TokenSink
import uniffi.tonalli_core.aiCancel
import uniffi.tonalli_core.ramTier

/** Disposable Task11 native probe; no chat, history, model downloads or persisted prompts. */
class AiProbeActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private val embeddingCanceller = Executors.newSingleThreadExecutor()
    private val session = WorkerSession<AiNativeSession>()
    private val answerInputs = WorkerSession<AiAnswerProbe.Prepared>()
    private val gate = AiRunGate()
    private val embeddingRun = AiEmbeddingRun(gate::cancelled)
    private val actions = mutableListOf<Button>()
    private lateinit var output: TextView
    private lateinit var request: EditText
    private lateinit var prompt: EditText
    private lateinit var models: Spinner
    private var measurementStop: AtomicBoolean? = null
    private var probeRequest = AiProbeRequest(32u, null)
    private var requestRefusal: String? = null
    private var answerCase: AiAnswerCase? = null
    private var answerRefusal: String? = null
    @Volatile private var activeFixture: AiFixtureRecord? = null

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        runCatching {
            val extras = intent.extras
            val question = extras?.get("question")
            val context = extras?.get("context")
            if ((question != null && question !is String) || (context != null && context !is String))
                throw java.io.IOException("Comparison question/context must be strings")
            probeRequest = AiProbeRequest.parse(extras?.get("output_tokens"), question as String?, context as String?)
        }.onFailure { requestRefusal = "Comparison request refused: ${it.message}" }
        runCatching {
            val extras = intent.extras
            answerCase = AiAnswerCase.parse(extras?.get("answer_case"), extras?.get("question"), extras?.get("context"))
        }.onFailure { answerRefusal = "Cited-answer request refused: ${it.message}" }
        output = TextView(this).apply { setTextIsSelectable(true); setPadding(24, 12, 24, 12) }
        request = EditText(this).apply { hint = "Prepared request UUID"; isSingleLine = true }
        prompt = EditText(this).apply {
            hint = "One bounded generation prompt or embedding text"
            maxLines = 5
            isVerticalScrollBarEnabled = true
            setText(probeRequest.prompt ?: "Reply with one Spanish greeting. /no_think")
        }
        models = Spinner(this).apply {
            adapter = ArrayAdapter(this@AiProbeActivity, android.R.layout.simple_spinner_dropdown_item, AiModelCatalog.models.map { it.id })
        }
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(models); addView(request); addView(prompt)
            addView(action("Import verified model (all RAM tiers)") { prepare(false) })
            addView(action("Check Core load admission / info") { prepare(true) })
            addView(action("Generate at most ${probeRequest.outputTokens} tokens") { generate() })
            addView(action("Embed one bounded text") { embed() })
            addView(action("Rerank fixed 20-source fixture (3 Core calls)") { rerank() })
            addView(action("Cited SMALL answer ${answerCase?.let { "${it.id}/${it.language}" } ?: "(refused)"} — BM25, cap128") { answer() })
            addView(action("Run 10-minute foreground measurement") { measure() })
            addView(Button(this@AiProbeActivity).apply {
                text = "Cancel"
                setOnClickListener { cancelWork() }
            })
            addView(action("Close model") {
                cancelNative(gate.cancel())
                val stamp = gate.begin()
                busy(true)
                worker.execute {
                    val result = runCatching { try { answerInputs.release() } finally { session.release() } }
                    report(stamp, result.fold({ "Model closed." }, ::failure))
                }
            })
            addView(output)
        }
        setContentView(ScrollView(this).apply { addView(column) })
    }

    private fun action(label: String, work: () -> Unit) = Button(this).apply {
        text = label
        setOnClickListener { work() }
        actions.add(this)
    }

    private fun busy(value: Boolean) {
        actions.forEach { it.isEnabled = !value }
        models.isEnabled = !value
        request.isEnabled = !value
        prompt.isEnabled = !value
    }

    private fun actualMemory(): String {
        val memory = ActivityManager.MemoryInfo()
        getSystemService(ActivityManager::class.java).getMemoryInfo(memory)
        val total = memory.totalMem.coerceAtLeast(0)
        val tier = runCatching { ramTier(total.toULong()).name }.getOrDefault("unavailable")
        return "Actual Android RAM: total=$total available=${memory.availMem.coerceAtLeast(0)} tier=$tier. " +
            "Import is independent of RAM; Core checks actual AI admission.\n"
    }

    private fun prepare(load: Boolean) {
        val spec = AiModelCatalog.models[models.selectedItemPosition]
        val id = request.text.toString()
        val stamp = gate.begin()
        busy(true)
        output.text = actualMemory() + if (load) "Verifying internal model, then asking Core for load admission…" else "Verifying and importing model…"
        worker.execute {
            val result = runCatching {
                session.release()
                val incoming = AiProbeInputs.incoming(applicationContext, id, spec)
                val path = AiModelFiles.prepare(incoming, filesDir.toPath(), spec, cancelled = { gate.cancelled(stamp) || session.isClosing })
                if (load) {
                    if (gate.cancelled(stamp)) throw java.io.IOException("AI load cancelled")
                    val loaded = AiNativeSession.load(path, spec)
                    if (gate.cancelled(stamp)) { loaded.close(); throw java.io.IOException("AI load cancelled") }
                    if (!session.adopt(loaded)) throw java.io.IOException("AI probe departed")
                    val info = loaded.info
                    "Loaded ${spec.id}: bytes=${info.fileBytes}, training_context=${info.trainingContextTokens}, dimensions=${info.embeddingDimensions}, estimated_reserved_bytes=${info.estimatedReservedBytes}. Estimate is not PSS."
                } else "Imported ${spec.id}: verified ${spec.bytes} bytes, SHA256=${spec.sha256}. No model loaded."
            }
            report(stamp, result.fold({ it }, ::failure))
        }
    }

    private fun generate() {
        requestRefusal?.let { output.text = it; return }
        val input = prompt.text.toString()
        if (probeRequest.prompt != null && input != probeRequest.prompt) { output.text = "Fixture prompt changed; relaunch with the original paired fixture."; return }
        if (input.toByteArray().size > 65_536) { output.text = "Prompt exceeds the Core input bound."; return }
        val stamp = gate.begin()
        busy(true)
        output.text = actualMemory() + "Requesting one bounded generation…\n"
        if (probeRequest.question != null) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val fixtureForRequest = AtomicReference<AiFixtureRecord?>()
        val sink = object : TokenSink {
            override fun onEvent(event: GenerationEvent) {
                when (event) {
                    is GenerationEvent.Started -> {
                        fixtureForRequest.get()?.started(event)
                        val accepted = gate.started(stamp, event.id)
                        cancelNative(accepted.cancelId)
                        if (accepted.accepted) post(stamp) { output.append("Started id=${event.id}\n") }
                    }
                    is GenerationEvent.Text -> {
                        fixtureForRequest.get()?.text(event)
                        if (gate.text(stamp, event.id)) post(stamp) {
                            if (!gate.cancelled(stamp)) output.append(event.text)
                        }
                    }
                    is GenerationEvent.Finished -> {
                        // Signal the waiting fixture worker before any queued cleanup.
                        fixtureForRequest.get()?.finished(event)
                        if (gate.finished(stamp, event.id)) {
                            // Core has dropped its context before publishing this terminal event.
                            if (fixtureForRequest.get() == null) runCatching { worker.execute { if (gate.permits(stamp)) runCatching { session.release() } } }
                            post(stamp) {
                                val stats = event.stats
                                output.append("\nFinished ${event.reason}: input=${stats.inputTokens} output=${stats.outputTokens} setup_us=${stats.setupUs} prefill_us=${stats.prefillUs} first_text_us=${stats.firstTokenUs} decode_us=${stats.decodeUs} total_us=${stats.totalUs}; error_present=${event.error != null}\n")
                                if (fixtureForRequest.get() != null) window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                                busy(false)
                            }
                        }
                    }
                }
            }
        }
        worker.execute {
            var fixture: AiFixtureRecord? = null
            var fixtureReleased = false
            var fixtureStage = "fixture_submission"
            try {
                if (gate.cancelled(stamp)) throw java.io.IOException("Generation cancelled before submission")
                val loaded = session.current() ?: throw java.io.IOException("Load a generation model first")
                if (loaded.spec.role != "generation") throw java.io.IOException("Embedding model cannot be used for generation")
                if (probeRequest.question != null) {
                    fixture = AiFixtureRecord(applicationContext, loaded.spec, loaded.info.trainingContextTokens, probeRequest, input)
                    fixtureForRequest.set(fixture)
                    activeFixture = fixture
                }
                cancelNative(gate.returned(stamp, loaded.generate(input, sink, probeRequest.outputTokens)))
                fixtureStage = "fixture_terminal_wait"
                fixture?.awaitTerminal { cancelNative(gate.cancel(stamp)) }
                if (fixture != null) {
                    fixtureStage = "fixture_model_release"
                    fixtureReleased = true
                    session.release()
                }
                fixtureStage = "fixture_terminal_write"
                fixture?.finish()
                fixture?.let { report(stamp, "Fixture trace: ai-measurements/${it.directory.fileName}/fixture.json. Trace completeness is not a gate PASS.") }
            } catch (error: Exception) {
                val snapshot = fixture?.evidence?.snapshot()
                if (snapshot?.started != null && snapshot.terminal == null) {
                    runCatching { fixture?.awaitTerminal { cancelNative(gate.cancel(stamp)) } }
                }
                runCatching { fixture?.failure(error, fixtureStage) }
                report(stamp, failure(error))
            } finally {
                if (fixture != null) {
                    if (activeFixture === fixture) activeFixture = null
                    if (!fixtureReleased) {
                        try { session.release() } catch (error: Exception) {
                            runCatching { fixture?.failure(error, "fixture_model_release") }
                            report(stamp, failure(error))
                        }
                    }
                }
                if (probeRequest.question != null) post(stamp) { window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
            }
        }
    }

    private fun embed() {
        val input = try { AiEmbeddingProbe.input(prompt.text.toString()) }
            catch (error: Exception) { output.text = failure(error); return }
        val stamp = gate.begin()
        busy(true)
        output.text = actualMemory() + "Requesting one bounded embedding; Core checks actual native tokens (at most min(512, training context)).\n"
        worker.execute {
            var result = runCatching {
                if (gate.cancelled(stamp)) throw java.io.IOException("Embedding cancelled before submission")
                val loaded = session.current() ?: throw java.io.IOException("Load the verified embedding model first")
                if (loaded.spec.role != "embedding") throw java.io.IOException("Generation model cannot be used for embedding")
                if (!embeddingRun.publish(stamp, loaded::requestEmbeddingCancel)) throw java.io.IOException("Embedding cancelled before submission")
                val started = SystemClock.elapsedRealtimeNanos()
                val vectors = loaded.embed(input)
                val duration = SystemClock.elapsedRealtimeNanos() - started
                val checked = AiEmbeddingProbe.assess(vectors, loaded.info.embeddingDimensions.toInt())
                "Embedded one text: dimensions=${checked.dimensions}, finite=true, L2_norm=${checked.norm}, native_call_ns=$duration. Actual token count is not returned by this API. Model closed; reload before another request."
            }
            embeddingRun.clear(stamp)
            try { session.release() } catch (cleanup: Exception) {
                val primary = result.exceptionOrNull()
                if (primary != null && primary !== cleanup) primary.addSuppressed(cleanup)
                else if (primary == null) result = Result.failure(cleanup)
            }
            report(stamp, result.fold({ it }, ::failure))
        }
    }

    private fun cancelNative(id: ULong?) {
        // Independent of the single loader executor, including a pending generate return.
        if (id != null) runCatching { aiCancel(id) }
    }

    private fun rerank() {
        val query = intent.getStringExtra("question")
        val rawContext = intent.getStringExtra("context")
        if (query == null || rawContext == null) {
            output.text = "Rerank requires original paired question/context extras from the trusted fixture catalog."
            return
        }
        val stamp = gate.begin()
        busy(true)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        output.text = actualMemory() + "Verifying original rerank fixture, then requesting three Core calls. The editable prompt is not used.\n"
        worker.execute {
            var runnerOwnsRelease = false
            var result = runCatching {
                if (gate.cancelled(stamp)) throw java.io.IOException("Rerank cancelled before submission")
                val loaded = session.current() ?: throw java.io.IOException("Load the verified BGE embedding model first")
                if (loaded.spec.role != "embedding") throw java.io.IOException("Generation model cannot be used for reranking")
                if (!embeddingRun.publish(stamp, loaded::requestEmbeddingCancel)) throw java.io.IOException("Rerank cancelled before submission")
                runnerOwnsRelease = true
                AiRerankProbe.run(applicationContext, loaded, query, rawContext,
                    { gate.cancelled(stamp) || session.isClosing }, { session.release() })
            }
            embeddingRun.clear(stamp)
            if (!runnerOwnsRelease) try { session.release() } catch (cleanup: Exception) {
                val primary = result.exceptionOrNull()
                if (primary != null && primary !== cleanup) primary.addSuppressed(cleanup)
                else if (primary == null) result = Result.failure(cleanup)
            }
            report(stamp, result.fold({ it }, ::failure))
            post(stamp) { window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
        }
    }

    private fun cancelWork() {
        measurementStop?.set(true)
        activeFixture?.cancellationRequested()
        cancelNative(gate.cancel())
        val closeExactModel = embeddingRun.cancellation()
        if (closeExactModel != null) runCatching {
            embeddingCanceller.execute { runCatching { closeExactModel() } }
        }
    }

    private fun measure() {
        val stop = AtomicBoolean(false)
        measurementStop = stop
        val stamp = gate.begin()
        busy(true)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        output.text = actualMemory() + "Calibrating actual 2,000 input tokens, then measuring for ten minutes. Stay on this screen.\n"
        worker.execute {
            var result = runCatching {
                if (stop.get() || gate.cancelled(stamp)) throw java.io.IOException("Measurement cancelled before submission")
                val loaded = session.current() ?: throw java.io.IOException("Load a generation model first")
                if (loaded.spec.role != "generation") throw java.io.IOException("Embedding model cannot be used for generation measurement")
                AiMeasurementMetrics(applicationContext, loaded.spec).use { metrics ->
                    AiMeasurementWorker(loaded, gate, { stop.get() || session.isClosing }, metrics).run()
                }
            }
            try { session.release() } catch (cleanup: Exception) {
                val primary = result.exceptionOrNull()
                if (primary != null && primary !== cleanup) primary.addSuppressed(cleanup)
                else if (primary == null) result = Result.failure(cleanup)
            }
            runOnUiThread {
                if (measurementStop === stop && !isDestroyed && !session.isClosing) {
                    measurementStop = null
                    window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                    output.append("\n" + result.fold({ it }, ::failure) + "\n")
                    busy(false)
                }
            }
        }
    }

    private fun failure(error: Throwable): String = if (error is PackException.Mismatch) {
        "Core Mismatch: ${error.v1.take(256)}"
    } else "Probe refused/failed: ${error.javaClass.simpleName}. Existing installed models were preserved."

    private fun answer() {
        answerRefusal?.let { output.text = it; return }
        val selected = answerCase ?: return
        val epochGate = gate
        val stamp = epochGate.begin()
        busy(true)
        output.text = actualMemory() + "Preparing the fixed SMALL index and one cited answer…\n"
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val context = applicationContext
        worker.execute {
            var result = runCatching {
                if (epochGate.cancelled(stamp)) throw java.io.IOException("Answer cancelled before preparation")
                val loaded = session.current() ?: throw java.io.IOException("Load a generation model first")
                if (loaded.spec.role != "generation") throw java.io.IOException("An embedding model cannot generate the cited answer")
                val prepared = answerInputs.current() ?: AiAnswerProbe.prepare(context) { epochGate.cancelled(stamp) }.also {
                    if (epochGate.cancelled(stamp)) { it.close(); throw java.io.IOException("Answer preparation cancelled") }
                    if (!answerInputs.adopt(it)) throw java.io.IOException("Answer probe departed")
                }
                AiAnswerProbe.run(context, prepared, loaded, selected, epochGate, stamp)
            }
            if (result.isFailure) {
                try { answerInputs.release() } catch (cleanup: Throwable) {
                    val primary = result.exceptionOrNull()
                    if (primary != null && primary !== cleanup) primary.addSuppressed(cleanup)
                    else result = Result.failure(cleanup)
                }
            }
            post(stamp) {
                window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                output.append("\n" + result.fold({ it }, ::failure) + "\n")
                busy(false)
            }
        }
    }

    private fun post(stamp: Long, work: () -> Unit) = runOnUiThread { if (gate.permits(stamp) && !session.isClosing) work() }
    private fun report(stamp: Long, message: String) = post(stamp) { output.append("\n$message\n"); busy(false) }

    override fun onResume() {
        super.onResume()
        gate.resume()
        output.text = actualMemory() + "Explicit M1 import/load/generation/embedding only. Only explicit paired fixtures record raw output; no normal prompt history. Lite execution refusal is expected."
        probeRequest.prompt?.let { prompt.setText(it) }
        requestRefusal?.let { output.append("\n$it") }
        busy(false)
    }

    override fun onPause() {
        cancelWork()
        measurementStop = null
        window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        cancelNative(gate.depart())
        prompt.text?.clear()
        output.text = ""
        worker.execute { runCatching { try { answerInputs.release() } finally { session.release() } } }
        super.onPause()
    }

    override fun onDestroy() {
        window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        cancelWork()
        cancelNative(gate.depart())
        session.requestClose()
        answerInputs.requestClose()
        worker.execute { runCatching { try { answerInputs.release() } finally { session.release() } } }
        worker.shutdown()
        embeddingCanceller.shutdown()
        super.onDestroy()
    }
}
