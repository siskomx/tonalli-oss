package org.tonalli.spike.saf

import java.io.IOException
import java.util.UUID

private fun rejects(work: () -> Unit) { try { work(); error("Expected invalid binding rejection") } catch (_: IOException) { } }

fun checkSafBenchmark() {
    val selection = SafSelection(SafKind.FILES, listOf(SafDocument("content://provider/selected", "Niñez.zim")))
    val fingerprint = SafBenchmarkContract.fingerprint(selection)
    check(Regex("[a-f0-9]{64}").matches(fingerprint))
    check(fingerprint == SafBenchmarkContract.fingerprint(selection.copy()))
    check(fingerprint != SafBenchmarkContract.fingerprint(selection.copy(documents = listOf(SafDocument("content://provider/changed", "Niñez.zim")))))
    val revision = UUID.randomUUID().toString()
    SafBenchmarkContract.requireSelection(selection, revision, fingerprint, revision, fingerprint)
    rejects { SafBenchmarkContract.requireSelection(selection, revision, fingerprint, UUID.randomUUID().toString(), fingerprint) }
    rejects { SafBenchmarkContract.requireSelection(selection, revision, fingerprint, revision, "0".repeat(64)) }
    val iso = runCatching { SafBenchmarkContract.requireSelection(selection.copy(kind = SafKind.ISO, innerPath = "/pack.zim"), revision, fingerprint, revision, fingerprint) }.exceptionOrNull()
    check(iso is SafBenchmarkRefusal && iso.reason == SafBenchmarkRefusal.Reason.ISO_SELECTION)
    check(SafBenchmarkErrors.message(iso) == iso.userMessage && "ISO" in iso.userMessage)
    val stale = runCatching { SafBenchmarkContract.requireSelection(selection, revision, fingerprint, UUID.randomUUID().toString(), fingerprint) }.exceptionOrNull()
    check(stale is SafBenchmarkRefusal && stale.reason == SafBenchmarkRefusal.Reason.SELECTION_CHANGED)
    check(SafBenchmarkErrors.message(stale) == stale.userMessage && "receipt" in stale.userMessage)
    val provider = SafBenchmarkErrors.message(IOException("content://private-provider/operator-file"))
    check(provider == "Benchmark/preparation failed: IOException; no success is claimed." && "private-provider" !in provider)
    SafBenchmarkContract.requireLengths(listOf(4L, 6L), listOf(4L, 6L), 10L)
    SafBenchmarkContract.requireLengths(listOf(null, 6L), listOf(4L, 6L), 10L)
    rejects { SafBenchmarkContract.requireLengths(listOf(4L, 6L), listOf(5L, 5L), 10L) }
    rejects { SafBenchmarkContract.requireLengths(listOf(null, null), listOf(4L, 5L), 10L) }
    rejects { SafBenchmarkContract.requireLengths(listOf(4L), listOf(4L, 6L), 10L) }
    println("SAF benchmark identity/reselection/ISO refusal/observable length binding checks passed")
}
