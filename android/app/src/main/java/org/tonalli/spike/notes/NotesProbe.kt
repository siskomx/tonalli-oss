package org.tonalli.spike.notes

import java.io.IOException
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import org.tonalli.spike.web.WorkerSession
import uniffi.tonalli_core.NotesException
import uniffi.tonalli_core.NotesHandle
import uniffi.tonalli_core.notesAdd
import uniffi.tonalli_core.notesCreate
import uniffi.tonalli_core.notesGet
import uniffi.tonalli_core.notesOpenPassphrase
import uniffi.tonalli_core.notesOpenRecovery
import uniffi.tonalli_core.notesSearch

/** Disposable native test operations; every call runs on one worker. */
class NotesProbe(private val directory: Path, private val lifetime: WorkerSession<NotesHandle>) {
    private val files = VaultFiles(directory)
    // Deliberately not a data class: default stringification must not reveal the key.
    class Result(val message: String, val recoveryKey: String? = null)

    fun create(passphrase: String): Result {
        closeVault()
        Files.createDirectories(directory)
        if (!Files.isDirectory(directory, NOFOLLOW_LINKS)) throw IOException("Invalid private vault directory")
        files.requireNew()
        val key = notesCreate(files.database.toString(), passphrase)
        return Result("Created encrypted vault; it is closed. Record the one-time recovery key privately before leaving.", key)
    }

    fun openPassphrase(passphrase: String): Result {
        closeVault()
        if (!lifetime.adopt(notesOpenPassphrase(files.database.toString(), passphrase))) return Result("Cancelled; vault closed.")
        return Result("Vault unlocked with the supplied passphrase. No note content is logged.")
    }

    fun openRecovery(recovery: String): Result {
        closeVault()
        if (!lifetime.adopt(notesOpenRecovery(files.database.toString(), recovery))) return Result("Cancelled; vault closed.")
        return Result("Vault unlocked with the supplied recovery key. Use Read sentinel to verify persistence.")
    }

    fun addSentinel(): Result {
        val handle = unlocked()
        val existing = notesSearch(handle, SENTINEL_BODY).firstOrNull { it.title == SENTINEL_TITLE && it.body == SENTINEL_BODY }
        val id = existing?.id ?: notesAdd(handle, SENTINEL_TITLE, SENTINEL_BODY)
        val note = notesGet(handle, id)
        if (note.title != SENTINEL_TITLE || note.body != SENTINEL_BODY) return Result("FAILURE: synthetic sentinel did not round-trip.")
        return Result(if (existing == null) "Synthetic sentinel added and verified." else "Existing synthetic sentinel verified; no duplicate added.")
    }

    fun readSentinel(): Result {
        val handle = unlocked()
        val found = notesSearch(handle, SENTINEL_BODY).firstOrNull { it.title == SENTINEL_TITLE && it.body == SENTINEL_BODY }
            ?: return Result("FAILURE: synthetic sentinel is absent; no persistence result established.")
        val note = notesGet(handle, found.id)
        return Result(if (note.title == SENTINEL_TITLE && note.body == SENTINEL_BODY)
            "Synthetic sentinel verified after unlock." else "FAILURE: synthetic sentinel differs.")
    }

    fun wrongPassphrase(passphrase: String): Result {
        closeVault()
        val before = files.snapshot()
        var failure: Throwable? = null
        try { notesOpenPassphrase(files.database.toString(), passphrase).destroy() }
        catch (error: Throwable) { failure = error }
        val after = files.snapshot()
        if (before != after) return Result("FAILURE: vault fingerprints changed during wrong-passphrase check; preserve files for review.")
        val hashes = "DB SHA256 ${after.database.sha256} (${after.database.bytes} bytes)\n" +
            "Keys SHA256 ${after.keys.sha256} (${after.keys.bytes} bytes)"
        return when (failure) {
            is NotesException.UnlockFailed -> Result("PASS: specifically UnlockFailed; both closed-vault fingerprints unchanged.\n$hashes")
            null -> Result("FAILURE: that credential opened the vault; it was closed again. Wrong-passphrase behavior is unproven.\n$hashes")
            else -> throw failure
        }
    }

    fun closeVault(): Result { lifetime.release(); return Result("Vault closed.") }

    private fun unlocked() = lifetime.current() ?: throw VaultLocked()
    class VaultLocked : Exception()

    companion object {
        const val SENTINEL_TITLE = "Tonalli M1 synthetic sentinel"
        const val SENTINEL_BODY = "TONALLI_M1_NOTES_SENTINEL_20261111"

        fun sanitizedFailure(error: Throwable): String = when (error) {
            is VaultLocked -> "Vault is locked; unlock it first."
            is VaultFiles.ExistingVault -> "Create refused: a database or key sidecar already exists; nothing was reset."
            is NotesException.UnlockFailed -> "Unlock failed; no reset or plaintext fallback was attempted."
            is NotesException.AlreadyExists -> "Create refused: vault artifacts already exist."
            is NotesException.MissingKeys -> "Key sidecar is missing; use the separately recorded recovery key."
            is NotesException.IncompleteCreate -> "Creation was incomplete; encrypted artifacts were retained. Do not reset or delete them."
            is NotesException.Corrupt -> "Vault or recovery input is invalid; files were not reset."
            is IOException -> "Vault I/O or precondition check failed; no reset was attempted."
            else -> "Notes operation failed; no reset or plaintext fallback was attempted."
        }
    }
}
