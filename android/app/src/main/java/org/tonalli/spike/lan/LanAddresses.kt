package org.tonalli.spike.lan

internal data class LanBinding(val interfaceName: String, val address: String, val hotspot: Boolean)

internal object LanAddresses {
    fun retains(binding: LanBinding, permittedLinks: Map<String, List<String>>): Boolean =
        binding.address in permittedLinks[binding.interfaceName].orEmpty()

    /** Only a single newly created interface can be attributed to our hotspot request. */
    fun hotspot(before: Set<String>, candidates: Map<String, List<String>>): String? {
        val added = candidates.filterKeys { it !in before }.mapValues { (_, addresses) ->
            addresses.filter { LanBindPolicy.allows(it, wifi = true, vpn = false) }
        }.filterValues { it.isNotEmpty() }
        val addresses = added.values.singleOrNull() ?: return null
        return addresses.sortedWith(compareBy<String> { ':' in it }.thenBy { it }).first()
    }
}
