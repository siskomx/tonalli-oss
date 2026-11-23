package org.tonalli.spike.reader

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import java.io.File
import java.io.IOException
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicLong
import org.tonalli.spike.R
import org.tonalli.spike.index.*
import org.tonalli.spike.saf.*
import uniffi.tonalli_core.indexOpen
import uniffi.tonalli_core.zimIdentity

/** Native-only references and trusted descriptor authority; no URI or path crosses into JavaScript. */
class ReaderIndexActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private val publication = SafPublication()
    private val generation = AtomicLong()
    @Volatile private var active = false
    @Volatile private var destroyed = false
    private var pendingId: String? = null
    private var pendingHash: String? = null
    private var result: Intent? = null
    private var dialog: AlertDialog? = null
    private lateinit var status: TextView
    private lateinit var choose: Button

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        if (state?.getInt(KIND) == PICK_INDEX) {
            pendingId = state.getString(ID)?.takeIf { Regex("reader-[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}").matches(it) }
            pendingHash = state.getString(HASH)?.takeIf { Regex("[a-f0-9]{64}").matches(it) }
            if (pendingId == null || pendingHash == null) { pendingId = null; pendingHash = null }
        }
        status = TextView(this).apply { setText(R.string.index_setup_help); setPadding(24, 24, 24, 24) }
        choose = Button(this).apply { setText(R.string.index_choose_library); setOnClickListener { chooseLibrary() } }
        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(status); addView(choose)
            addView(Button(this@ReaderIndexActivity).apply { setText(R.string.index_back_library); setOnClickListener { finish() } })
        })
    }

    private fun valid(epoch: Long) = active && !destroyed && generation.get() == epoch
    private fun cancelled(epoch: Long) = destroyed || generation.get() != epoch
    private fun show(epoch: Long, message: Int) = runOnUiThread {
        if (valid(epoch)) { status.setText(message); choose.isEnabled = true }
    }

    private fun chooseLibrary() {
        val epoch = generation.incrementAndGet(); publication.invalidate(); choose.isEnabled = false
        worker.execute {
            try {
                val entries = ReaderCatalogStore(applicationContext).load()
                runOnUiThread {
                    if (!valid(epoch)) return@runOnUiThread
                    choose.isEnabled = true
                    if (entries.isEmpty()) { status.setText(R.string.index_no_libraries); return@runOnUiThread }
                    val labels = entries.map { entry ->
                        entry.selection.documents.firstOrNull()?.name?.take(100)?.takeIf { it.isNotBlank() }
                            ?: getString(R.string.index_unnamed_library)
                    }
                    dialog = AlertDialog.Builder(this).setTitle(R.string.index_choose_library)
                        .setItems(labels.toTypedArray()) { _, which -> if (valid(epoch)) prepare(entries[which], epoch) }
                        .setNegativeButton(android.R.string.cancel, null).show()
                }
            } catch (_: Exception) { show(epoch, R.string.index_failed) }
        }
    }

    private fun profile(entry: ReaderEntry, pack: SafPackSession): IndexTransferSpec? {
        val actual = zimIdentity(pack.handle)
        ReaderCatalogPolicy.requireIdentity(entry, ReaderIdentity(actual.uuid, actual.storedMd5))
        return listOf("small", "full", "medicine").firstNotNullOfOrNull { dataset ->
            val spec = try { PackagedIndexTransfer.load(applicationContext, dataset) } catch (_: Exception) { null }
            spec?.takeIf { ReaderIndexPolicy.matches(it, entry.identity, pack.partSizes) }
        }
    }

    private fun prepare(entry: ReaderEntry, epoch: Long) {
        choose.isEnabled = false; status.setText(R.string.index_checking_library)
        worker.execute {
            try {
                val spec = SafPackSession.open(SafDocuments(contentResolver), entry.selection, { cancelled(epoch) }).use { profile(entry, it) }
                runOnUiThread {
                    if (!valid(epoch)) return@runOnUiThread
                    choose.isEnabled = true
                    if (spec == null) { status.setText(R.string.index_no_profile); return@runOnUiThread }
                    pendingId = entry.id; pendingHash = spec.manifestHash
                    val picker = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
                    try { startActivityForResult(picker, PICK_INDEX) }
                    catch (_: android.content.ActivityNotFoundException) { pendingId = null; pendingHash = null; status.setText(R.string.index_no_picker) }
                }
            } catch (_: Exception) { show(epoch, R.string.index_failed) }
        }
    }

    @Deprecated("Platform SAF result avoids another dependency")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != PICK_INDEX || pendingId == null || pendingHash == null) return
        if (resultCode != RESULT_OK || data?.data == null) {
            pendingId = null; pendingHash = null; status.setText(R.string.index_cancelled); return
        }
        result = data
        if (active) consume()
    }

    private fun consume() {
        val data = result ?: return
        val id = pendingId ?: return
        val hash = pendingHash ?: return
        result = null; pendingId = null; pendingHash = null
        val epoch = generation.get(); val token = publication.begin()
        choose.isEnabled = false; status.setText(R.string.index_installing)
        worker.execute {
            try {
                val store = ReaderCatalogStore(applicationContext)
                val entry = store.load().firstOrNull { it.id == id } ?: throw IOException("Reader was removed")
                val documents = SafDocuments(contentResolver)
                val tree = data.data ?: throw IOException("Missing index folder")
                documents.persist(tree, data.flags)
                SafPackSession.open(documents, entry.selection, { cancelled(epoch) }).use { pack ->
                    val spec = profile(entry, pack)?.takeIf { it.manifestHash == hash } ?: throw IOException("Trusted source profile changed")
                    TrustedIndexInstaller.install(SafIndexInput(contentResolver, tree), File(filesDir, "index-handoff/${spec.dataset}").toPath(), spec,
                        cancelled = { cancelled(epoch) }, withSelection = { commit ->
                            if (!publication.publish(token) { store.withCurrent(entry) {
                                if (cancelled(epoch)) throw IOException("Import cancelled")
                                commit()
                            } }) throw IOException("Import owner departed")
                        }, validate = { staged -> indexOpen(staged.toString(), pack.handle).destroy() })
                }
                runOnUiThread { if (valid(epoch)) { setResult(RESULT_OK); finish() } }
            } catch (_: Exception) { show(epoch, R.string.index_failed) }
        }
    }

    override fun onResume() { super.onResume(); active = true; choose.isEnabled = true; consume() }
    override fun onSaveInstanceState(outState: Bundle) {
        if (pendingId != null && pendingHash != null) {
            outState.putInt(KIND, PICK_INDEX); outState.putString(ID, pendingId); outState.putString(HASH, pendingHash)
        }
        super.onSaveInstanceState(outState)
    }
    override fun onPause() {
        active = false; generation.incrementAndGet(); publication.invalidate(); dialog?.dismiss(); dialog = null
        super.onPause()
    }
    override fun onDestroy() {
        destroyed = true; publication.invalidate(); dialog?.dismiss(); dialog = null; worker.shutdown()
        super.onDestroy()
    }
    companion object {
        private const val PICK_INDEX = 87
        private const val KIND = "reader_index_kind"; private const val ID = "reader_index_id"; private const val HASH = "reader_index_hash"
    }
}
