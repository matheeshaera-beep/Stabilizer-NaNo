package com.stabilizernano.app.service

import android.app.KeyguardManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.TrafficStats
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.os.Process
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import com.stabilizernano.app.MainActivity
import com.stabilizernano.app.R
import com.stabilizernano.app.data.StabilizerRepository
import com.stabilizernano.uniffi.StabilizerEngine
import com.stabilizernano.uniffi.StabilizerState
import com.stabilizernano.uniffi.TelemetrySnapshot
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChangedBy
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.util.concurrent.ConcurrentHashMap

class StabilizerForegroundService : Service() {
    private val serviceScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private lateinit var engine: StabilizerEngine
    private lateinit var notificationManager: NotificationManager
    private lateinit var connectivityManager: ConnectivityManager
    private lateinit var tetheringDetector: TetheringDetector

    private val cellularNetworks = ConcurrentHashMap<Network, Pair<Boolean, Boolean>>()
    private var listenersRegistered = false
    private var trafficJob: Job? = null
    private var notificationJob: Job? = null

    private val screenReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            when (intent.action) {
                Intent.ACTION_SCREEN_OFF -> {
                    engine.onScreenStateChanged(isScreenOn = false, isUnlocked = false)
                }
                Intent.ACTION_SCREEN_ON -> {
                    val unlocked = !isKeyguardLocked()
                    engine.onScreenStateChanged(isScreenOn = true, isUnlocked = unlocked)
                }
                Intent.ACTION_USER_PRESENT -> {
                    engine.onScreenStateChanged(isScreenOn = true, isUnlocked = true)
                }
            }
        }
    }

    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            val caps = connectivityManager.getNetworkCapabilities(network)
            updateNetworkEntry(network, caps)
        }

        override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) {
            updateNetworkEntry(network, networkCapabilities)
        }

        override fun onLost(network: Network) {
            cellularNetworks.remove(network)
            dispatchAggregatedNetworkState()
        }
    }

    override fun onCreate() {
        super.onCreate()
        notificationManager = getSystemService(NotificationManager::class.java)
        connectivityManager = getSystemService(ConnectivityManager::class.java)
        tetheringDetector = TetheringDetector(applicationContext)
        engine = StabilizerRepository.ensureInitialized(applicationContext)
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopMonitoring()
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }

        promoteToForeground(StabilizerRepository.telemetry.value)
        startMonitoring()
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        stopMonitoring()
        serviceScope.cancel()
        super.onDestroy()
    }

    private fun startMonitoring() {
        registerSystemListeners()
        syncInitialScreenAndNetworkState()
        engine.start()
        startTrafficSampler()
        observeTelemetryForNotification()
    }

    private fun stopMonitoring() {
        trafficJob?.cancel()
        trafficJob = null
        notificationJob?.cancel()
        notificationJob = null
        unregisterSystemListeners()
        if (::engine.isInitialized) {
            engine.stop()
        }
    }

    private fun registerSystemListeners() {
        if (listenersRegistered) return

        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_OFF)
            addAction(Intent.ACTION_SCREEN_ON)
            addAction(Intent.ACTION_USER_PRESENT)
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(screenReceiver, filter, RECEIVER_NOT_EXPORTED)
        } else {
            @Suppress("DEPRECATION")
            registerReceiver(screenReceiver, filter)
        }

        val request = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_CELLULAR)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        connectivityManager.registerNetworkCallback(request, networkCallback)
        listenersRegistered = true
    }

    private fun unregisterSystemListeners() {
        if (!listenersRegistered) return
        runCatching { unregisterReceiver(screenReceiver) }
        runCatching { connectivityManager.unregisterNetworkCallback(networkCallback) }
        cellularNetworks.clear()
        listenersRegistered = false
    }

    private fun syncInitialScreenAndNetworkState() {
        val powerManager = getSystemService(PowerManager::class.java)
        val isScreenOn = powerManager?.isInteractive ?: true
        val isUnlocked = isScreenOn && !isKeyguardLocked()
        engine.onScreenStateChanged(isScreenOn = isScreenOn, isUnlocked = isUnlocked)

        cellularNetworks.clear()
        @Suppress("DEPRECATION")
        for (network in connectivityManager.allNetworks) {
            val caps = connectivityManager.getNetworkCapabilities(network) ?: continue
            if (caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) &&
                caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            ) {
                val validated = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
                cellularNetworks[network] = true to validated
            }
        }
        dispatchAggregatedNetworkState()

        val initialTether = tetheringDetector.inspect()
        engine.onTetheringStateChanged(initialTether.isTethering)
    }

    private fun updateNetworkEntry(network: Network, caps: NetworkCapabilities?) {
        if (caps == null || !caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) {
            cellularNetworks.remove(network)
        } else {
            val hasInternet = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            val isValidated = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
            cellularNetworks[network] = hasInternet to isValidated
        }
        dispatchAggregatedNetworkState()
    }

    private fun dispatchAggregatedNetworkState() {
        val values = cellularNetworks.values
        val hasInternet = values.any { it.first }
        val isValidated = values.any { it.first && it.second }
        engine.onNetworkCapabilitiesChanged(hasInternet = hasInternet, isValidated = isValidated)
    }

    private fun startTrafficSampler() {
        if (trafficJob?.isActive == true) return
        val uid = Process.myUid()
        trafficJob = serviceScope.launch {
            var prevMobileBytes = readTotalMobileBytes()
            var prevOwnUidBytes = readUidTotalBytes(uid)
            val initialTether = tetheringDetector.inspect()
            var prevTetheringActive = initialTether.isTethering
            var prevTetherBytes = if (initialTether.isTethering) {
                tetheringDetector.readInterfaceTotalBytes(initialTether.activeInterfaces)
            } else {
                null
            }

            while (isActive) {
                delay(1_000L)
                val currMobileBytes = readTotalMobileBytes()
                val currOwnUidBytes = readUidTotalBytes(uid)
                val tetherStatus = tetheringDetector.inspect()

                if (tetherStatus.isTethering != prevTetheringActive) {
                    prevTetheringActive = tetherStatus.isTethering
                    engine.onTetheringStateChanged(tetherStatus.isTethering)
                }

                val currTetherBytes = if (tetherStatus.isTethering) {
                    tetheringDetector.readInterfaceTotalBytes(tetherStatus.activeInterfaces)
                } else {
                    null
                }

                val mobileDelta = if (prevMobileBytes >= 0L && currMobileBytes >= prevMobileBytes) {
                    currMobileBytes - prevMobileBytes
                } else {
                    0L
                }
                val ownUidDelta = if (prevOwnUidBytes >= 0L && currOwnUidBytes >= prevOwnUidBytes) {
                    currOwnUidBytes - prevOwnUidBytes
                } else {
                    0L
                }
                val tetherDelta = if (
                    prevTetherBytes != null &&
                    currTetherBytes != null &&
                    currTetherBytes >= prevTetherBytes
                ) {
                    currTetherBytes - prevTetherBytes
                } else {
                    0L
                }

                prevMobileBytes = currMobileBytes
                prevOwnUidBytes = currOwnUidBytes
                prevTetherBytes = currTetherBytes

                // Subtract our own UID traffic so stabilizer HEAD pings never count as real traffic,
                // while accounting for tethered device traffic even when hardware-offloaded.
                val baseExternalDelta = (mobileDelta - ownUidDelta).coerceAtLeast(0L)
                val externalMobileDelta = if (tetherStatus.isTethering) {
                    maxOf(baseExternalDelta, tetherDelta)
                } else {
                    baseExternalDelta
                }
                engine.onTrafficSample(externalMobileDelta.toULong())
            }
        }
    }

    private fun observeTelemetryForNotification() {
        if (notificationJob?.isActive == true) return
        notificationJob = serviceScope.launch {
            StabilizerRepository.telemetry
                .distinctUntilChangedBy {
                    listOf(it.state, it.lastPingLatencyMs, it.totalPings, it.isTethering)
                }
                .collectLatest { snapshot ->
                    if (snapshot.isRunning) {
                        notificationManager.notify(NOTIFICATION_ID, buildNotification(snapshot))
                    }
                }
        }
    }

    private fun promoteToForeground(snapshot: TelemetrySnapshot) {
        val notification = buildNotification(snapshot)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            ServiceCompat.startForeground(
                this,
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
            )
        } else {
            ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, 0)
        }
    }

    private fun buildNotification(snapshot: TelemetrySnapshot): Notification {
        val openAppIntent = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            },
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

        val stopIntent = PendingIntent.getService(
            this,
            1,
            Intent(this, StabilizerForegroundService::class.java).apply {
                action = ACTION_STOP
            },
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

        val stateLabel = when (snapshot.state) {
            StabilizerState.ACTIVE -> "Active (0 pings)"
            StabilizerState.IDLE_PING -> "IdlePing"
            StabilizerState.WARMUP -> "Warmup"
            StabilizerState.SLEEPING -> "Sleeping (0 pings)"
            StabilizerState.RECOVERING -> "Recovering"
            StabilizerState.STOPPED -> "Starting..."
        }
        val tetherPart = if (snapshot.isTethering) " • Tethering" else ""
        val latencyPart = snapshot.lastPingLatencyMs?.let { " • ${it}ms" } ?: ""
        val contentText = "$stateLabel$tetherPart$latencyPart • ${snapshot.totalPings} pings"

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_stabilizer_tile)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(contentText)
            .setOngoing(true)
            .setSilent(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setContentIntent(openAppIntent)
            .addAction(0, "Stop", stopIntent)
            .build()
    }

    private fun createNotificationChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.notification_channel_name),
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            description = getString(R.string.notification_channel_desc)
            setShowBadge(false)
            enableLights(false)
            enableVibration(false)
        }
        notificationManager.createNotificationChannel(channel)
    }

    private fun isKeyguardLocked(): Boolean {
        val km = getSystemService(KeyguardManager::class.java)
        return km?.isKeyguardLocked ?: false
    }

    private fun readTotalMobileBytes(): Long {
        val rx = TrafficStats.getMobileRxBytes()
        val tx = TrafficStats.getMobileTxBytes()
        if (rx == TrafficStats.UNSUPPORTED.toLong() || tx == TrafficStats.UNSUPPORTED.toLong()) {
            return -1L
        }
        return rx + tx
    }

    private fun readUidTotalBytes(uid: Int): Long {
        val rx = TrafficStats.getUidRxBytes(uid)
        val tx = TrafficStats.getUidTxBytes(uid)
        if (rx == TrafficStats.UNSUPPORTED.toLong() || tx == TrafficStats.UNSUPPORTED.toLong()) {
            return -1L
        }
        return rx + tx
    }

    companion object {
        const val ACTION_START = "com.stabilizernano.app.ACTION_START"
        const val ACTION_STOP = "com.stabilizernano.app.ACTION_STOP"
        private const val CHANNEL_ID = "stabilizer_status_channel"
        private const val NOTIFICATION_ID = 1001
    }
}
