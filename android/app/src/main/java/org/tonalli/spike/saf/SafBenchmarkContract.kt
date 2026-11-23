package org.tonalli.spike.saf

import java.io.ByteArrayOutputStream
import java.io.DataOutputStream
import java.io.IOException
import java.security.MessageDigest

/** Identity metadata is not a checksum of the provider's current content. */
object SafBenchmarkContract {
    fun fingerprint(selection: SafSelection): String {
        SafSelections.checked(selection)
        val bytes = ByteArrayOutputStream()
        DataOutputStream(bytes).use { output ->
            fun field(value: String) { val encoded = value.toByteArray(Charsets.UTF_8); output.writeInt(encoded.size); output.write(encoded) }
            field(selection.kind.name); field(selection.treeUri.orEmpty()); field(selection.innerPath.orEmpty())
            output.writeInt(selection.documents.size)
            selection.documents.forEach { field(it.uri); field(it.name) }
        }
        return MessageDigest.getInstance("SHA-256").digest(bytes.toByteArray()).joinToString("") { "%02x".format(it) }
    }

    fun requireSelection(selection: SafSelection, currentRevision: String?, currentFingerprint: String,
        expectedRevision: String, expectedFingerprint: String) {
        if (selection.kind == SafKind.ISO) throw SafBenchmarkRefusal(SafBenchmarkRefusal.Reason.ISO_SELECTION)
        if (!Regex("[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}").matches(expectedRevision) ||
            !Regex("[a-f0-9]{64}").matches(expectedFingerprint) || currentRevision != expectedRevision ||
            currentFingerprint != expectedFingerprint || fingerprint(selection) != currentFingerprint) {
            throw SafBenchmarkRefusal(SafBenchmarkRefusal.Reason.SELECTION_CHANGED)
        }
    }

    fun requireLengths(bound: List<Long?>, current: List<Long?>, expectedBytes: Long) {
        if (bound.size !in 1..128 || current.size != bound.size || expectedBytes <= 0 ||
            current.any { it != null && (it < 0 || it > expectedBytes) } ||
            bound.indices.any { bound[it] != null && bound[it] != current[it] }) throw IOException("Observable SAF part metadata changed")
        if (current.all { it != null } && current.sumOf { it ?: 0 } != expectedBytes) throw IOException("Observed logical ZIM length differs from expected dataset")
    }
}
