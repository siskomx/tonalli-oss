package org.tonalli.spike.ai

import java.io.IOException

/** Exact optional model pins from docs/spike/packs.toml; installation is independent of RAM tier. */
object AiModelCatalog {
    val models: List<AiModelSpec> = listOf(
        AiModelSpec.checked("bge-small-en-v1.5-q8-0", "embedding", 36_685_152L,
            "f046db1dc724cf4f6f0a0c5917e922823b73eb1d27b8f9a9c2797f7866974804"),
        AiModelSpec.checked("qwen3-1.7b-q4-k-m", "generation", 1_282_439_264L,
            "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5"),
        AiModelSpec.checked("qwen3-4b-q4-k-m", "generation", 2_497_280_256L,
            "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5"),
    )
    fun model(id: String): AiModelSpec = models.singleOrNull { it.id == id }
        ?: throw IOException("Unknown trusted AI model")
}
