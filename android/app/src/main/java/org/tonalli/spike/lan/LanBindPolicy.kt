package org.tonalli.spike.lan

import java.net.Inet6Address
import java.net.InetAddress

/** Candidates must come from actual link properties; never resolve a hostname. */
object LanBindPolicy {
    fun allows(address: String, wifi: Boolean, vpn: Boolean): Boolean {
        if (!wifi || vpn) return false
        if (':' in address) {
            if (!address.matches(Regex("[0-9a-fA-F:]+"))) return false
            val parsed = try { InetAddress.getByName(address) } catch (_: Exception) { return false }
            return parsed is Inet6Address && (parsed.address[0].toInt() and 0xfe) == 0xfc
        }
        val parts = address.split('.')
        if (parts.size != 4 || parts.any { !it.matches(Regex("0|[1-9][0-9]{0,2}")) }) return false
        val bytes = parts.map { it.toInt() }
        if (bytes.any { it > 255 }) return false
        return bytes[0] == 10 || (bytes[0] == 172 && bytes[1] in 16..31) ||
            (bytes[0] == 192 && bytes[1] == 168) || (bytes[0] == 169 && bytes[1] == 254)
    }
}
