package org.tonalli.spike.ai

import java.io.IOException
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.Path
import java.nio.file.StandardOpenOption.READ
import java.security.MessageDigest

/** Synthetic copy fixtures only: no native model, Android runtime or large pack is loaded. */
fun main() {
    trustedDescriptorChecks()
    checkedCopyAndReuse()
    sharedVolumeBudget()
    failuresPreservePreviousGeneration()
    cancellationNeverPublishes()
    corruptionAndPublicationFailurePreserveExistingFiles()
    linkedPathsAreRefusedWhenSupported()
    trustedRootAliasesAndOwnedLinks()
    reusedModelCompletionCancellation()
    installedOnlyNeverStagesOrAcceptsCorruption()
    println("PASS: trusted single-model descriptors, verified bounded staging, shared-volume space, cancellation/failure/old-generation preservation")
}

private fun installedOnlyNeverStagesOrAcceptsCorruption() = Fixture().use { fixture ->
    ioFailure("installed-only refuses missing publication") {
        AiModelFiles.installed(fixture.files, fixture.spec)
    }
    check(!Files.exists(fixture.installed) && fixture.stages().isEmpty())
    val installed = fixture.prepare()
    val modified = Files.getLastModifiedTime(installed)
    check(AiModelFiles.installed(fixture.files, fixture.spec) == installed)
    check(Files.getLastModifiedTime(installed) == modified)
    ioFailure("installed-only sticky cancellation") {
        AiModelFiles.installed(fixture.files, fixture.spec) { true }
    }
    val corrupt = fixture.payload.copyOf().also { it[0] = (it[0].toInt() xor 1).toByte() }
    Files.write(installed, corrupt)
    ioFailure("installed-only rejects same-length corruption without repair") {
        AiModelFiles.installed(fixture.files, fixture.spec)
    }
    check(Files.readAllBytes(installed).contentEquals(corrupt) && fixture.stages().isEmpty())
    fixture.previousUntouched()
}

private fun digest(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256")
    .digest(bytes).joinToString("") { "%02x".format(it) }

private fun checkedSpec(id: String, role: String, bytes: Long, sha: String): AiModelSpec =
    AiModelSpec.checked(id, role, bytes, sha)

private fun ioFailure(label: String, work: () -> Unit) {
    try { work() } catch (_: IOException) { return }
    error("Expected typed IOException: $label")
}

private class Fixture : AutoCloseable {
    private val tempRoot = Path.of(System.getProperty("java.io.tmpdir")).toRealPath()
    val base: Path = Files.createTempDirectory(tempRoot, "tonalli-ai-model-test-").toRealPath()
    val files: Path = Files.createDirectory(base.resolve("files"))
    val incoming: Path = Files.createDirectory(base.resolve("incoming"))
    val payload: ByteArray = ByteArray(2 * 64 * 1024 + 17) { ((it * 37 + 11) and 255).toByte() }
    val spec: AiModelSpec = checkedSpec("synthetic-stage-fixture", "generation", payload.size.toLong(), digest(payload))
    val installed: Path get() = files.resolve("ai/models/${spec.sha256}/model.gguf")
    val oldFile: Path
    val active: Path
    val oldBytes = "existing model remains owned by an open reader".toByteArray()
    val activeBytes = "previous-selected-model\n".toByteArray()

    init {
        Files.write(incoming.resolve("model.gguf"), payload)
        val old = Files.createDirectories(files.resolve("ai/models/previous-immutable-generation"))
        oldFile = Files.write(old.resolve("model.gguf"), oldBytes)
        active = Files.write(files.resolve("ai/ACTIVE"), activeBytes)
    }

    fun prepare(cancelled: () -> Boolean = { false }, available: Long = Long.MAX_VALUE): Path =
        AiModelFiles.prepare(incoming, files, spec, cancelled, availableBytes = { available })

    fun stages(): List<Path> {
        val models = files.resolve("ai/models")
        if (!Files.isDirectory(models, NOFOLLOW_LINKS)) return emptyList()
        return Files.list(models).use { paths -> paths.filter { it.fileName.toString().startsWith(".stage-") }.toList() }
    }

    fun stagedModelSize(): Long = stages().map { it.resolve("model.gguf") }
        .filter { Files.isRegularFile(it, NOFOLLOW_LINKS) }.maxOfOrNull(Files::size) ?: 0

    fun previousUntouched() {
        check(Files.readAllBytes(oldFile).contentEquals(oldBytes))
        check(Files.readAllBytes(active).contentEquals(activeBytes))
    }

    override fun close() {
        check(base.isAbsolute && base.parent == tempRoot && base.fileName.toString().startsWith("tonalli-ai-model-test-"))
        // Files.walk doesn't follow symbolic links; cleanup is confined to this owned root.
        Files.walk(base).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists) }
    }
}

