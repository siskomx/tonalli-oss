package org.tonalli.spike.lan

fun main() {
    bootstrapHandoffTests()
    val wifiBinding = LanBinding("selected", "192.168.1.2", hotspot = false)
    check(LanAddresses.retains(wifiBinding, mapOf("selected" to listOf("192.168.1.2"))))
    check(!LanAddresses.retains(wifiBinding, mapOf("other" to listOf("192.168.1.2"))))
    check(!LanAddresses.retains(wifiBinding, mapOf("selected" to listOf("192.168.1.3"))))
    check(!LanAddresses.retains(wifiBinding, emptyMap()))
    check(LanAddresses.retains(LanBinding("owned-hotspot", "fd00::1", hotspot = true),
        mapOf("owned-hotspot" to listOf("fd00::1", "192.168.43.1"))))
    check(LanAddresses.hotspot(setOf("existing"), mapOf("existing" to listOf("192.168.1.2"),
        "new" to listOf("fd00::1", "192.168.43.1", "192.168.43.2"))) == "192.168.43.1")
    check(LanAddresses.hotspot(emptySet(), mapOf("one" to listOf("10.0.0.1"), "two" to listOf("192.168.1.1"))) == null)
    check(LanAddresses.hotspot(emptySet(), mapOf("new" to listOf("8.8.8.8", "100.64.0.1"))) == null)
    check(LanAddresses.hotspot(setOf("existing"), mapOf("existing" to listOf("10.0.0.1", "192.168.43.1"))) == null)
    check(LanAddresses.hotspot(emptySet(), mapOf("new" to listOf("fd00::1"))) == "fd00::1")
    val closed = mutableListOf<String>()
    val state = LanLifecycle()
    check(!state.isStarting && !state.isActive)
    check(state.begin(lowBattery = true) == null)
    val first = checkNotNull(state.begin(lowBattery = false))
    check(state.begin(false) == null)
    check(state.adopt(first, AutoCloseable { closed += "multicast" }))
    state.stop()
    check(!state.adopt(first, AutoCloseable { closed += "late-native" }))
    check(closed == listOf("multicast", "late-native"))
    val second = checkNotNull(state.begin(false))
    check(state.adopt(second, AutoCloseable { closed += "hotspot" }))
    check(state.adopt(second, AutoCloseable { closed += "wake" }))
    check(!state.complete(first))
    check(state.complete(second) && state.isActive)
    state.stop()
    state.stop()
    check(closed.takeLast(2) == listOf("wake", "hotspot"))
    val failing = checkNotNull(state.begin(false))
    var released = false
    state.adopt(failing, AutoCloseable { released = true })
    state.adopt(failing, AutoCloseable { throw IllegalStateException("close") })
    try { state.stop() } catch (_: IllegalStateException) { }
    check(released && !state.isActive && !state.isStarting)
    val repeated = checkNotNull(state.begin(false))
    var finalLeaseClosed = false
    val sharedFailure = IllegalStateException("shared close failure")
    state.adopt(repeated, AutoCloseable { finalLeaseClosed = true })
    repeat(2) { state.adopt(repeated, AutoCloseable { throw sharedFailure }) }
    var observed: Throwable? = null
    try { state.stop() } catch (error: Throwable) { observed = error }
    check(finalLeaseClosed && observed === sharedFailure)

    for (address in listOf("10.0.2.16", "172.16.0.1", "172.31.255.254", "192.168.1.1", "169.254.1.1", "fd00::1")) {
        check(LanBindPolicy.allows(address, wifi = true, vpn = false)) { address }
        check(!LanBindPolicy.allows(address, wifi = false, vpn = false))
        check(!LanBindPolicy.allows(address, wifi = true, vpn = true))
    }
    for (address in listOf("0.0.0.0", "127.0.0.1", "8.8.8.8", "100.64.0.1", "172.32.0.1", "::", "::1", "fe80::1", "example.test", "010.0.0.1", "10.0.0.1 ")) {
        check(!LanBindPolicy.allows(address, wifi = true, vpn = false)) { address }
    }
    println("LAN lifecycle and bind policy checks passed")
}

private fun bootstrapHandoffTests() {
    val events = mutableListOf<String>()
    val lan = AutoCloseable { events += "LAN close" }
    val transferred = transferLan({ events += "LAN start"; lan },
        { events += "bootstrap stop" }, { events += "bootstrap destroy" })
    check(transferred === lan)
    check(events == listOf("LAN start", "bootstrap stop", "bootstrap destroy"))

    for (failureAt in listOf("stop", "destroy")) {
        events.clear()
        val primary = IllegalStateException("bootstrap $failureAt")
        var observed: Throwable? = null
        try {
            transferLan({ events += "LAN start"; lan }, {
                events += "bootstrap stop"
                if (failureAt == "stop") throw primary
            }, {
                events += "bootstrap destroy"
                if (failureAt == "destroy") throw primary
            })
        } catch (error: Throwable) { observed = error }
        check(observed === primary)
        check(events == listOf("LAN start", "bootstrap stop", "bootstrap destroy", "LAN close")) {
            "LAN ownership leaked after bootstrap $failureAt failure: $events"
        }
    }

    val startupFailure = IllegalStateException("LAN start")
    val stopFailure = IllegalStateException("bootstrap stop")
    val destroyFailure = IllegalStateException("bootstrap destroy")
    var observed: Throwable? = null
    try {
        transferLan<AutoCloseable>({ throw startupFailure }, { throw stopFailure }, { throw destroyFailure })
    } catch (error: Throwable) { observed = error }
    check(observed === startupFailure && startupFailure.suppressed.toList() == listOf(stopFailure, destroyFailure))

    val closeFailure = IllegalStateException("LAN close")
    val primary = IllegalStateException("bootstrap teardown")
    observed = null
    try {
        transferLan({ AutoCloseable { throw closeFailure } }, { throw primary }, {})
    } catch (error: Throwable) { observed = error }
    check(observed === primary && primary.suppressed.single() === closeFailure)
}
