package org.tonalli.litert

import android.app.Activity
import android.app.ActivityManager
import android.os.Bundle
import android.os.SystemClock
import android.view.WindowManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import com.google.ai.edge.litertlm.*
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/** Isolated text-only experiment; no shared app engine, history, downloader or network permission. */
@OptIn(ExperimentalApi::class)
class ProbeActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private val canceller = Executors.newSingleThreadExecutor()
    private val busy = AtomicBoolean(false)
    @Volatile private var lease: ProbeLease? = null
    @Volatile private var destroyed = false
    @Volatile private var records: ProbeRecords? = null
    private lateinit var status: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val column = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(20, 20, 20, 20) }
        column.addView(TextView(this).apply { text = getString(applicationInfo.labelRes) + "\nCPU only; unmeasured candidate. Stage trusted model before running." })
        val question = EditText(this).apply {
            hint = "Question"
            maxLines = 5
            isVerticalScrollBarEnabled = true
            setText(intent.getStringExtra("question") ?: "According to the supplied passage, what is photosynthesis?")
        }
        val context = EditText(this).apply {
            hint = "Retrieved passage (same bytes as Qwen comparison)"
            maxLines = 5
            isVerticalScrollBarEnabled = true
            setText(intent.getStringExtra("context") ?: "")
        }
        column.addView(question); column.addView(context)
        column.addView(Button(this).apply {
            text = "Run one offline probe"
            setOnClickListener {
                val q = question.text.toString(); val c = context.text.toString()
                if (q.isBlank() || q.length + c.length > 32768) append("Refused: missing question or oversized fixture")
                else runProbe(q, c)
            }
        })
        column.addView(Button(this).apply { text = "Cancel"; setOnClickListener { requestCancel() } })
        status = TextView(this).apply { setTextIsSelectable(true) }
        column.addView(status)
        setContentView(ScrollView(this).apply { addView(column) })
    }

    private fun append(text: String) {
        records?.offer("status", text)
        display(text)
    }

    private fun display(text: String) {
        runOnUiThread { if (!destroyed) status.append(text + "\n") }
    }

    private fun requestCancel() {
        val current = lease ?: return
        append("Cancellation requested; waiting for native terminal callback. Initialization cannot be interrupted by this API.")
        try {
            canceller.execute { try { current.cancel() } catch (error: Exception) { append("Cancel error: ${error.javaClass.simpleName}: ${error.message}") } }
        } catch (_: RejectedExecutionException) {
            // Shutdown follows disposal; a lease captured just before completion needs no further cancel.
        }
    }

    private fun runProbe(question: String, context: String) {
        if (!busy.compareAndSet(false, true)) { append("Probe already active"); return }
        val current = ProbeLease()
        lease = current
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val requested = SystemClock.elapsedRealtimeNanos()
        worker.execute {
            var engine: Engine? = null
            var conversation: Conversation? = null
            var initialized = false
            var possibleAccepted = false
            var statsRecorded = false
            var disposalSucceeded = true
            val nativeDone = AtomicBoolean(false)
            val terminal = CountDownLatch(1)
            var log: ProbeRecords? = null
            try {
                val trustedPrivate = filesDir.toPath().toRealPath()
                val activeLog = ProbeRecords(trustedPrivate)
                log = activeLog
                records = activeLog
                append("run_id=${activeLog.id}; private_record=files/litert-probe-records/${activeLog.id}.jsonl")
                val external = getExternalFilesDir(null) ?: error("Incoming app directory unavailable")
                // Resolve Context-owned OS aliases before validating our owned children.
                val trustedExternal = external.toPath().toRealPath()
                val incoming = trustedExternal.resolve("incoming")
                val internal = trustedPrivate.resolve("litert-models")
                for (directory in listOf(incoming, internal)) {
                    if (!java.nio.file.Files.exists(directory, java.nio.file.LinkOption.NOFOLLOW_LINKS)) java.nio.file.Files.createDirectory(directory)
                }
                append("Verifying and staging trusted model into internal storage")
                val model = ModelStaging.prepare(incoming, internal, cancelled = current::isCancelled)
                if (current.isCancelled()) return@execute
                val budget = intent.getIntExtra("context_tokens", 2048)
                require(budget in setOf(1024, 2048, 4096)) { "Unsupported context budget" }
                val memory = ActivityManager.MemoryInfo()
                getSystemService(ActivityManager::class.java).getMemoryInfo(memory)
                // Temporary conservative admission assumption; this is not a measured runtime budget.
                val workspaceReserve = 1024L * 1024 * 1024
                val osReserve = maxOf(workspaceReserve, memory.threshold)
                val requiredAvailable = TrustedModel.GEMMA.bytes + workspaceReserve + osReserve
                append("RAM admission: available_bytes=${memory.availMem}; required_bytes=$requiredAvailable; full_file_bytes=${TrustedModel.GEMMA.bytes}; workspace_reserve=$workspaceReserve; OS_reserve=$osReserve; low_memory=${memory.lowMemory}. Admission is not a fit guarantee.")
                check(!memory.lowMemory && memory.availMem >= requiredAvailable) { "Insufficient current available RAM for conservative experiment admission" }
                fun digest(text: String) = java.security.MessageDigest.getInstance("SHA-256")
                    .digest(text.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }
                val systemText = "Answer only from the provided passage, in the question's language. Cite supporting numbered sources as [n]. If it does not contain the answer, say so. Keep the answer concise."
                activeLog.offer("running_metadata", "before SDK construction/initialization", fields = mapOf(
                    "uid" to android.os.Process.myUid().toString(), "pid" to android.os.Process.myPid().toString(),
                    "model_sha256" to TrustedModel.GEMMA.sha256, "model_bytes" to TrustedModel.GEMMA.bytes.toString(),
                    "context_tokens" to budget.toString(), "output_tokens" to "128", "backend" to "CPU", "threads" to "4",
                    "instruction_placement" to "system; model-specific chat template and tokenizer",
                    "thinking" to "false", "thinking_budget" to "0", "speculative" to "OFF",
                    "sampler_top_k" to "20", "sampler_top_p" to "0.8", "sampler_temperature" to "0.7", "sampler_seed" to "42",
                    "system_sha256" to digest(systemText), "question_sha256" to digest(question), "context_sha256" to digest(context),
                    "user_message_sha256" to digest("Passage:\n$context\n\nQuestion:\n$question")))
                activeLog.checkpoint(::display)
                ExperimentalFlags.enableBenchmark = true
                activeLog.offer("engine_constructor_start", "marker before durable checkpoint; exact call bounds follow on return")
                activeLog.checkpoint(::display)
                val constructorStartNs = SystemClock.elapsedRealtimeNanos()
                val loaded = Engine(EngineConfig(modelPath = model.toString(), backend = Backend.CPU(threadCount = 4),
                    maxNumTokens = budget, cacheDir = File(cacheDir, "litert").apply { mkdirs() }.path,
                    speculativeDecodingConfig = SpeculativeDecodingConfig(mode = SpeculativeDecodingMode.OFF)))
                val constructorEndNs = SystemClock.elapsedRealtimeNanos()
                engine = loaded
                activeLog.offer("engine_constructor_end", "", fields = mapOf("call_start_ns" to constructorStartNs.toString(), "call_end_ns" to constructorEndNs.toString(), "call_wall_ns" to (constructorEndNs - constructorStartNs).toString()))
                append("Initializing; cancellation is sticky but cannot abort native initialization")
                activeLog.offer("engine_initialize_start", "marker before durable checkpoint; exact call bounds follow on return")
                activeLog.checkpoint(::display)
                val initializeStartNs = SystemClock.elapsedRealtimeNanos()
                loaded.initialize()
                val initializeEndNs = SystemClock.elapsedRealtimeNanos()
                initialized = true
                activeLog.offer("engine_initialize_end", "", fields = mapOf("call_start_ns" to initializeStartNs.toString(), "call_end_ns" to initializeEndNs.toString(), "call_wall_ns" to (initializeEndNs - initializeStartNs).toString()))
                if (current.isCancelled()) return@execute
                activeLog.offer("conversation_create_start", "marker before durable checkpoint; exact call bounds follow on return")
                activeLog.checkpoint(::display)
                val conversationStartNs = SystemClock.elapsedRealtimeNanos()
                val active = loaded.createConversation(ConversationConfig(
                    systemInstruction = Contents.of(systemText),
                    automaticToolCalling = false, maxOutputToken = 128,
                    samplerConfig = SamplerConfig(topK = 20, topP = 0.8, temperature = 0.7, seed = 42),
                    thinkingConfig = ThinkingConfig(enableThinking = false, thinkingTokenBudget = 0)))
                val conversationEndNs = SystemClock.elapsedRealtimeNanos()
                conversation = active
                activeLog.offer("conversation_create_end", "", fields = mapOf("call_start_ns" to conversationStartNs.toString(), "call_end_ns" to conversationEndNs.toString(), "call_wall_ns" to (conversationEndNs - conversationStartNs).toString()))
                val generationStart = AtomicLong(0L)
                val firstRaw = AtomicBoolean(false)
                val callback = object : MessageCallback {
                    override fun onMessage(message: Message) {
                        if (firstRaw.compareAndSet(false, true)) activeLog.offer("first_raw_callback_ms", ((SystemClock.elapsedRealtimeNanos() - generationStart.get()) / 1_000_000).toString())
                        activeLog.offer("raw", message.toString(), message.channels)
                    }
                    override fun onDone() { activeLog.offer("native_terminal", "done"); nativeDone.set(true); current.terminal(); terminal.countDown() }
                    override fun onError(throwable: Throwable) {
                        activeLog.offer("native_terminal_error", "${throwable.javaClass.simpleName}: ${throwable.message}")
                        current.terminal(); terminal.countDown()
                    }
                }
                activeLog.offer("send_prepared", "durable marker before guarded submission; cancellation may still refuse send")
                activeLog.checkpoint(::display)
                if (!current.start { active.cancelProcess() } || !current.sendIfActive {
                    activeLog.offer("send_start", "public async send about to be called")
                    val sendStartNs = SystemClock.elapsedRealtimeNanos()
                    generationStart.set(sendStartNs)
                    possibleAccepted = true
                    active.sendMessageAsync("Passage:\n$context\n\nQuestion:\n$question", callback)
                    val sendEndNs = SystemClock.elapsedRealtimeNanos()
                    activeLog.offer("send_return", "async API return is not inference completion", fields = mapOf("call_start_ns" to sendStartNs.toString(), "call_end_ns" to sendEndNs.toString(), "call_wall_ns" to (sendEndNs - sendStartNs).toString()))
                }) { current.terminal(); terminal.countDown() }
                while (!terminal.await(250, TimeUnit.MILLISECONDS)) activeLog.flush(::display)
                current.awaitQuiescentTerminal()
                append("terminal_ms=${(SystemClock.elapsedRealtimeNanos() - requested) / 1_000_000}; cancelled=${current.isCancelled()}")
                if (!current.isCancelled()) {
                    try {
                        val stats = active.getBenchmarkInfo()
                        activeLog.offer("sdk_stats", stats.toString(), fields = mapOf(
                            "init_seconds" to stats.initTimeInSecond.toString(),
                            "ttft_seconds" to stats.timeToFirstTokenInSecond.toString(),
                            "prefill_tokens" to stats.lastPrefillTokenCount.toString(),
                            "decode_tokens" to stats.lastDecodeTokenCount.toString(),
                            "prefill_tokens_per_second" to stats.lastPrefillTokensPerSecond.toString(),
                            "decode_tokens_per_second" to stats.lastDecodeTokensPerSecond.toString(),
                            "effective_max_tokens" to stats.maxNumTokens.toString(),
                            "kv_tokens" to active.getTokenCount().toString()))
                        statsRecorded = true
                        append("SDK_stats=$stats; SDK_kv_token_count=${active.getTokenCount()}")
                    } catch (error: Exception) {
                        append("SDK_stats_missing=${error.javaClass.simpleName}: ${error.message}")
                    }
                } else append("SDK_stats_missing=cancelled conversation is discarded without further inference-state queries")
                append("First-useful-answer timing missing: raw callbacks require content inspection; callback count is not token count. Thought channels require output inspection.")
            } catch (error: Exception) {
                append("Probe refused/failed: ${error.javaClass.simpleName}: ${error.message}")
            } finally {
                if (possibleAccepted && !current.isTerminal()) {
                    append("Uncertain native acceptance; retaining engine/conversation until an actual terminal callback. No forced disposal.")
                    while (!current.isTerminal()) {
                        try { terminal.await(250, TimeUnit.MILLISECONDS) } catch (_: InterruptedException) { }
                        log?.flush(::display)
                    }
                }
                if (!possibleAccepted) current.terminal()
                while (true) {
                    try { current.awaitQuiescentTerminal(); break } catch (_: InterruptedException) { }
                }
                try {
                    current.closeIfTerminal {
                        try { conversation?.close() } finally { if (initialized) engine?.close() }
                    }
                } catch (error: Exception) {
                    disposalSucceeded = false
                    append("Disposal error: ${error.javaClass.simpleName}: ${error.message}")
                } finally {
                    try {
                        log?.finish(nativeDone.get() && !current.isCancelled() && statsRecorded && disposalSucceeded, ::display)
                        log?.close()
                    } catch (error: Exception) { display("Record close error; comparison ineligible: ${error.message}") }
                    records = null
                    lease = null
                    busy.set(false)
                    runOnUiThread { window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
                    if (destroyed) canceller.shutdown()
                }
            }
        }
    }

    override fun onPause() {
        window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        requestCancel()
        super.onPause()
    }

    override fun onDestroy() {
        destroyed = true
        requestCancel()
        worker.shutdown()
        if (!busy.get()) canceller.shutdown()
        super.onDestroy()
    }
}
