package com.stabilizernano.app.service

import android.content.Context
import android.net.ConnectivityManager
import android.net.wifi.WifiManager
import java.io.File
import java.net.NetworkInterface

data class TetheringStatus(
    val isTethering: Boolean,
    val activeInterfaces: Set<String>
)

class TetheringDetector(context: Context) {
    private val appContext = context.applicationContext
    private val connectivityManager =
        appContext.getSystemService(ConnectivityManager::class.java)
    private val wifiManager =
        appContext.getSystemService(WifiManager::class.java)

    fun inspect(): TetheringStatus {
        val interfaces = linkedSetOf<String>()

        // 1. Query ConnectivityManager.getTetheredIfaces() safely via reflection.
        runCatching {
            val method = connectivityManager?.javaClass?.getMethod("getTetheredIfaces")
            @Suppress("UNCHECKED_CAST")
            val tethered = method?.invoke(connectivityManager) as? Array<String>
            tethered?.filterTo(interfaces) { it.isNotBlank() }
        }

        // 2. Check Wi-Fi Hotspot (SoftAP) state safely via reflection.
        val wifiApEnabled = isWifiApEnabled()

        // 3. Inspect active kernel network interfaces for USB, Hotspot, and Bluetooth PAN tethering.
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

        val isTethering = wifiApEnabled || interfaces.isNotEmpty()
        return TetheringStatus(
            isTethering = isTethering,
            activeInterfaces = interfaces
        )
    }

    fun readInterfaceTotalBytes(interfaces: Set<String>): Long? {
        if (interfaces.isEmpty()) return null
        var total = 0L
        var anyReadable = false
        for (iface in interfaces) {
            val rx = readSysCounter(iface, "rx_bytes")
            val tx = readSysCounter(iface, "tx_bytes")
            if (rx != null && tx != null) {
                total += rx + tx
                anyReadable = true
            }
        }
        return if (anyReadable) total else null
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

    private fun isKnownTetherInterfaceName(name: String): Boolean {
        return name.startsWith("rndis") ||
            name.startsWith("ncm") ||
            name.startsWith("usb") ||
            name.startsWith("ap") ||
            name.startsWith("swlan") ||
            name.startsWith("softap") ||
            name == "wlan1" ||
            name == "wlan2" ||
            name.startsWith("bt-pan") ||
            name.startsWith("bnep")
    }

    companion object {
        private const val WIFI_AP_STATE_ENABLED = 13
    }
}
