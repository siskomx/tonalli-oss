package org.tonalli.spike.reader

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import java.io.IOException
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicLong
import org.tonalli.spike.saf.*
import org.tonalli.spike.R
import uniffi.tonalli_core.zimVerifyIdentity

/** Native picker owns provider identities; no content URI is exposed to the WebView. */
class OpenContentActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private val publication = SafPublication()
    private val generation = AtomicLong()
    @Volatile private var active = false
    @Volatile private var destroyed = false
    private var pendingRequest: Int? = null
    private var pending: Pair<Int, Intent>? = null
    private var groupDialog: AlertDialog? = null
    private lateinit var status: TextView

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        pendingRequest = state?.getInt(PENDING_REQUEST)?.let(ReaderFolderPolicy::restoreRequest)
        status = TextView(this).apply { setText(R.string.reader_choose_help); setPadding(24, 24, 24, 24) }
        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(status)
            addView(Button(this@OpenContentActivity).apply { setText(R.string.reader_choose_files); setOnClickListener { pick(FILES) } })
            addView(Button(this@OpenContentActivity).apply { setText(R.string.reader_choose_folder); setOnClickListener { pick(TREE) } })
            addView(Button(this@OpenContentActivity).apply { setText(R.string.reader_back_library); setOnClickListener { finish() } })
        })
    }

    private fun pick(request: Int) {
        publication.invalidate()
        generation.incrementAndGet()
        pendingRequest = request
        val picker = if (request == TREE) Intent(Intent.ACTION_OPEN_DOCUMENT_TREE) else Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE); type = "*/*"
            putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
        }
        picker.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        try { startActivityForResult(picker, request) }
        catch (_: android.content.ActivityNotFoundException) { pendingRequest = null; status.setText(R.string.reader_no_picker) }
    }

    @Deprecated("Platform picker callback avoids another dependency")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (!ReaderFolderPolicy.acceptsResult(pendingRequest, requestCode)) return
        pendingRequest = null
        if (resultCode != RESULT_OK || data == null) { status.setText(R.string.reader_cancelled); return }
        pending = requestCode to data
        if (active) consume()
    }

    private fun consume() {
        val (request, result) = pending ?: return
        pending = null
        val epoch = generation.get()
        status.setText(R.string.reader_reading)
        worker.execute {
            try {
                val documents = SafDocuments(contentResolver)
                if (request == TREE) {
                    val tree = result.data ?: throw IOException("No folder was selected")
                    documents.persist(tree, result.flags)
                    val groups = ReaderFolderPolicy.groups(documents.children(tree))
                    if (groups.isEmpty()) throw IOException("No immediate ZIM group found")
                    runOnUiThread {
                        if (!valid(epoch)) return@runOnUiThread
                        groupDialog?.dismiss()
                        // Provider names are plain, bounded labels, never commands or markup.
                        groupDialog = AlertDialog.Builder(this).setTitle(R.string.reader_choose_library)
                            .setItems(groups.map { it.first().name.take(100) + if (it.size > 1) getString(R.string.reader_part_count, it.size) else "" }.toTypedArray()) { _, which ->
                                if (valid(epoch)) verify(SafSelection(SafKind.TREE, groups[which], treeUri = tree.toString()))
                            }.setNegativeButton(android.R.string.cancel) { _, _ ->
                                if (valid(epoch)) status.setText(R.string.reader_cancelled)
                            }.show()
                    }
                    return@execute
                }
                val uris = result.clipData?.let { clip ->
                    if (clip.itemCount !in 1..128) throw IOException("Select all parts of one pack, at most 128 files")
                    (0 until clip.itemCount).map { clip.getItemAt(it).uri }
                } ?: listOf(result.data ?: throw IOException("No document was selected"))
                uris.forEach { documents.persist(it, result.flags) }
                val selection = SafSelections.checked(SafSelection(SafKind.FILES, uris.map { documents.document(it) }))
                runOnUiThread { if (valid(epoch)) verify(selection) }
            } catch (_: Exception) { showFailure(epoch) }
        }
    }

    private fun valid(epoch: Long) = active && !destroyed && generation.get() == epoch

    private fun showFailure(epoch: Long) { runOnUiThread {
        if (valid(epoch)) status.setText(R.string.reader_file_failed)
    } }

    private fun verify(selection: SafSelection) {
        val token = publication.begin()
        val epoch = generation.get()
        status.setText(R.string.reader_checking_file)
        worker.execute {
            try {
                val documents = SafDocuments(contentResolver)
                val verified = SafPackSession.open(documents, selection, cancelled = { destroyed || generation.get() != epoch }).use {
                    // Core verifies through positioned reads; Kotlin never implements a checksum engine.
                    zimVerifyIdentity(it.handle).let { identity -> ReaderIdentity(identity.uuid, identity.storedMd5) }
                }
                runOnUiThread {
                    if (!valid(epoch)) return@runOnUiThread
                    try {
                        if (publication.publish(token) { ReaderCatalogStore(applicationContext).add(selection, verified) }) {
                            setResult(RESULT_OK)
                            finish()
                        }
                    } catch (_: Exception) { status.setText(R.string.reader_save_failed) }
                }
            } catch (_: Exception) { showFailure(epoch) }
        }
    }

    override fun onResume() { super.onResume(); active = true; consume() }
    override fun onSaveInstanceState(outState: Bundle) {
        pendingRequest?.let { outState.putInt(PENDING_REQUEST, it) }
        super.onSaveInstanceState(outState)
    }
    override fun onPause() {
        active = false; generation.incrementAndGet(); publication.invalidate()
        groupDialog?.dismiss(); groupDialog = null
        super.onPause()
    }
    override fun onDestroy() {
        destroyed = true; publication.invalidate(); groupDialog?.dismiss(); groupDialog = null
        worker.shutdown(); super.onDestroy()
    }
    companion object {
        private const val FILES = ReaderFolderPolicy.FILES; private const val TREE = ReaderFolderPolicy.TREE
        private const val PENDING_REQUEST = "reader_pending_picker"
    }
}
