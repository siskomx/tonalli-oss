package org.tonalli.spike.ai

import java.io.IOException

/** Metadata supplied by the trusted catalog, never inferred from an incoming filename. */
class AiModelSpec private constructor(val id: String, val role: String, val bytes: Long, val sha256: String) {
    companion object {
        fun checked(id: String, role: String, bytes: Long, sha256: String): AiModelSpec {
            if (!id.matches(Regex("[a-z0-9][a-z0-9.-]{0,79}")) || role !in setOf("generation", "embedding") ||
                bytes <= 0 || !sha256.matches(Regex("[a-f0-9]{64}"))) {
                throw IOException("Invalid trusted AI model metadata")
            }
            return AiModelSpec(id, role, bytes, sha256)
        }
    }
}
