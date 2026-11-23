package org.tonalli.spike.ai

import java.io.IOException

private fun answerRefused(work: () -> Unit) {
    try { work(); error("Expected typed cited-answer refusal") } catch (_: IOException) { }
}

fun answerCaseTests() {
    fixedAnswerSelection()
    fixedAnswerCatalog()
    fixedAnswerAuthority()
}

private fun fixedAnswerAuthority() {
    val questions = "a".repeat(64)
    val source = "b".repeat(64)
    AiAnswerCase.requireAuthority(questions, questions, source, source)
    answerRefused { AiAnswerCase.requireAuthority("c".repeat(64), questions, source, source) }
    answerRefused { AiAnswerCase.requireAuthority(questions, questions, "c".repeat(64), source) }
    answerRefused { AiAnswerCase.requireAuthority("", "", source, source) }
    answerRefused { AiAnswerCase.requireAuthority(questions, questions, "not a source pin", "not a source pin") }
}

private fun fixedAnswerSelection() {
    val default = AiAnswerCase.parse(null, null, null)
    check(default.id == "q01" && default.language == "en" && default.dataset == "small" && default.maxTokens == 128u)
    for (number in 1..10) for (language in listOf("en", "es")) {
        val id = "q${number.toString().padStart(2, '0')}"
        val selected = AiAnswerCase.parse("$id:$language", null, null)
        check(selected.id == id && selected.language == language && selected.dataset == "small")
    }
    for (invalid in listOf<Any>(1, true, "", "q00:en", "q11:en", "q01:fr", "q01:EN", "q1:en", "q01:en:extra", "../q01:en")) {
        answerRefused { AiAnswerCase.parse(invalid, null, null) }
    }
    answerRefused { AiAnswerCase.parse("q01:en", "arbitrary question", null) }
    answerRefused { AiAnswerCase.parse("q01:en", null, "arbitrary context") }
    answerRefused { AiAnswerCase.parse(null, 1, null) }
    answerRefused { AiAnswerCase.parse(null, null, true) }
}

private fun fixedAnswerCatalog() {
    // Synthetic decoded rows exercise policy; production text comes only from the packaged tracked JSON.
    val rows = (1..10).map { number ->
        val id = "q${number.toString().padStart(2, '0')}"
        AiAnswerCase.Question(id, "English full question $number?", "Pregunta española $number?", "English full question $number?")
    }
    val english = AiAnswerCase.parse("q03:en", null, null).select(rows)
    val spanish = AiAnswerCase.parse("q03:es", null, null).select(rows)
    check(english.question == rows[2].questionEn && spanish.question == rows[2].questionEs)
    check(english.retrievalQuery == rows[2].questionEn && spanish.retrievalQuery == english.retrievalQuery)
    val selection = AiAnswerCase.parse("q01:en", null, null)
    answerRefused { selection.select(rows.dropLast(1)) }
    answerRefused { selection.select(rows.dropLast(1) + rows.first()) }
    answerRefused { selection.select(rows.mapIndexed { index, row -> if (index == 0) row.copy(retrievalQueryEn = "privileged keywords") else row }) }
    for (bad in listOf("", "\u0000question", "é".repeat(2049))) {
        answerRefused { selection.select(rows.mapIndexed { index, row -> if (index == 0) row.copy(questionEs = bad) else row }) }
    }
}
