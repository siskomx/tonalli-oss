package org.tonalli.spike.notes

import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import java.security.MessageDigest
import org.tonalli.spike.web.WorkerSession

private fun rejects(block: () -> Unit) {
    try { block() } catch (_: IOException) { return }
    error("Expected IOException")
}

fun main() {
    val gate = NotesActionGate()
    gate.resume()
    val queuedCreate = gate.begin()
    gate.pause()
    var creates = 0
    if (gate.permits(queuedCreate)) creates++
    gate.resume()
    check(!gate.permits(queuedCreate) && creates == 0)
    val current = gate.begin()
    check(gate.permits(current))
    gate.begin()
    check(!gate.permits(current))
    val directory = Files.createTempDirectory("tonalli-notes-test-")
    try {
        val vault = VaultFiles(directory)
        vault.requireNew()
        check(!Files.exists(vault.database) && !Files.exists(vault.keys))
        Files.write(vault.database, "abc".toByteArray())
        rejects { vault.requireNew() }
        check(Files.readAllBytes(vault.database).contentEquals("abc".toByteArray()))
        Files.delete(vault.database)
        Files.write(vault.keys, "keys".toByteArray())
        rejects { vault.requireNew() }
        Files.write(vault.database, "abc".toByteArray())
        rejects { vault.requireNew() }
        val before = vault.snapshot()
        check(before.database.sha256 == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        check(before.database.bytes == 3L && before == vault.snapshot())
        Files.write(vault.database, "abd".toByteArray())
        check(before != vault.snapshot())
        val multiBuffer = ByteArray(128 * 1024 + 7) { (it % 251).toByte() }
        Files.write(vault.database, multiBuffer)
        val expected = MessageDigest.getInstance("SHA-256").digest(multiBuffer).joinToString("") { "%02x".format(it) }
        check(vault.snapshot().database.sha256 == expected)
        Files.delete(vault.keys)
        rejects { vault.snapshot() }
    } finally {
        check(directory.toRealPath().parent == Path.of(System.getProperty("java.io.tmpdir")).toRealPath())
        check(directory.fileName.toString().startsWith("tonalli-notes-test-"))
        Files.walk(directory).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach(Files::delete) }
    }
    var closes = 0
    val lifetime = WorkerSession<AutoCloseable>()
    check(lifetime.current() == null)
    check(lifetime.adopt(AutoCloseable { closes++ }))
    check(lifetime.current() != null)
    lifetime.release()
    check(lifetime.current() == null && closes == 1)
    check(lifetime.adopt(AutoCloseable { closes++ }))
    lifetime.requestClose()
    lifetime.release()
    check(!lifetime.adopt(AutoCloseable { closes++ }))
    lifetime.release()
    check(closes == 3)
    println("PASS: queued-action/stale-result cancellation, create refuses either existing file without mutation, known/multibuffer SHA fingerprints, same-length change/missing-key detection, lock/reopen/final-close/late-adoption lifecycle")
}
