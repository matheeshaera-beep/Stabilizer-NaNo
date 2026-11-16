package com.stabilizernano.app.service

import android.content.Context
import android.net.ConnectivityManager
import android.net.wifi.WifiManager
import java.io.File
import java.lang.reflect.Proxy
import java.net.NetworkInterface
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger

data class InterfaceTraffic(
    val rxBytes: Long,
    val txBytes: Long
)

data class TetheringStatus(
    val isTetheringActive: Boolean,
    val hotspotClients: Int,
    val isUsbOrBtTethering: Boolean,
    val activeInterfaces: Set<String>
)

class TetheringDetector(context: Context) {
    private val appContext = context.applicationContext
    private val connectivityManager =
        appContext.getSystemService(ConnectivityManager::class.java)
    private val wifiManager =
        appContext.getSystemService(WifiManager::class.java)

    private val softApClientCount = AtomicInteger(-1)
    private var softApCallbackProxy: Any? = null
    private val callbackExecutor = Executors.newSingleThreadExecutor()

    fun registerCallbacks() {
        if (softApCallbackProxy != null) return
        val manager = wifiManager ?: return
        runCatching {
            val cbInterface = Class.forName("android.net.wifi.WifiManager\$SoftApCallback")
            val proxy = Proxy.newProxyInstance(
                cbInterface.classLoader,
                arrayOf(cbInterface)
            ) { _, method, args ->
                when (method.name) {
                    "onConnectedClientsChanged" -> {
                        val clients = args?.lastOrNull() as? Collection<*>
                        if (clients != null) {
                            softApClientCount.set(clients.size)
                        }
                    }
                    "onStateChanged" -> {
                        val state = args?.firstOrNull() as? Int
                        if (state != WIFI_AP_STATE_ENABLED) {
                            softApClientCount.set(0)
                        }
                    }
                }
                null
            }
            val regMethod = manager.javaClass.getMethod(
                "registerSoftApCallback",
                Executor::class.java,
                cbInterface
            )
            regMethod.invoke(manager, callbackExecutor, proxy)
            softApCallbackProxy = proxy
        }
    }

    fun unregisterCallbacks() {
        val proxy = softApCallbackProxy ?: return
        val manager = wifiManager ?: return
        runCatching {
            val cbInterface = Class.forName("android.net.wifi.WifiManager\$SoftApCallback")
            val unregMethod = manager.javaClass.getMethod("unregisterSoftApCallback", cbInterface)
            unregMethod.invoke(manager, proxy)
        }
        softApCallbackProxy = null
        softApClientCount.set(-1)
    }

    fun inspect(): TetheringStatus {
        val interfaces = linkedSetOf<String>()
        runCatching {
            val method = connectivityManager?.javaClass?.getMethod("getTetheredIfaces")
            @Suppress("UNCHECKED_CAST")
            val tethered = method?.invoke(connectivityManager) as? Array<String>
            tethered?.filterTo(interfaces) { it.isNotBlank() }
        }

        val wifiApEnabled = isWifiApEnabled()
        runCatching {
            val enumIfaces = NetworkInterface.getNetworkInterfaces()
            while (enumIfaces != null && enumIfaces.hasMoreElements()) {
                val iface = enumIfaces.nextElement()
                val name = iface.name ?: continue
                if (!iface.isUp || iface.isLoopback) continue
                if (isKnownTetherInterfaceName(name)) {
                    interfaces.add(name)
                }
            }
        }

        val isUsbOrBt = interfaces.any { isUsbOrBtInterface(it) }
        val callbackClients = softApClientCount.get()
        val hotspotClients = when {
            !wifiApEnabled && interfaces.none { isHotspotInterface(it) } -> 0
            callbackClients >= 0 -> callbackClients
            else -> countNeighborClients(interfaces)
        }

        val active = hotspotClients > 0 || isUsbOrBt
        return TetheringStatus(
            isTetheringActive = active,
            hotspotClients = hotspotClients,
            isUsbOrBtTethering = isUsbOrBt,
            activeInterfaces = interfaces
        )
    }

    fun readInterfaceTraffic(interfaces: Set<String>): InterfaceTraffic? {
        if (interfaces.isEmpty()) return null
        var totalRx = 0L
        var totalTx = 0L
        var anyReadable = false
        for (iface in interfaces) {
            val rx = readSysCounter(iface, "rx_bytes")
            val tx = readSysCounter(iface, "tx_bytes")
            if (rx != null && tx != null) {
                totalRx += rx
                totalTx += tx
                anyReadable = true
            }
        }
        return if (anyReadable) InterfaceTraffic(rxBytes = totalRx, txBytes = totalTx) else null
    }

    private fun countNeighborClients(interfaces: Set<String>): Int {
        if (interfaces.isEmpty()) return 0

        // Strategy 1: /proc/net/arp (fast, zero subprocess overhead where readable).
        val arpCount = runCatching {
            val arpFile = File("/proc/net/arp")
            if (!arpFile.canRead()) return@runCatching -1
            arpFile.readLines()
                .drop(1)
                .count { line ->
                    val cols = line.trim().split(Regex("\\s+"))
                    if (cols.size < 6) return@count false
                    val flags = cols[2]
                    val hwAddr = cols[3]
                    val iface = cols[5]
                    flags != "0x0" &&
                        hwAddr != "00:00:00:00:00:00" &&
                        (interfaces.contains(iface) || isHotspotInterface(iface))
                }
        }.getOrDefault(-1)
        if (arpCount >= 0) return arpCount

        // Strategy 2: `ip -4 neigh show` fallback on Android 10+ where /proc/net/arp is restricted.
        return runCatching {
            val process = ProcessBuilder("ip", "-4", "neigh", "show")
                .redirectErrorStream(true)
                .start()
            val lines = process.inputStream.bufferedReader().use { it.readLines() }
            process.waitFor()
            lines.count { line ->
                val activeState = line.contains("REACHABLE") ||
                    line.contains("STALE") ||
                    line.contains("DELAY")
                val matchesHotspot = interfaces.any { iface -> line.contains(" dev $iface ") }
                activeState && matchesHotspot
            }
        }.getOrDefault(0)
    }

    private fun readSysCounter(iface: String, counter: String): Long? {
        return runCatching {
            val file = File("/sys/class/net/$iface/statistics/$counter")
            if (!file.canRead()) return@runCatching null
            file.readText().trim().toLongOrNull()
        }.getOrNull()
    }

    private fun isWifiApEnabled(): Boolean {
        val manager = wifiManager ?: return false
        val byBool = runCatching {
            val method = manager.javaClass.getMethod("isWifiApEnabled")
            method.invoke(manager) as? Boolean
        }.getOrNull()
        if (byBool == true) return true

        val state = runCatching {
            val method = manager.javaClass.getMethod("getWifiApState")
            method.invoke(manager) as? Int
        }.getOrNull()
        return state == WIFI_AP_STATE_ENABLED
    }

    private fun isUsbOrBtInterface(name: String): Boolean {
        return name.startsWith("rndis") ||
            name.startsWith("ncm") ||
            name.startsWith("usb") ||
            name.startsWith("bt-pan") ||
            name.startsWith("bnep")
    }

    private fun isHotspotInterface(name: String): Boolean {
        return name.startsWith("ap") ||
            name.startsWith("swlan") ||
            name.startsWith("softap") ||
            name == "wlan1" ||
            name == "wlan2"
    }

    private fun isKnownTetherInterfaceName(name: String): Boolean {
        return isUsbOrBtInterface(name) || isHotspotInterface(name)
    }

    companion object {
        private const val WIFI_AP_STATE_ENABLED = 13
    }
}
