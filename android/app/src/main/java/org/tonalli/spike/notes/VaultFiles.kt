package org.tonalli.spike.notes

import java.io.IOException
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.security.MessageDigest

/** Called only with closed native handles, on the notes worker. */
class VaultFiles(directory: Path) {
    val database: Path = directory.resolve("notes.db")
    val keys: Path = directory.resolve("notes.keys")
    data class Fingerprint(val bytes: Long, val sha256: String)
    data class Snapshot(val database: Fingerprint, val keys: Fingerprint)
    class ExistingVault : IOException()

    fun requireNew() {
        if (Files.exists(database, NOFOLLOW_LINKS) || Files.exists(keys, NOFOLLOW_LINKS)) {
            throw ExistingVault()
        }
    }

    fun snapshot(): Snapshot = Snapshot(fingerprint(database), fingerprint(keys))

    private fun fingerprint(path: Path): Fingerprint {
        if (!Files.isRegularFile(path, NOFOLLOW_LINKS)) throw IOException("Missing or nonregular vault file")
        val expected = Files.size(path)
        val hash = MessageDigest.getInstance("SHA-256")
        var total = 0L
        Files.newInputStream(path, NOFOLLOW_LINKS).use { input ->
            val buffer = ByteArray(64 * 1024)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                if (count == 0 || total > expected - count) throw IOException("Vault changed while fingerprinting")
                hash.update(buffer, 0, count)
                total += count
            }
        }
        if (total != expected || Files.size(path) != expected) throw IOException("Vault changed while fingerprinting")
        return Fingerprint(total, hash.digest().joinToString("") { "%02x".format(it) })
    }
}
