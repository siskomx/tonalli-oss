package org.tonalli.spike.index

import java.io.IOException
import java.security.MessageDigest
import java.util.Locale

/** A separate profile whose byte limits come from one exact packaged descriptor. */
class IndexTransferSpec private constructor(val dataset: String, val descriptor: ByteArray, val generation: String,
    val source: Source, val entries: List<Entry>, val queries: List<Query>, val payloadBytes: Long,
    val incomingBytes: Long, val requiredBytes: Long) {
    data class Source(val uuid: String, val bytes: Long, val md5: String, val expectedSha256: String)
    data class Entry(val path: String, val bytes: Long, val sha256: String)
    data class Hit(val title: String, val url: String)
    data class Query(val query: String, val hits: List<Hit>)
    val manifestHash: String = MessageDigest.getInstance("SHA-256").digest(descriptor).joinToString("") { "%02x".format(it) }
    val incomingName: String get() = "probe-index-${manifestHash.take(12)}"
    val stageRequiredBytes: Long get() = requiredBytes - incomingBytes
    companion object {
        const val RESERVE_BYTES = 33554432L
        const val MAX_DESCRIPTOR_BYTES = 1024 * 1024
        const val MAX_FILES = 4096
        val QUERY_NAMES = listOf("solar system", "photosynthesis", "Mexican history")
        private val hash = Regex("[a-f0-9]{64}")
        private fun text(value: String, limit: Int) = value.isNotEmpty() &&
            value.toByteArray(Charsets.UTF_8).size <= limit && value.none { it.code < 32 || it.code == 127 }
        fun checked(dataset: String, descriptor: ByteArray, generation: String, source: Source,
            entries: List<Entry>, queries: List<Query>): IndexTransferSpec {
            if (dataset !in setOf("small", "full", "medicine") || descriptor.size !in 1..MAX_DESCRIPTOR_BYTES ||
                !Regex("generation-[A-Za-z0-9_-]{1,64}").matches(generation) || entries.size !in 3..MAX_FILES ||
                entries.map { it.path.lowercase(Locale.ROOT) }.distinct().size != entries.size || entries.none { it.path == "CURRENT" } ||
                !Regex("[a-f0-9]{32}").matches(source.uuid) || !Regex("[a-f0-9]{32}").matches(source.md5) ||
                !hash.matches(source.expectedSha256) || source.bytes <= 0 || queries.map { it.query } != QUERY_NAMES) throw IOException("Invalid trusted index profile")
            val paths = entries.map { it.path.lowercase(Locale.ROOT) }.toSet()
            if (!paths.containsAll(listOf("$generation/meta.json", "$generation/tonalli-meta.json").map { it.lowercase(Locale.ROOT) })) {
                throw IOException("Trusted index metadata files are missing")
            }
            var total = 0L
            for (entry in entries) {
                val name = entry.path.removePrefix("$generation/")
                if (entry.bytes < 0 || !hash.matches(entry.sha256) ||
                    (entry.path == "CURRENT" && entry.bytes !in 1L..256L) ||
                    (entry.path != "CURRENT" && (name == entry.path || !Regex("[A-Za-z0-9._-]{1,160}").matches(name) ||
                        name.endsWith('.') || name in setOf(".", "..", ".tantivy-meta.lock", ".tantivy-writer.lock")))) throw IOException("Invalid trusted index file")
                total = add(total, entry.bytes)
            }
            for (query in queries) if (query.hits.size > 5 || query.hits.any {
                !text(it.title, 8192) || !text(it.url, 4096) || it.url.startsWith('/') || "://" in it.url
            }) throw IOException("Invalid trusted ordered query baseline")
            val incoming = add(total, descriptor.size.toLong())
            return IndexTransferSpec(dataset, descriptor.copyOf(), generation, source, entries.toList(), queries.toList(),
                total, incoming, add(add(incoming, total), RESERVE_BYTES))
        }
        private fun add(a: Long, b: Long): Long = try { Math.addExact(a, b) }
            catch (error: ArithmeticException) { throw IOException("Trusted index byte total overflow", error) }
        fun requireHits(query: Query, actual: List<Hit>) {
            if (actual != query.hits) throw IOException("Ordered index title/URL parity differs from trusted baseline")
        }
    }
}
