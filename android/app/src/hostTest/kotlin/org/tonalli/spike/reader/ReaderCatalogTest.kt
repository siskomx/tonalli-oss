package org.tonalli.spike.reader

import org.tonalli.spike.saf.*

/** The original RED ran this same contract against the reused diagnostic chooser. */
private fun readerFolderPolicyTests(groups: (List<SafDocument>) -> List<List<SafDocument>>) {
    val standalone = SafDocument("content://provider/standalone", "encyclopedia.zim")
    val ab = SafDocument("content://provider/ab", "knots.zimab")
    val aa = SafDocument("content://provider/aa", "knots.zimaa")
    val iso = SafDocument("content://provider/iso", "optical.iso")
    val isoUpper = SafDocument("content://provider/iso2", "optical.ISO")
    val unrelated = SafDocument("content://provider/txt", "readme.txt")
    val actual = groups(listOf(standalone, ab, aa, iso, isoUpper, unrelated))
    check(actual == listOf(listOf(standalone), listOf(ab, aa))) {
        "Normal reader folder choices must exclude optical images and preserve ZIM/split display names and provider order"
    }
    // Grouping is a chooser, never a promise that Core will accept the split archive.
    check(groups(listOf(ab)) == listOf(listOf(ab)))
    check(groups(emptyList()).isEmpty())
    check(runCatching { groups((0..256).map { SafDocument("content://provider/$it", "$it.zim") }) }.isFailure)
    check(runCatching { groups(listOf(SafDocument("content://provider/bad", "x".repeat(4097) + ".zim"))) }.isFailure)
    println("Reader folder: ZIM-only choices, incomplete group retained for Core, unchanged names/order, bounded enumeration passed")
}

fun main() {
    readerIndexTests()
    readerFolderPolicyTests(ReaderFolderPolicy::groups)
    for (request in listOf(81, 83)) {
        check(ReaderFolderPolicy.restoreRequest(request) == request)
        check(ReaderFolderPolicy.acceptsResult(ReaderFolderPolicy.restoreRequest(request), request))
    }
    check(ReaderFolderPolicy.restoreRequest(0) == null)
    check(ReaderFolderPolicy.restoreRequest(73) == null) // Diagnostic ISO request is not a reader request.
    check(!ReaderFolderPolicy.acceptsResult(null, 83))
    check(!ReaderFolderPolicy.acceptsResult(81, 83))
    val a = SafSelection(SafKind.FILES, listOf(SafDocument("content://provider/a", "a.zim")))
    val b = SafSelection(SafKind.FILES, listOf(SafDocument("content://provider/b", "b.zim")))
    val identity = ReaderIdentity("a".repeat(32), "b".repeat(32))
    val changed = ReaderIdentity("c".repeat(32), "d".repeat(32))
    val first = ReaderCatalogPolicy.add(emptyList(), a, identity, "reader-00000000-0000-0000-0000-000000000001")
    val second = ReaderCatalogPolicy.add(first, b, identity, "reader-00000000-0000-0000-0000-000000000002")
    check(second.size == 2 && second.first() == first.first())
    check(ReaderCatalogPolicy.add(second, a, identity, "reader-00000000-0000-0000-0000-000000000003") == second)
    check(runCatching { ReaderCatalogPolicy.requireIdentity(first.single(), changed) }.isFailure)
    ReaderCatalogPolicy.requireIdentity(first.single(), identity)
    val reverified = ReaderCatalogPolicy.add(second, a, changed, "reader-00000000-0000-0000-0000-000000000004")
    check(reverified.size == 2 && reverified[0].id == first[0].id && reverified[0].identity == changed && reverified[1] == second[1])
    check(first[0].identity == identity) // A failed validation cannot mutate a published snapshot.
    check(runCatching { ReaderCatalogPolicy.add(first, b, identity, first.single().id) }.isFailure)
    check(runCatching { ReaderCatalogPolicy.add(first, a.copy(documents = emptyList()), identity, "invalid") }.isFailure)
    check(runCatching { ReaderCatalogPolicy.add(emptyList(), a, identity.copy(uuid = "bad"), first.single().id) }.isFailure)
    check(runCatching { ReaderCatalogPolicy.checked(listOf(first.single(), first.single())) }.isFailure)
    check(runCatching { ReaderCatalogPolicy.checked((1..65).map { ReaderEntry("reader-" + "%08d-0000-0000-0000-000000000001".format(it), a, identity) }) }.isFailure)
    val tree = SafSelection(SafKind.TREE, a.documents, treeUri = "content://provider/tree")
    val withTree = ReaderCatalogPolicy.add(second, tree, identity, "reader-00000000-0000-0000-0000-000000000005")
    check(withTree.size == 3 && withTree.take(2) == second && withTree.last().selection.treeUri == tree.treeUri)
    check(ReaderCatalogPolicy.add(withTree, tree, identity, "reader-00000000-0000-0000-0000-000000000006") == withTree)
    check(runCatching { ReaderCatalogPolicy.add(second, tree.copy(treeUri = null), identity, "reader-00000000-0000-0000-0000-000000000007") }.isFailure)
    println("Reader catalog: multiple sources, stable duplicate identity, invalid/duplicate/bounded records passed")
}
