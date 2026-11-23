package org.tonalli.spike.notes

import android.app.Activity
import android.os.Bundle
import android.text.InputType
import android.text.method.PasswordTransformationMethod
import android.util.Log
import android.view.View
import android.view.WindowManager
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import java.io.File
import java.util.concurrent.Executors
import org.tonalli.spike.web.WorkerSession
import uniffi.tonalli_core.NotesHandle

/** Secure disposable native gate controls; product notes UI belongs to the shared SPA. */
class NotesProbeActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private val lifetime = WorkerSession<NotesHandle>()
    private lateinit var probe: NotesProbe
    private lateinit var passphrase: EditText
    private lateinit var recovery: EditText
    private lateinit var oneTimeKey: TextView
    private lateinit var status: TextView
    private val actions = mutableListOf<Button>()
    private val gate = NotesActionGate()

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        passphrase = secretInput("Passphrase (verbatim; spaces are significant)")
        recovery = secretInput("Recovery key (separate from passphrase)")
        oneTimeKey = TextView(this).apply {
            isSaveEnabled = false
            isSaveFromParentEnabled = false
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
            importantForContentCapture = View.IMPORTANT_FOR_CONTENT_CAPTURE_NO_EXCLUDE_DESCENDANTS
            setTextIsSelectable(true)
        }
        status = TextView(this).apply { isSaveEnabled = false; setTextIsSelectable(true) }
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(24, 16, 24, 16)
            isSaveFromParentEnabled = false
            addView(TextView(this@NotesProbeActivity).apply {
                text = "Encrypted notes M1 probe. Create never replaces files. Record the one-time recovery key privately before any other action or leaving this screen. Only synthetic notes are added."
            })
            addView(passphrase)
            addView(recovery)
            addView(action("Create new vault only", Credential.PASSPHRASE) { create(it) })
            addView(action("Open with passphrase", Credential.PASSPHRASE) { openPassphrase(it) })
            addView(action("Open with recovery", Credential.RECOVERY) { openRecovery(it) })
            addView(action("Add and verify synthetic sentinel") { addSentinel() })
            addView(action("Read synthetic sentinel") { readSentinel() })
            addView(action("Close vault") { closeVault() })
            addView(action("Check wrong passphrase (closes first)", Credential.PASSPHRASE) { wrongPassphrase(it) })
            addView(Button(this@NotesProbeActivity).apply { text = "Close notes probe"; setOnClickListener { finish() } })
            addView(oneTimeKey)
            addView(status)
        }
        setContentView(ScrollView(this).apply { addView(column) })
        worker.execute { probe = NotesProbe(File(applicationContext.filesDir, "core").toPath(), lifetime) }
    }

    private enum class Credential { NONE, PASSPHRASE, RECOVERY }

    private fun secretInput(label: String) = EditText(this).apply {
        hint = label
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        imeOptions = EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
        isSingleLine = true
        transformationMethod = PasswordTransformationMethod.getInstance()
        isSaveEnabled = false
        isSaveFromParentEnabled = false
        importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
        importantForContentCapture = View.IMPORTANT_FOR_CONTENT_CAPTURE_NO_EXCLUDE_DESCENDANTS
    }

    private fun action(label: String, credential: Credential = Credential.NONE, work: NotesProbe.(String) -> NotesProbe.Result): Button =
        Button(this).apply {
            text = label
            setOnClickListener {
                val secret = when (credential) {
                    Credential.NONE -> ""
                    Credential.PASSPHRASE -> passphrase.text.toString()
                    Credential.RECOVERY -> recovery.text.toString()
                }
                clearSecrets()
                if (credential != Credential.NONE && secret.isEmpty()) {
                    status.text = "Enter a nonempty credential; spaces in nonempty input are preserved verbatim."
                    return@setOnClickListener
                }
                val stamp = gate.begin()
                actions.forEach { it.isEnabled = false }
                status.text = "Working on encrypted vault…"
                worker.execute {
                    if (!gate.permits(stamp) || lifetime.isClosing) return@execute
                    val result = runCatching { probe.work(secret) }
                        .getOrElse { NotesProbe.Result(NotesProbe.sanitizedFailure(it)) }
                    // Only controlled status text is logged; never stringify credentials or results.
                    Log.i("TonalliM1", "Notes probe: ${result.message}")
                    runOnUiThread {
                        if (gate.permits(stamp) && !lifetime.isClosing) {
                            status.text = result.message
                            oneTimeKey.text = result.recoveryKey ?: ""
                            actions.forEach { it.isEnabled = true }
                        }
                    }
                }
            }
            actions.add(this)
        }

    private fun clearSecrets() {
        passphrase.text?.clear()
        recovery.text?.clear()
        oneTimeKey.text = ""
    }

    override fun onResume() {
        super.onResume()
        gate.resume()
        status.text = "Explicit unlock required after pauses. Enter a credential for an action; device gates remain pending."
        actions.forEach { it.isEnabled = true }
    }

    override fun onPause() {
        gate.pause()
        clearSecrets()
        if (!lifetime.isClosing) worker.execute { if (::probe.isInitialized) runCatching { probe.closeVault() } }
        super.onPause()
    }

    override fun onDestroy() {
        gate.pause()
        clearSecrets()
        lifetime.requestClose()
        worker.execute { runCatching { lifetime.release() } }
        worker.shutdown()
        super.onDestroy()
    }
}