private fun trustedDescriptorChecks() {
    // Actual catalog pins exercise Long-sized metadata without allocating/copying these models.
    val models = listOf(
        checkedSpec("qwen3-1.7b-q4-k-m", "generation", 1_282_439_264L,
            "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5"),
        checkedSpec("qwen3-4b-q4-k-m", "generation", 2_497_280_256L,
            "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5"),
        checkedSpec("bge-small-en-v1.5-q8-0", "embedding", 36_685_152L,
            "f046db1dc724cf4f6f0a0c5917e922823b73eb1d27b8f9a9c2797f7866974804"),
    )
    check(models[1].bytes > Int.MAX_VALUE.toLong())
    for (model in models) {
        check(AiModelFiles.requiredAdditionalBytes(model) > model.bytes)
    }
    for (id in listOf("", "../outside", "/absolute", "a/b", "a\\b", "name:stream", "two words")) {
        ioFailure("unsafe descriptor id") { checkedSpec(id, "generation", 1, "a".repeat(64)) }
    }
    ioFailure("unknown role") { checkedSpec("known", "other", 1, "a".repeat(64)) }
    ioFailure("zero bytes") { checkedSpec("known", "generation", 0, "a".repeat(64)) }
    ioFailure("negative bytes") { checkedSpec("known", "generation", -1, "a".repeat(64)) }
    ioFailure("overflowing stage budget") {
        AiModelFiles.requiredAdditionalBytes(checkedSpec("known", "generation", Long.MAX_VALUE, "a".repeat(64)))
    }
    ioFailure("malformed SHA") { checkedSpec("known", "generation", 1, "A".repeat(64)) }
}

private fun checkedCopyAndReuse() = Fixture().use { fixture ->
    var greatestProgress = 0L
    var largestStep = 0L
    val installed = fixture.prepare(cancelled = {
        check(!Files.exists(fixture.installed, NOFOLLOW_LINKS))
        val progress = fixture.stagedModelSize()
        largestStep = maxOf(largestStep, progress - greatestProgress)
        greatestProgress = maxOf(greatestProgress, progress)
        false
    })
    check(installed == fixture.installed)
    check(Files.readAllBytes(installed).contentEquals(fixture.payload))
    check(greatestProgress == fixture.payload.size.toLong() && largestStep <= 64 * 1024)
    Files.list(installed.parent).use { check(it.toList() == listOf(installed)) }
    check(fixture.stages().isEmpty())
    val modified = Files.getLastModifiedTime(installed)
    check(fixture.prepare(available = 0) == installed)
    check(Files.getLastModifiedTime(installed) == modified)
    fixture.previousUntouched()
}

private fun sharedVolumeBudget() = Fixture().use { fixture ->
    val required = AiModelFiles.requiredAdditionalBytes(fixture.spec)
    check(required == fixture.spec.bytes + AiModelFiles.RESERVE_BYTES)
    ioFailure("one byte short of additional internal stage") { fixture.prepare(available = required - 1) }
    check(!Files.exists(fixture.installed) && fixture.stages().isEmpty())
    fixture.previousUntouched()
    // Existing incoming and old installed bytes already reduce usableSpace; charge neither again.
    check(Files.isRegularFile(fixture.incoming.resolve("model.gguf")))
    check(fixture.prepare(available = required) == fixture.installed)
    fixture.previousUntouched()
}

