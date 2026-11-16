package com.stabilizernano.app.service

import android.content.Context
import android.net.ConnectivityManager
import android.net.wifi.SoftApConfiguration
import android.net.wifi.WifiClient
import android.net.wifi.WifiManager
import android.os.Build
import java.io.File
import java.net.NetworkInterface
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
    private var softApCallbackRegistered = false
    private val callbackExecutor = Executors.newSingleThreadExecutor()

    private val softApCallback: Any? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
        object : WifiManager.SoftApCallback {
            override fun onStateChanged(state: Int, failureReason: Int) {
                if (state != WIFI_AP_STATE_ENABLED) {
                    softApClientCount.set(0)
                }
            }

            override fun onConnectedClientsChanged(clients: List<WifiClient>) {
                softApClientCount.set(clients.size)
            }

            override fun onConnectedClientsChanged(info: android.net.wifi.SoftApInfo, clients: List<WifiClient>) {
                softApClientCount.set(clients.size)
            }
        }
    } else {
        null
    }

    fun registerCallbacks() {
        if (softApCallbackRegistered || Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return
        val manager = wifiManager ?: return
        val cb = softApCallback as? WifiManager.SoftApCallback ?: return
        runCatching {
            manager.registerSoftApCallback(callbackExecutor, cb)
            softApCallbackRegistered = true
        }
    }

    fun unregisterCallbacks() {
        if (!softApCallbackRegistered || Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return
        val manager = wifiManager ?: return
        val cb = softApCallback as? WifiManager.SoftApCallback ?: return
        runCatching {
            manager.unregisterSoftApCallback(cb)
        }
        softApCallbackRegistered = false
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
        // Fallback: inspect kernel ARP table entries associated with hotspot interfaces.
        return runCatching {
            val arpFile = File("/proc/net/arp")
            if (!arpFile.canRead()) return@runCatching 0
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
