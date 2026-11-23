package org.tonalli.spike.saf

import java.io.IOException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

private fun rejects(block: () -> Unit) { try { block() } catch (_: IOException) { return }; error("Expected IOException") }

fun main() {
    for (suffix in listOf("aa", "ab", "zz")) {
        check(SafErrorText.missingSplitPart("duplicate part or missing part .zim$suffix") ==
            "Duplicate part or missing part .zim$suffix; select the complete group and reopen.")
    }
    for (reason in listOf("invalid split filename", "duplicate part or missing part .zimab\n",
        "duplicate part or missing part /private/notes.zimab", "duplicate part or missing part .zimAB",
        "duplicate part or missing part .zimabc", "prefix duplicate part or missing part .zimab",
        "duplicate part or missing part .zimab\nprovider secret", "duplicate part or missing part .zimab\u0000")) {
        check(SafErrorText.missingSplitPart(reason) == null)
    }
    val documents = listOf(SafDocument("content://provider/b", "Wiki.zimab"), SafDocument("content://provider/a", "Wiki.zimaa"))
    class Descriptor(val fd: Int, val fail: Boolean = false) : TransferDescriptor {
        var closes = 0
        override fun detachFd(): Int { if (fail) throw IOException("detach"); return fd }
        override fun close() { closes++ }
    }
    fun exercise(failOpen: Int = -1, failDetach: Int = -1, cancelAt: Int = -1, enter: Boolean = true, nativeFails: Boolean = false): Pair<List<Descriptor>, List<Int>> {
        val opened = mutableListOf<Descriptor>(); val reclaimed = mutableListOf<Int>(); var checks = 0
        try {
            DescriptorTransfer.open(documents, { checks++ == cancelAt }, { _ ->
                if (opened.size == failOpen) throw IOException("open")
                Descriptor(40 + opened.size, opened.size == failDetach).also(opened::add)
            }, reclaimed::add) { parts, entered ->
                check(parts.map { it.name } == documents.map { it.name })
                check(parts.map { it.fd } == listOf(40, 41))
                if (enter) entered()
                if (nativeFails) throw IOException("native typed error")
            }
            check(enter && !nativeFails && failOpen < 0 && failDetach < 0 && cancelAt < 0)
        } catch (_: IOException) { }
        check(opened.all { it.closes == 1 })
        return opened to reclaimed
    }
    check(exercise().second.isEmpty())
    check(exercise(nativeFails = true).second.isEmpty())
    check(exercise(enter = false).second == listOf(40, 41))
    check(exercise(enter = false, nativeFails = true).second == listOf(40, 41))
    check(exercise(failOpen = 1).second.isEmpty())
    check(exercise(failDetach = 1).second == listOf(40))
    val closedAfterFailure = mutableListOf<Int>()
    rejects {
        DescriptorTransfer.open(documents, { false }, { doc ->
            object : TransferDescriptor {
                override fun detachFd() = if (doc == documents.first()) 70 else 71
                override fun close() { if (doc == documents.last()) throw IOException("wrapper close") }
            }
        }, { fd -> closedAfterFailure.add(fd); if (fd == 70) throw IOException("raw close") }) { _, _ ->
            error("Native must not be entered when wrapper cleanup failed")
        }
    }
    check(closedAfterFailure == listOf(70, 71))
    for ((point, expected) in listOf(emptyList(), emptyList(), emptyList(), emptyList(), listOf(40), listOf(40, 41), listOf(40, 41)).withIndex()) {
        check(exercise(cancelAt = point).second == expected)
    }
    check(SafSelections.checked(SafSelection(SafKind.FILES, documents)).documents == documents)
    check(SafSelections.checked(SafSelection(SafKind.FILES, listOf(SafDocument("content://p/a", "")))).documents.single().name.isEmpty())
    rejects { SafSelections.checked(SafSelection(SafKind.FILES, documents + SafDocument("content://p/c", ""))) }
    check(SafSelections.groups(documents).single() == documents)
    check(SafSelections.groups(listOf(SafDocument("content://p/1", "one.iso"), SafDocument("content://p/2", "two.zim"))).size == 2)
    rejects { SafSelections.checked(SafSelection(SafKind.FILES, emptyList())) }
    rejects { SafSelections.groups(List(257) { SafDocument("content://p/$it", "$it.zim") }) }
    rejects { SafSelections.checked(SafSelection(SafKind.FILES, List(129) { SafDocument("content://p/$it", "x.zimaa") })) }
    for (path in listOf("", "a//b", "../a", "a/./b", "a\\b", "a\u0000b", "a/", "x/".repeat(65))) {
        rejects { SafSelections.checked(SafSelection(SafKind.ISO, documents.take(1), innerPath = path)) }
    }
    check(SafSelections.checked(SafSelection(SafKind.ISO, documents.take(1), innerPath = "/nested/wiki.zim")).innerPath == "/nested/wiki.zim")
    rejects { SafSelections.checked(SafSelection(SafKind.ISO, documents.take(1), innerPath = "/" + "x".repeat(4096))) }
    val publication = SafPublication()
    val stale = publication.begin()
    publication.invalidate()
    check(!publication.publish(stale) { error("Stale selection activated") })
    val current = publication.begin()
    check(publication.publish(current) { })
    val first = SafPublication(); val second = SafPublication()
    val firstEpoch = first.begin(); val secondEpoch = second.begin()
    val start = CountDownLatch(1); val active = AtomicInteger(); val commits = AtomicInteger()
    val writers = listOf(first to firstEpoch, second to secondEpoch).map { (gate, epoch) ->
        Thread {
            start.await()
            gate.publish(epoch) {
                check(active.incrementAndGet() == 1)
                Thread.sleep(20)
                commits.incrementAndGet()
                check(active.decrementAndGet() == 0)
            }
        }.also(Thread::start)
    }
    start.countDown(); writers.forEach(Thread::join)
    check(commits.get() == 2 && active.get() == 0)
    val generation = AtomicLong(1)
    check(SafReadFence.removedEpoch(generation, 1) == 2L)
    generation.set(3)
    check(SafReadFence.removedEpoch(generation, 1) == null && generation.get() == 3L)
    checkProbeInputs()
    checkSafBenchmark()
    println("SAF ownership and selection checks passed")
}
