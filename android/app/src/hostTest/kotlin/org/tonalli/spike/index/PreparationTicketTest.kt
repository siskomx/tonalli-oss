package org.tonalli.spike.index

fun preparationTicketTests() {
    check(preparationIdentity("book-1", "1") == Pair("book-1", 1uL))
    check(preparationIdentity("book-1", "18446744073709551615") == Pair("book-1", ULong.MAX_VALUE))
    for (epoch in listOf(null, "", "0", "01", "+1", "-1", "1e3", "1.0", " 1", "1 ", "18446744073709551616")) {
        check(preparationIdentity("book-1", epoch) == null)
    }
    for (pack in listOf(null, "", "../notes", "book/1", "content://book", "book 1", "a".repeat(129))) {
        check(preparationIdentity(pack, "1") == null)
    }
}
