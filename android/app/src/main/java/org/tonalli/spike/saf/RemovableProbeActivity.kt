package org.tonalli.spike.saf

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import java.io.IOException
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import uniffi.tonalli_core.PackException

/** Disposable read-only SAF probe, independent from the shared product UI. */
class RemovableProbeActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private val generation = AtomicLong()
    private val active = AtomicBoolean(false)
    private val destroyed = AtomicBoolean(false)
    private val publication = SafPublication()
    private lateinit var documents: SafDocuments
    private lateinit var store: SafSelectionStore
    private lateinit var status: TextView
    private lateinit var inner: EditText
    private lateinit var prefix: EditText
    private lateinit var article: EditText
    private var session: SafPackSession? = null // worker only
    private var pendingIsoInnerPath: String? = null
    private var pendingPicker: Pair<Int, Intent>? = null
    private var groupDialog: AlertDialog? = null

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        documents = SafDocuments(contentResolver)
        store = SafSelectionStore(applicationContext)
        pendingIsoInnerPath = state?.getString(PENDING_ISO_INNER)?.takeIf {
            it.isNotEmpty() && it.toByteArray(Charsets.UTF_8).size <= 4096 && !it.any(Char::isISOControl)
        }
        inner = input("ISO inner path (explicit, case-sensitive)", pendingIsoInnerPath.orEmpty())
        prefix = input("Raw title prefix (case-sensitive)", "Trefoil")
        article = input("Article URL inside selected ZIM", "C/Trefoil_knot")
        status = TextView(this).apply { setTextIsSelectable(true) }
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL; setPadding(20, 12, 20, 12)
            addView(TextView(this@RemovableProbeActivity).apply {
                text = "Read-only SAF probe. Select a ZIM, all split parts, or one ISO. Folder selection lists immediate children only. Persisted selection does not auto-open. Reopen obtains fresh descriptors. A provider URI alone does not prove physical SD storage."
            })
            addView(button("Select ZIM file or split parts") { pick(FILES) })
            addView(button("Select folder; choose one pack group") { pick(TREE) })
            addView(inner)
            addView(button("Select one ISO (uses explicit inner path)") { pick(ISO) })
            addView(button("Reopen saved selection with fresh descriptors") {
                submit { epoch ->
                    closeSession()
                    val selected = store.load() ?: throw IOException("No saved selection; use a picker first")
                    val opened = SafPackSession.open(documents, selected) { !valid(epoch) }
                    if (!valid(epoch)) { opened.close(); return@submit }
                    session = opened
                    show(epoch, "Opened ${selected.documents.size} document(s) through provider positioned reads. Ready for title/article probes.")
                }
            })
            addView(prefix)
            addView(button("Search titles") {
                val query = bounded(prefix.text.toString(), "Title prefix") ?: return@button
                submit { epoch ->
                    val opened = requireSession()
                    val hits = opened.suggest(query)
                    show(epoch, "${hits.size} title hit(s)\n" + hits.joinToString("\n") { "${it.title} — ${it.url}" })
                }
            })
            addView(article)
            addView(button("Read explicit article") {
                val url = bounded(article.text.toString(), "Article URL") ?: return@button
                submit { epoch -> show(epoch, requireSession().article(url)) }
            })
            addView(button("Close selected source") {
                generation.incrementAndGet()
                worker.execute { closeQuietly() }
                status.text = "Close queued. Explicit reopen is required before more reads."
            })
            addView(button("Close SAF probe") { finish() })
            addView(status)
        }
        setContentView(ScrollView(this).apply { addView(column) })
        status.text = "Choose a source or explicitly reopen the privately saved selection."
    }

    private fun input(hintText: String, value: String) = EditText(this).apply {
        hint = hintText; setSingleLine(); setText(value); isSaveEnabled = false
        filters = arrayOf(android.text.InputFilter.LengthFilter(4096))
    }
    private fun button(label: String, action: () -> Unit) = Button(this).apply { text = label; setOnClickListener { action() } }
    private fun bounded(value: String, name: String): String? {
        if (value.isEmpty() || value.toByteArray(Charsets.UTF_8).size > 4096 || value.any { it < ' ' }) {
            status.text = "$name must contain 1–4096 UTF-8 bytes without control characters."; return null
        }
        return value
    }

    private fun pick(request: Int) {
        val explicitInner = inner.text.toString()
        if (request == ISO && bounded(explicitInner, "ISO inner path") == null) return
        // Public request input only: a tree picker may also return an ISO group.
        pendingIsoInnerPath = explicitInner.takeIf {
            it.isNotEmpty() && it.toByteArray(Charsets.UTF_8).size <= 4096 && !it.any(Char::isISOControl)
        }
        generation.incrementAndGet()
        worker.execute { closeQuietly() }
        val intent = if (request == TREE) Intent(Intent.ACTION_OPEN_DOCUMENT_TREE) else Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE); type = "*/*"; putExtra(Intent.EXTRA_ALLOW_MULTIPLE, request == FILES)
        }
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        try { startActivityForResult(intent, request) }
        catch (_: android.content.ActivityNotFoundException) {
            pendingIsoInnerPath = null
            status.text = "No system document picker is available."
        }
    }

    @Deprecated("Platform picker callback retained to avoid a new AndroidX dependency")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode in setOf(FILES, TREE, ISO) && resultCode == RESULT_OK && data != null) {
            pendingPicker = requestCode to data
            if (active.get()) consumePicker()
        } else if (requestCode in setOf(FILES, TREE, ISO)) {
            pendingIsoInnerPath = null
        }
    }

    private fun consumePicker() {
        val (request, result) = pendingPicker ?: return
        pendingPicker = null
        val publicationEpoch = publication.begin()
        // The picker may recreate this Activity; never substitute a default path.
        val explicitInner = pendingIsoInnerPath
        pendingIsoInnerPath = null
        val path = if (request == ISO) bounded(explicitInner.orEmpty(), "ISO inner path") ?: return else null
        submit { epoch ->
            closeSession()
            if (request == TREE) {
                val tree = result.data ?: throw IOException("Picker returned no folder")
                documents.persist(tree, result.flags)
                val groups = SafSelections.groups(documents.children(tree))
                if (groups.isEmpty()) throw IOException("No immediate ZIM or ISO pack group found")
                runOnUiThread {
                    if (valid(epoch)) {
                        groupDialog?.dismiss()
                        groupDialog = AlertDialog.Builder(this).setTitle("Choose one immediate pack group")
                        .setItems(groups.map { it.first().name + if (it.size > 1) " (${it.size} parts)" else "" }.toTypedArray()) { _, which ->
                            if (!valid(epoch)) return@setItems
                            val selected = groups[which]
                            val iso = selected.size == 1 && selected[0].name.endsWith(".iso", true)
                            val innerPath = if (iso) bounded(explicitInner.orEmpty(), "ISO inner path") ?: return@setItems else null
                            saveSelection(SafSelection(if (iso) SafKind.ISO else SafKind.TREE, selected, tree.toString(), innerPath))
                        }.setNegativeButton("Cancel", null).show()
                    }
                }
            } else {
                val uris = result.clipData?.let { clip ->
                    if (clip.itemCount > 128) throw IOException("Select at most 128 split parts")
                    (0 until clip.itemCount).map { clip.getItemAt(it).uri }
                } ?: listOf(result.data ?: throw IOException("Picker returned no document"))
                uris.forEach { documents.persist(it, result.flags) }
                val selected = SafSelection(if (request == ISO) SafKind.ISO else SafKind.FILES,
                    uris.map { documents.document(it) }, innerPath = path)
                publishSelection(epoch, publicationEpoch, selected)
            }
        }
    }

    private fun saveSelection(selection: SafSelection) {
        val publicationEpoch = publication.begin()
        submit { epoch -> closeSession(); publishSelection(epoch, publicationEpoch, selection) }
    }
    private fun publishSelection(epoch: Long, publicationEpoch: Long, selection: SafSelection) {
        // Publication and pause run on the same looper: an old worker cannot commit after pause.
        runOnUiThread {
            if (!valid(epoch)) return@runOnUiThread
            try {
                if (publication.publish(publicationEpoch) { store.save(selection) }) {
                    status.text = "Selection saved privately. Tap Reopen to obtain fresh descriptors."
                }
            } catch (_: Throwable) { status.text = "Selection could not be saved; the previous selection is preserved." }
        }
    }
    private fun requireSession(): SafPackSession = session ?: throw IOException("Source unavailable or closed; explicitly reopen saved selection")
    private fun valid(epoch: Long) = !destroyed.get() && active.get() && generation.get() == epoch
    private fun show(epoch: Long, message: String) { runOnUiThread { if (valid(epoch)) status.text = message } }
    private fun submit(work: (Long) -> Unit) {
        val epoch = generation.get()
        worker.execute {
            if (!valid(epoch)) return@execute
            try { work(epoch) }
            catch (_: PackException.MediaRemoved) {
                val failedEpoch = SafReadFence.removedEpoch(generation, epoch)
                closeQuietly()
                if (failedEpoch != null) show(failedEpoch,
                    "Media removed. Source unavailable; reinsert media and explicitly reopen. Old queued reads were cancelled.")
            } catch (error: Throwable) {
                val message = when (error) {
                    is SecurityException -> "Read grant missing or revoked; select again."
                    is PackException.Mismatch -> SafErrorText.missingSplitPart(error.v1)
                        ?: "Core read failed: Mismatch. Check the selected file/group and reopen."
                    is PackException -> "Core read failed: ${error.javaClass.simpleName}. Check the selected file/group and reopen."
                    is IOException -> error.message ?: "Provider read failed; reinsert or select again."
                    else -> "SAF operation failed: ${error.javaClass.simpleName}."
                }
                show(epoch, message)
            }
        }
    }
    private fun closeSession() { val owned = session; session = null; owned?.close() }
    private fun closeQuietly() {
        runCatching { closeSession() }.onFailure {
            android.util.Log.e("TonalliM1", "SAF source cleanup failed: ${it.javaClass.simpleName}")
        }
    }
    override fun onSaveInstanceState(outState: Bundle) {
        pendingIsoInnerPath?.let { outState.putString(PENDING_ISO_INNER, it) }
        super.onSaveInstanceState(outState)
    }
    override fun onResume() { super.onResume(); active.set(true); consumePicker() }
    override fun onPause() {
        active.set(false); generation.incrementAndGet(); publication.invalidate()
        groupDialog?.dismiss(); groupDialog = null
        worker.execute { closeQuietly() }; super.onPause()
    }
    override fun onDestroy() {
        destroyed.set(true); generation.incrementAndGet(); publication.invalidate()
        groupDialog?.dismiss(); groupDialog = null
        worker.shutdown(); super.onDestroy()
    }
    companion object {
        private const val FILES = 71; private const val TREE = 72; private const val ISO = 73
        private const val PENDING_ISO_INNER = "pending_iso_inner"
    }
}