private fun failuresPreservePreviousGeneration() {
    for (failure in listOf("short", "long", "checksum", "missing")) Fixture().use { fixture ->
        when (failure) {
            "short" -> Files.write(fixture.incoming.resolve("model.gguf"), fixture.payload.copyOf(fixture.payload.size - 1))
            "long" -> Files.write(fixture.incoming.resolve("model.gguf"), fixture.payload + byteArrayOf(1))
            "checksum" -> Files.write(fixture.incoming.resolve("model.gguf"), fixture.payload.copyOf().also { it[100] = (it[100].toInt() xor 1).toByte() })
            "missing" -> Files.delete(fixture.incoming.resolve("model.gguf"))
        }
        val other = Files.createDirectory(fixture.files.resolve("ai/models/.stage-unrelated-owner"))
        val sentinel = Files.write(other.resolve("keep"), byteArrayOf(7))
        FileChannel.open(fixture.oldFile, READ).use { lease ->
            ioFailure("$failure preserves prior activation") { fixture.prepare() }
            val bytes = ByteBuffer.allocate(fixture.oldBytes.size)
            check(lease.read(bytes, 0) == fixture.oldBytes.size && bytes.array().contentEquals(fixture.oldBytes))
        }
        check(!Files.exists(fixture.installed) && fixture.stages() == listOf(other))
        check(Files.readAllBytes(sentinel).contentEquals(byteArrayOf(7)))
        fixture.previousUntouched()
    }
}

private fun cancellationNeverPublishes() {
    Fixture().use { fixture ->
        ioFailure("cancel before copy") { fixture.prepare(cancelled = { true }) }
        check(!Files.exists(fixture.installed) && fixture.stages().isEmpty())
        fixture.previousUntouched()
    }
    Fixture().use { fixture ->
        var observedCopy = false
        ioFailure("cancel during positioned chunk copy") {
            fixture.prepare(cancelled = { (fixture.stagedModelSize() >= 64 * 1024).also { if (it) observedCopy = true } })
        }
        check(observedCopy && !Files.exists(fixture.installed) && fixture.stages().isEmpty())
        fixture.previousUntouched()
    }
    Fixture().use { fixture ->
        var observedCompleteCopy = false
        ioFailure("cancel after complete copy before publication") {
            fixture.prepare(cancelled = {
                (fixture.stagedModelSize() == fixture.payload.size.toLong())
                    .also { if (it) observedCompleteCopy = true }
            })
        }
        check(observedCompleteCopy && !Files.exists(fixture.installed) && fixture.stages().isEmpty())
        fixture.previousUntouched()
    }
}

private fun corruptionAndPublicationFailurePreserveExistingFiles() {
    Fixture().use { fixture ->
        val installed = fixture.prepare()
        val corrupted = fixture.payload.copyOf().also { it[0] = (it[0].toInt() xor 1).toByte() }
        Files.write(installed, corrupted)
        ioFailure("same-length installed corruption is not overwritten") { fixture.prepare() }
        check(Files.readAllBytes(installed).contentEquals(corrupted) && fixture.stages().isEmpty())
        fixture.previousUntouched()
    }
    Fixture().use { fixture ->
        Files.write(fixture.installed.parent, byteArrayOf(9))
        ioFailure("publication target is not a directory") { fixture.prepare() }
        check(Files.readAllBytes(fixture.installed.parent).contentEquals(byteArrayOf(9)) && fixture.stages().isEmpty())
        fixture.previousUntouched()
    }
    Fixture().use { fixture ->
        val otherPublisherBytes = byteArrayOf(42)
        var publishedByOther = false
        ioFailure("concurrent publication is never overwritten") {
            fixture.prepare(cancelled = {
                if (!publishedByOther && fixture.stagedModelSize() >= 64 * 1024) {
                    Files.createDirectory(fixture.installed.parent)
                    Files.write(fixture.installed, otherPublisherBytes)
                    publishedByOther = true
                }
                false
            })
        }
        check(publishedByOther && Files.readAllBytes(fixture.installed).contentEquals(otherPublisherBytes))
        check(fixture.stages().isEmpty())
        fixture.previousUntouched()
    }
}

