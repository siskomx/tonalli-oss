package org.tonalli.spike.ai

import java.io.IOException

/** Fixed fixture selection only. Core owns retrieval, prompting and citation policy. */
class AiAnswerCase private constructor(val id: String, val language: String) {
    val dataset: String get() = "small"
    val maxTokens: UInt get() = 128u
    data class Question(val id: String, val questionEn: String, val questionEs: String, val retrievalQueryEn: String)
    data class Selected(val question: String, val retrievalQuery: String)

    fun select(questions: List<Question>): Selected {
        fun bounded(value: String) = value.isNotBlank() && '\u0000' !in value && value.toByteArray(Charsets.UTF_8).size <= 4096
        val expected = (1..10).map { "q${it.toString().padStart(2, '0')}" }.toSet()
        if (questions.size != 10 || questions.map { it.id }.toSet() != expected || questions.any {
            !bounded(it.questionEn) || !bounded(it.questionEs) || it.retrievalQueryEn != it.questionEn
        }) throw IOException("Invalid fixed cited-answer question catalog")
        val selected = questions.single { it.id == id }
        return Selected(if (language == "en") selected.questionEn else selected.questionEs, selected.questionEn)
    }

    companion object {
        fun requireAuthority(actualQuestionsSha256: String, expectedQuestionsSha256: String,
            preparedPackSha256: String, expectedPackSha256: String) {
            val hash = Regex("[a-f0-9]{64}")
            if (!hash.matches(expectedQuestionsSha256) || !hash.matches(expectedPackSha256) ||
                actualQuestionsSha256 != expectedQuestionsSha256 || preparedPackSha256 != expectedPackSha256)
                throw IOException("Fixed question catalog or SMALL source differs from packaged authority")
        }
        fun parse(value: Any?, legacyQuestion: Any?, legacyContext: Any?): AiAnswerCase {
            if (legacyQuestion != null || legacyContext != null) throw IOException("Fixed answer cases cannot use arbitrary question/context extras")
            if (value != null && value !is String) throw IOException("answer_case must be a string")
            val selected = value as String? ?: "q01:en"
            if (!Regex("q(?:0[1-9]|10):(en|es)").matches(selected)) throw IOException("answer_case must select q01..q10 and en/es")
            return AiAnswerCase(selected.substringBefore(':'), selected.substringAfter(':'))
        }
    }
}
