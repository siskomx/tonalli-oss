package org.tonalli.spike.lan

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import java.net.NetworkInterface

internal class LanNetwork(context: Context) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)

    private fun wifiLinks(): Map<String, List<String>> = connectivity.allNetworks.mapNotNull { network ->
        val caps = connectivity.getNetworkCapabilities(network) ?: return@mapNotNull null
        if (!caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
            caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) ||
            caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) return@mapNotNull null
        val link = connectivity.getLinkProperties(network) ?: return@mapNotNull null
        val name = link.interfaceName ?: return@mapNotNull null
        name to link.linkAddresses.mapNotNull { it.address.hostAddress }
    }.toMap()

    fun wifiBinding(ipv4Only: Boolean = false, ipv6Only: Boolean = false): LanBinding? = wifiLinks().firstNotNullOfOrNull { (name, addresses) ->
        addresses.firstOrNull { (!ipv4Only || ':' !in it) && (!ipv6Only || ':' in it) && LanBindPolicy.allows(it, wifi = true, vpn = false) }
            ?.let { LanBinding(name, it, hotspot = false) }
    }

    fun interfaces(): Set<String> = NetworkInterface.getNetworkInterfaces().toList().map { it.name }.toSet()

    private fun hotspotLinks(): Map<String, List<String>> {
        val forbidden = connectivity.allNetworks.mapNotNull { network ->
            val caps = connectivity.getNetworkCapabilities(network) ?: return@mapNotNull null
            if (caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) ||
                caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR))
                connectivity.getLinkProperties(network)?.interfaceName else null
        }.toSet()
        return NetworkInterface.getNetworkInterfaces().toList()
            .filter { it.isUp && !it.isLoopback && !it.isVirtual && it.name !in forbidden }
            .associate { iface -> iface.name to iface.inetAddresses.toList().mapNotNull { it.hostAddress } }
    }

    fun hotspotBinding(before: Set<String>): LanBinding? {
        val candidates = hotspotLinks()
        val address = LanAddresses.hotspot(before, candidates) ?: return null
        val name = candidates.entries.single { it.key !in before && address in it.value }.key
        return LanBinding(name, address, hotspot = true)
    }

    fun contains(binding: LanBinding): Boolean = LanAddresses.retains(binding,
        if (binding.hotspot) hotspotLinks() else wifiLinks())
}