private fun linkedPathsAreRefusedWhenSupported() = Fixture().use { fixture ->
    val outside = Files.createDirectory(fixture.base.resolve("outside-app-files"))
    val external = Files.write(outside.resolve("model.gguf"), fixture.payload)
    val link = fixture.incoming.resolve("linked-probe")
    try { Files.createSymbolicLink(link, external) } catch (_: UnsupportedOperationException) {
        println("SKIP: host does not support symbolic links")
        return@use
    } catch (_: java.nio.file.FileSystemException) {
        println("SKIP: host lacks symbolic-link privilege")
        return@use
    }
    Files.delete(link)
    Files.delete(fixture.incoming.resolve("model.gguf"))
    Files.createSymbolicLink(fixture.incoming.resolve("model.gguf"), external)
    ioFailure("linked incoming model") { fixture.prepare() }
    check(Files.readAllBytes(external).contentEquals(fixture.payload))
    fixture.previousUntouched()
}

private fun trustedRootAliasesAndOwnedLinks() = Fixture().use { fixture ->
    val filesAlias = fixture.base.resolve("context-files-alias")
    val incomingAlias = fixture.base.resolve("context-incoming-alias")
    try { Files.createSymbolicLink(filesAlias, fixture.files) } catch (_: UnsupportedOperationException) {
        println("SKIP: trusted-root alias test needs symbolic links")
        return@use
    } catch (_: java.nio.file.FileSystemException) {
        println("SKIP: trusted-root alias test needs symbolic-link privilege")
        return@use
    }
    Files.createSymbolicLink(incomingAlias, fixture.incoming)
    // These two roots are explicit Context-provided authorities, even when the OS aliases them.
    check(AiModelFiles.prepare(incomingAlias, filesAlias, fixture.spec) == fixture.installed)
    fixture.previousUntouched()

    val outside = Files.createDirectory(fixture.base.resolve("outside-owned-tree"))
    val realAi = fixture.files.resolve("ai")
    val movedAi = fixture.files.resolve("saved-ai")
    Files.move(realAi, movedAi)
    Files.createSymbolicLink(realAi, outside)
    ioFailure("owned ai child symlink") { AiModelFiles.prepare(incomingAlias, filesAlias, fixture.spec) }
    Files.delete(realAi)
    Files.move(movedAi, realAi)

    val models = realAi.resolve("models")
    val movedModels = realAi.resolve("saved-models")
    Files.move(models, movedModels)
    Files.createSymbolicLink(models, outside)
    ioFailure("owned models child symlink") { AiModelFiles.prepare(incomingAlias, filesAlias, fixture.spec) }
    Files.delete(models)
    Files.move(movedModels, models)

    val externalModel = Files.write(outside.resolve("model.gguf"), fixture.payload)
    Files.delete(fixture.installed)
    Files.createSymbolicLink(fixture.installed, externalModel)
    ioFailure("published model leaf symlink") { AiModelFiles.prepare(incomingAlias, filesAlias, fixture.spec) }
    check(Files.readAllBytes(externalModel).contentEquals(fixture.payload))
    fixture.previousUntouched()
}

private fun reusedModelCompletionCancellation() = Fixture().use { fixture ->
    val payload = byteArrayOf(17)
    val spec = checkedSpec("single-byte-cancellation", "embedding", 1, digest(payload))
    Files.write(fixture.incoming.resolve("model.gguf"), payload)
    val installed = AiModelFiles.prepare(fixture.incoming, fixture.files, spec)
    var checks = 0
    // Permit entry and the single read, then cancel at completed verification before return.
    ioFailure("reused model completion cancellation") {
        AiModelFiles.prepare(fixture.incoming, fixture.files, spec,
            cancelled = { ++checks >= 3 }, availableBytes = { 0 })
    }
    check(checks == 3 && Files.readAllBytes(installed).contentEquals(payload))
    check(fixture.stages().isEmpty())
    fixture.previousUntouched()
}
