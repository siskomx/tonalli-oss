package org.tonalli.spike.index

import java.io.IOException
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardOpenOption.READ
import java.security.MessageDigest

/** Shared importer contract for path and SAF inputs; authored bytes only. Not yet executed. */
fun trustedIndexInputTests() {
    val root = Files.createTempDirectory("positioned-index-input-")
    try {
        val generation = "generation-input"
        val incoming = Files.createDirectories(root.resolve("source/$generation")).parent
        val descriptor = "packaged descriptor".toByteArray()
        Files.write(incoming.resolve("snapshot.json"), descriptor)
        val payloads = mapOf("CURRENT" to "$generation\n", "$generation/meta.json" to "{}",
            "$generation/tonalli-meta.json" to "{}", "$generation/segment.store" to "authored index bytes")
        val entries = payloads.map { (name, text) ->
            val bytes = text.toByteArray(); Files.write(incoming.resolve(name), bytes)
            IndexTransferSpec.Entry(name, bytes.size.toLong(), MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) })
        }
        val spec = IndexTransferSpec.checked("small", descriptor, generation,
            IndexTransferSpec.Source("a".repeat(32), 300, "b".repeat(32), "c".repeat(64)), entries,
            IndexTransferSpec.QUERY_NAMES.map { IndexTransferSpec.Query(it, emptyList()) })
        val paths = listOf("snapshot.json", generation) + entries.map { it.path }
        TrustedIndexInput.requirePaths(spec, paths)
        for (bad in listOf(paths + "extra.bin", paths.dropLast(1), paths + paths.last(),
            paths.map { if (it == "CURRENT") "../CURRENT" else it }, List(IndexTransferSpec.MAX_FILES + 3) { "extra" })) {
            check(runCatching { TrustedIndexInput.requirePaths(spec, bad) }.exceptionOrNull() is IOException)
        }
        var opened = 0; var closed = 0; var live = 0; var cancel = false
        fun refuses(work: () -> Unit) { try { work(); error("Expected refusal") } catch (_: IOException) { } }
        val pathInput = TrustedIndexInput.path(incoming)
        pathInput.checkContents(spec)
        Files.writeString(incoming.resolve("extra.bin"), "unlisted")
        refuses { pathInput.checkContents(spec) }; Files.delete(incoming.resolve("extra.bin"))
        Files.delete(incoming.resolve("$generation/meta.json"))
        refuses { pathInput.checkContents(spec) }; Files.writeString(incoming.resolve("$generation/meta.json"), "{}")
        Files.createDirectory(incoming.resolve("unexpected-folder"))
        refuses { pathInput.checkContents(spec) }; Files.delete(incoming.resolve("unexpected-folder"))
        val source = object : TrustedIndexInput {
            override fun checkContents(spec: IndexTransferSpec) { check(spec.entries == entries); TrustedIndexInput.requirePaths(spec, paths) }
            override fun read(path: String, consume: (FileChannel) -> Unit) {
                check(path in payloads || path == "snapshot.json")
                opened++; live++; check(live == 1) { "Only one owned provider descriptor at a time" }
                try {
                    FileChannel.open(incoming.resolve(path), READ).use { channel ->
                        channel.position(1)
                        consume(channel)
                        check(channel.position() == 1L) { "Source reads must be positioned" }
                    }
                } finally { live--; closed++ }
            }
        }
        val installed = root.resolve("private")
        Files.createDirectories(installed.resolve("snapshot-old"))
        Files.writeString(installed.resolve("snapshot-old/marker"), "preserved")
        refuses { TrustedIndexInstaller.install(source, installed, spec, { false }, space = { 0 }, validate = {}) }
        check(opened == 0) { "No provider payload read before space admission" }
        var validations = 0
        refuses { TrustedIndexInstaller.install(source, installed, spec, { cancel }, space = { Long.MAX_VALUE }, validate = {
            validations++; cancel = true
        }) }
        check(validations == 1 && opened == closed && live == 0)
        check(!Files.exists(installed.resolve("snapshot-${spec.manifestHash}")))
        check(Files.readString(installed.resolve("snapshot-old/marker")) == "preserved")
        check(Files.list(installed).use { paths -> paths.noneMatch { it.fileName.toString().startsWith(".stage-") } })
        cancel = false
        refuses { TrustedIndexInstaller.install(source, installed, spec, { false }, space = { Long.MAX_VALUE },
            validate = { throw IOException("Core refused archive/index binding") }) }
        refuses { TrustedIndexInstaller.install(source, installed, spec, { false }, space = { Long.MAX_VALUE },
            withSelection = { throw IOException("Reader selection or Activity epoch changed") }, validate = {}) }
        check(!Files.exists(installed.resolve("snapshot-${spec.manifestHash}")))
        Files.writeString(incoming.resolve("$generation/segment.store"), "short")
        refuses { TrustedIndexInstaller.install(source, installed, spec, { false }, space = { Long.MAX_VALUE }, validate = { error("Must validate source size before Core") }) }
        Files.writeString(incoming.resolve("$generation/segment.store"), "corrupt index bytes!")
        refuses { TrustedIndexInstaller.install(source, installed, spec, { false }, space = { Long.MAX_VALUE }, validate = { error("Must checksum before Core") }) }
        check(opened == closed && live == 0)
        Files.writeString(incoming.resolve("$generation/segment.store"), payloads.getValue("$generation/segment.store"))
        val published = TrustedIndexInstaller.install(source, installed, spec, { false }, space = { Long.MAX_VALUE }, validate = { staged ->
            check(staged != installed.resolve("snapshot-${spec.manifestHash}"))
            check(Files.readString(staged.resolve("$generation/segment.store")) == payloads.getValue("$generation/segment.store"))
        })
        check(published == installed.resolve("snapshot-${spec.manifestHash}"))
        check(opened == closed && live == 0)
        check(Files.readString(installed.resolve("snapshot-old/marker")) == "preserved")
        println("Trusted index input: one-stage positioned copy, descriptor cleanup, checksum/space/cancel preservation")
    } finally {
        Files.walk(root).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists) }
    }
}
