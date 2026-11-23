package org.tonalli.litert

import java.io.IOException
import java.nio.file.Files
import java.security.MessageDigest
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

private fun expectIo(block: () -> Unit) {
    try { block(); error("Expected IOException") } catch (_: IOException) { }
}

fun main() {
    val root = Files.createTempDirectory("litert-contract-")
    try {
        val incoming = Files.createDirectory(root.resolve("incoming"))
        val privateDir = Files.createDirectory(root.resolve("private"))
        val bytes = "small trusted test fixture".toByteArray()
        val digest = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
        val spec = TrustedModel(bytes.size.toLong(), digest)
        Files.write(incoming.resolve("model.litertlm"), bytes)
        val installed = ModelStaging.prepare(incoming, privateDir, spec)
        check(installed.fileName.toString() == "model.litertlm")
        check(installed.startsWith(privateDir))
        check(Files.readAllBytes(installed).contentEquals(bytes))
        Files.write(incoming.resolve("model.litertlm"), "wrong".toByteArray())
        check(ModelStaging.prepare(incoming, privateDir, spec) == installed)
        check(Files.readAllBytes(installed).contentEquals(bytes))
        val secondPrivate = Files.createDirectory(root.resolve("second"))
        expectIo { ModelStaging.prepare(incoming, secondPrivate, spec) }
        Files.write(incoming.resolve("model.litertlm"), bytes)
        expectIo { ModelStaging.prepare(incoming, secondPrivate, spec, cancelled = { true }) }
        expectIo { ModelStaging.prepare(incoming, secondPrivate, spec, availableBytes = { 0 }) }
        check(Files.list(secondPrivate).use { it.count() } == 0L)
        // Windows may deny symlink creation without Developer Mode; do not claim a pass then.
        val link = root.resolve("linked-incoming")
        try {
            Files.createSymbolicLink(link, incoming)
            expectIo { ModelStaging.prepare(link, secondPrivate, spec) }
        } catch (_: UnsupportedOperationException) { println("SKIP symlink creation unsupported")
        } catch (_: java.nio.file.FileSystemException) { println("SKIP symlink creation denied") }

        val lease = ProbeLease()
        var cancels = 0
        var closes = 0
        lease.start { cancels++ }
        lease.cancel()
        check(cancels == 1)
        check(!lease.sendIfActive { error("Cancellation must prevent a late native start") })
        check(!lease.isTerminal())
        check(!lease.closeIfTerminal { closes++ })
        lease.terminal()
        check(lease.closeIfTerminal { closes++ })
        check(!lease.closeIfTerminal { closes++ })
        lease.cancel()
        check(closes == 1 && cancels == 1)

        val cancelledInit = ProbeLease()
        cancelledInit.cancel()
        check(!cancelledInit.start { error("Must not start after initialization cancellation") })

        // JNI is permitted to wait for an off-thread terminal callback before returning.
        // Callback bookkeeping must never wait on a lock held across the JNI call.
        for (operation in listOf("cancel", "send")) {
            val callbackLease = ProbeLease()
            val terminalRecorded = CountDownLatch(1)
            val nativeReturning = CountDownLatch(1)
            fun nativeCall() {
                val callback = Thread {
                    callbackLease.terminal()
                    terminalRecorded.countDown()
                }.apply { start() }
                try {
                    check(terminalRecorded.await(500, TimeUnit.MILLISECONDS)) {
                        "$operation blocks its off-thread native terminal callback"
                    }
                    check(!callbackLease.closeIfTerminal { error("Must retain resources during native call") })
                } finally { nativeReturning.countDown() }
                callback.join(1000)
            }
            callbackLease.start { nativeCall() }
            if (operation == "cancel") callbackLease.cancel()
            else check(callbackLease.sendIfActive { nativeCall() })
            check(nativeReturning.count == 0L)
            check(callbackLease.closeIfTerminal { })
        }

        val race = ProbeLease()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        race.start { entered.countDown(); check(release.await(2, TimeUnit.SECONDS)) }
        val cancelling = Thread { race.cancel() }.apply { start() }
        check(entered.await(2, TimeUnit.SECONDS))
        var disposed = false
        val completing = Thread { race.terminal(); race.awaitQuiescentTerminal(); race.closeIfTerminal { disposed = true } }.apply { start() }
        check(!disposed)
        release.countDown()
        cancelling.join(2000); completing.join(2000)
        check(!cancelling.isAlive && !completing.isAlive && disposed)
        println("PASS model staging and cancellation lifetime contract")
    } finally {
        Files.walk(root).use { paths -> paths.sorted(Comparator.reverseOrder()).forEach(Files::deleteIfExists) }
    }
}
