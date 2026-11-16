package com.stabilizernano.app.service

import android.Manifest
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
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioManager
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.TrafficStats
import android.os.BatteryManager
import android.os.Build
import android.os.IBinder
import android.os.ParcelFileDescriptor
import android.os.PowerManager
import android.os.Process
import android.os.SystemClock
import android.telephony.SubscriptionManager
import android.telephony.TelephonyCallback
import android.telephony.TelephonyDisplayInfo
import android.telephony.TelephonyManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import com.stabilizernano.app.MainActivity
import com.stabilizernano.app.R
import com.stabilizernano.app.data.CellularNetworkDelegate
import com.stabilizernano.app.data.StabilizerRepository
import com.stabilizernano.uniffi.RuntimeContext
import com.stabilizernano.uniffi.StabilizerEngine
import com.stabilizernano.uniffi.StabilizerState
import com.stabilizernano.uniffi.TelemetrySnapshot
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChangedBy
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

class StabilizerForegroundService : Service(), CellularNetworkDelegate {
    private val serviceScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private lateinit var engine: StabilizerEngine
    private lateinit var notificationManager: NotificationManager
    private lateinit var connectivityManager: ConnectivityManager
    private lateinit var tetheringDetector: TetheringDetector

    private val cellularNetworks = ConcurrentHashMap<Network, Pair<Boolean, Boolean>>()
    private val preferredCellularNetwork = AtomicReference<Network?>(null)
    private val isWifiDefaultRoute = AtomicBoolean(false)
    private val boostActiveFlag = AtomicBoolean(false)
    private val samplerWakeSignal = Channel<Unit>(Channel.CONFLATED)

    private var listenersRegistered = false
    private var trafficJob: Job? = null
    private var notificationJob: Job? = null
    private var lastDispatchedContext: RuntimeContext? = null
    private var lastDispatchedNetCaps: Pair<Boolean, Boolean>? = null
    private var lastInterfaceSignature: String? = null
    private var lastDisplayOverrideNetwork: Int? = null
    private var telephonyCallback: Any? = null
    private var subscriptionListener: SubscriptionManager.OnSubscriptionsChangedListener? = null

    private val screenReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            when (intent.action) {
                Intent.ACTION_SCREEN_OFF -> {
                    engine.onScreenStateChanged(isScreenOn = false, isUnlocked = false)
                }
                Intent.ACTION_SCREEN_ON -> {
                    val unlocked = !isKeyguardLocked()
                    engine.onScreenStateChanged(isScreenOn = true, isUnlocked = unlocked)
                    samplerWakeSignal.trySend(Unit)
                }
                Intent.ACTION_USER_PRESENT -> {
                    engine.onScreenStateChanged(isScreenOn = true, isUnlocked = true)
                    samplerWakeSignal.trySend(Unit)
                }
                PowerManager.ACTION_DEVICE_IDLE_MODE_CHANGED,
                PowerManager.ACTION_POWER_SAVE_MODE_CHANGED,
                Intent.ACTION_BATTERY_CHANGED,
                Intent.ACTION_POWER_CONNECTED,
                Intent.ACTION_POWER_DISCONNECTED -> {
                    pushContextIfChanged(tetheringDetector.inspect())
                }
            }
        }
    }

    private val cellularCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            val caps = connectivityManager.getNetworkCapabilities(network)
            updateNetworkEntry(network, caps)
        }

        override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) {
            updateNetworkEntry(network, networkCapabilities)
        }

        override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) {
            val iface = linkProperties.interfaceName ?: return
            val prev = lastInterfaceSignature
            lastInterfaceSignature = iface
            if (prev != null && prev != iface) {
                engine.onRadioTopologyChanged("Link change ($iface)")
            }
        }

        override fun onLost(network: Network) {
            cellularNetworks.remove(network)
            if (preferredCellularNetwork.get() == network) {
                preferredCellularNetwork.set(null)
            }
            dispatchAggregatedNetworkState()
        }
    }

    private val defaultNetworkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            refreshDefaultRouteWifiState(network)
        }

        override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
            refreshDefaultRouteWifiState(network, caps)
        }

        override fun onLost(network: Network) {
            isWifiDefaultRoute.set(false)
            pushContextIfChanged(tetheringDetector.inspect())
        }
    }

    override fun onCreate() {
        super.onCreate()
        notificationManager = getSystemService(NotificationManager::class.java)
        connectivityManager = getSystemService(ConnectivityManager::class.java)
        tetheringDetector = TetheringDetector(applicationContext)
        engine = StabilizerRepository.ensureInitialized(applicationContext)
        StabilizerRepository.registerNetworkDelegate(this)
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
        StabilizerRepository.registerNetworkDelegate(null)
        serviceScope.cancel()
        super.onDestroy()
    }

    override fun bindCellularSocketFd(fd: Int): Boolean {
        val cellularNet = preferredCellularNetwork.get() ?: return false
        return runCatching {
            ParcelFileDescriptor.fromFd(fd).use { dupFd ->
                cellularNet.bindSocket(dupFd.fileDescriptor)
            }
            true
        }.getOrDefault(false)
    }

    override fun requestAndroidRevalidation() {
        val cellularNet = preferredCellularNetwork.get() ?: return
        runCatching {
            @Suppress("DEPRECATION")
            connectivityManager.reportNetworkConnectivity(cellularNet, false)
        }
    }

    override fun onBoostStateUpdated(active: Boolean) {
        boostActiveFlag.set(active)
        if (::tetheringDetector.isInitialized) {
            pushContextIfChanged(tetheringDetector.inspect())
        }
        if (active) {
            samplerWakeSignal.trySend(Unit)
        }
    }

    override fun wakeSampler() {
        samplerWakeSignal.trySend(Unit)
    }

    private fun startMonitoring() {
        boostActiveFlag.set(StabilizerRepository.boostRemainingMinutes.value > 0)
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
            addAction(PowerManager.ACTION_DEVICE_IDLE_MODE_CHANGED)
            addAction(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED)
            addAction(Intent.ACTION_POWER_CONNECTED)
            addAction(Intent.ACTION_POWER_DISCONNECTED)
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(screenReceiver, filter, RECEIVER_NOT_EXPORTED)
        } else {
            @Suppress("DEPRECATION")
            registerReceiver(screenReceiver, filter)
        }

        val cellularReq = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_CELLULAR)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        connectivityManager.registerNetworkCallback(cellularReq, cellularCallback)
        runCatching {
            connectivityManager.registerDefaultNetworkCallback(defaultNetworkCallback)
        }

        tetheringDetector.registerCallbacks()
        registerRadioEventListeners()
        listenersRegistered = true
    }

    private fun unregisterSystemListeners() {
        if (!listenersRegistered) return
        runCatching { unregisterReceiver(screenReceiver) }
        runCatching { connectivityManager.unregisterNetworkCallback(cellularCallback) }
        runCatching { connectivityManager.unregisterNetworkCallback(defaultNetworkCallback) }
        tetheringDetector.unregisterCallbacks()
        unregisterRadioEventListeners()
        cellularNetworks.clear()
        preferredCellularNetwork.set(null)
        lastDispatchedContext = null
        lastDispatchedNetCaps = null
        listenersRegistered = false
    }

    private fun registerRadioEventListeners() {
        // Data SIM change listener (no extra runtime permissions required).
        runCatching {
            val subManager = getSystemService(SubscriptionManager::class.java)
            val listener = object : SubscriptionManager.OnSubscriptionsChangedListener() {
                private var lastDataSubId: Int = SubscriptionManager.getDefaultDataSubscriptionId()

                override fun onSubscriptionsChanged() {
                    val currentSub = SubscriptionManager.getDefaultDataSubscriptionId()
                    if (currentSub != lastDataSubId) {
                        lastDataSubId = currentSub
                        engine.onRadioTopologyChanged("Data SIM switched")
                    }
                }
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                subManager?.addOnSubscriptionsChangedListener(
                    Executors.newSingleThreadExecutor(),
                    listener
                )
            } else {
                @Suppress("DEPRECATION")
                subManager?.addOnSubscriptionsChangedListener(listener)
            }
            subscriptionListener = listener
        }

        // Optional RAT LTE<->5G listener when READ_PHONE_STATE is already granted on API 31+.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val hasPhoneState = ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.READ_PHONE_STATE
            ) == PackageManager.PERMISSION_GRANTED
            if (hasPhoneState) {
                runCatching {
                    val telephony = getSystemService(TelephonyManager::class.java) ?: return
                    val cb = object : TelephonyCallback(), TelephonyCallback.DisplayInfoListener {
                        override fun onDisplayInfoChanged(info: TelephonyDisplayInfo) {
                            val ovr = info.overrideNetworkType
                            val prev = lastDisplayOverrideNetwork
                            lastDisplayOverrideNetwork = ovr
                            if (prev != null && prev != ovr) {
                                engine.onRadioTopologyChanged("RAT transition")
                            }
                        }
                    }
                    telephony.registerTelephonyCallback(Executors.newSingleThreadExecutor(), cb)
                    telephonyCallback = cb
                }
            }
        }
    }

    private fun unregisterRadioEventListeners() {
        subscriptionListener?.let { listener ->
            runCatching {
                getSystemService(SubscriptionManager::class.java)
                    ?.removeOnSubscriptionsChangedListener(listener)
            }
        }
        subscriptionListener = null

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            (telephonyCallback as? TelephonyCallback)?.let { cb ->
                runCatching {
                    getSystemService(TelephonyManager::class.java)?.unregisterTelephonyCallback(cb)
                }
            }
        }
        telephonyCallback = null
    }

    private fun syncInitialScreenAndNetworkState() {
        val powerManager = getSystemService(PowerManager::class.java)
        val isScreenOn = powerManager?.isInteractive ?: true
        val isUnlocked = isScreenOn && !isKeyguardLocked()
        engine.onScreenStateChanged(isScreenOn = isScreenOn, isUnlocked = isUnlocked)

        connectivityManager.activeNetwork?.let { active ->
            refreshDefaultRouteWifiState(active)
        }

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
        pushContextIfChanged(tetheringDetector.inspect())
    }

    private fun refreshDefaultRouteWifiState(
        network: Network,
        capsOverride: NetworkCapabilities? = null
    ) {
        val caps = capsOverride ?: connectivityManager.getNetworkCapabilities(network)
        val wifiDefault = caps != null &&
            caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) &&
            !caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)
        isWifiDefaultRoute.set(wifiDefault)
        pushContextIfChanged(tetheringDetector.inspect())
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
        val entries = cellularNetworks.entries.toList()
        val bestNetwork = entries.firstOrNull { it.value.first && it.value.second }?.key
            ?: entries.firstOrNull { it.value.first }?.key
        preferredCellularNetwork.set(bestNetwork)

        val hasInternet = entries.any { it.value.first }
        val isValidated = entries.any { it.value.first && it.value.second }
        val netCaps = hasInternet to isValidated
        if (netCaps != lastDispatchedNetCaps) {
            lastDispatchedNetCaps = netCaps
            engine.onNetworkCapabilitiesChanged(hasInternet = hasInternet, isValidated = isValidated)
        }
    }

    private fun pushContextIfChanged(tetherStatus: TetheringStatus) {
        val ctx = buildRuntimeContext(tetherStatus)
        if (ctx != lastDispatchedContext) {
            lastDispatchedContext = ctx
            engine.onContextChanged(ctx)
        }
    }

    private fun buildRuntimeContext(tetherStatus: TetheringStatus): RuntimeContext {
        val powerManager = getSystemService(PowerManager::class.java)
        val audioManager = getSystemService(AudioManager::class.java)
        val batteryStatus = registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))

        val level = batteryStatus?.getIntExtra(BatteryManager.EXTRA_LEVEL, -1) ?: -1
        val scale = batteryStatus?.getIntExtra(BatteryManager.EXTRA_SCALE, 100) ?: 100
        val batteryPct = if (level >= 0 && scale > 0) (level * 100) / scale else 100
        val plugged = (batteryStatus?.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) ?: 0) != 0

        val isPowerSave = powerManager?.isPowerSaveMode ?: false
        val isDoze = powerManager?.isDeviceIdleMode ?: false
        val audioMode = audioManager?.mode ?: AudioManager.MODE_NORMAL
        val inCall = audioMode == AudioManager.MODE_IN_CALL ||
            audioMode == AudioManager.MODE_IN_COMMUNICATION

        return RuntimeContext(
            isWifiDefault = isWifiDefaultRoute.get(),
            inCall = inCall,
            hotspotClients = tetherStatus.hotspotClients.toUInt(),
            isUsbOrBtTethering = tetherStatus.isUsbOrBtTethering,
            isCharging = plugged,
            lowBatteryOrSaver = isPowerSave || batteryPct < 20,
            isDoze = isDoze,
            boostActive = boostActiveFlag.get(),
            liveAppActive = false
        )
    }

    private fun startTrafficSampler() {
        if (trafficJob?.isActive == true) return
        val uid = Process.myUid()
        trafficJob = serviceScope.launch {
            var prevMobileRx = readMobileRxBytes()
            var prevMobileTx = readMobileTxBytes()
            var prevOwnRx = readUidRxBytes(uid)
            var prevOwnTx = readUidTxBytes(uid)

            val initialTether = tetheringDetector.inspect()
            var prevTetherTraffic = if (initialTether.isTetheringActive) {
                tetheringDetector.readInterfaceTraffic(initialTether.activeInterfaces)
            } else {
                null
            }

            // Drain any stale wake signal queued before sampler start.
            samplerWakeSignal.tryReceive()
            var lastSampleAtMs = SystemClock.elapsedRealtime()

            while (isActive) {
                val sampleMs = StabilizerRepository.telemetry.value.recommendedSampleMs
                    .toLong()
                    .coerceIn(1_000L, 10_000L)
                withTimeoutOrNull(sampleMs) {
                    samplerWakeSignal.receive()
                }
                val sinceLastMs = SystemClock.elapsedRealtime() - lastSampleAtMs
                if (sinceLastMs < 1_000L) {
                    delay(1_000L - sinceLastMs)
                }

                val nowMs = SystemClock.elapsedRealtime()
                val elapsedSecs = ((nowMs - lastSampleAtMs + 500L) / 1_000L)
                    .coerceAtLeast(1L)
                    .toULong()
                lastSampleAtMs = nowMs
                val currMobileRx = readMobileRxBytes()
                val currMobileTx = readMobileTxBytes()
                val currOwnRx = readUidRxBytes(uid)
                val currOwnTx = readUidTxBytes(uid)
                val tetherStatus = tetheringDetector.inspect()

                pushContextIfChanged(tetherStatus)

                val currTetherTraffic = if (tetherStatus.isTetheringActive) {
                    tetheringDetector.readInterfaceTraffic(tetherStatus.activeInterfaces)
                } else {
                    null
                }

                val mobileRxDelta = nonNegativeDelta(prevMobileRx, currMobileRx)
                val mobileTxDelta = nonNegativeDelta(prevMobileTx, currMobileTx)
                val ownRxDelta = nonNegativeDelta(prevOwnRx, currOwnRx)
                val ownTxDelta = nonNegativeDelta(prevOwnTx, currOwnTx)

                val tetherRxDelta = if (prevTetherTraffic != null && currTetherTraffic != null) {
                    nonNegativeDelta(prevTetherTraffic.rxBytes, currTetherTraffic.rxBytes)
                } else {
                    0L
                }
                val tetherTxDelta = if (prevTetherTraffic != null && currTetherTraffic != null) {
                    nonNegativeDelta(prevTetherTraffic.txBytes, currTetherTraffic.txBytes)
                } else {
                    0L
                }

                prevMobileRx = currMobileRx
                prevMobileTx = currMobileTx
                prevOwnRx = currOwnRx
                prevOwnTx = currOwnTx
                prevTetherTraffic = currTetherTraffic

                val extRx = (mobileRxDelta - ownRxDelta).coerceAtLeast(0L)
                val extTx = (mobileTxDelta - ownTxDelta).coerceAtLeast(0L)

                // Hardware offload / bridged interfaces swap direction cleanly on upstream vs downstream,
                // so take max against tether deltas when clients are active.
                val finalRx = if (tetherStatus.isTetheringActive) maxOf(extRx, tetherTxDelta) else extRx
                val finalTx = if (tetherStatus.isTetheringActive) maxOf(extTx, tetherRxDelta) else extTx

                engine.onSamplingTick(
                    elapsedSecs = elapsedSecs,
                    rxBytesDelta = finalRx.toULong(),
                    txBytesDelta = finalTx.toULong()
                )
            }
        }
    }

    private fun observeTelemetryForNotification() {
        if (notificationJob?.isActive == true) return
        notificationJob = serviceScope.launch {
            StabilizerRepository.telemetry
                .distinctUntilChangedBy {
                    listOf(it.state, it.stateReason, it.lastPingLatencyMs, it.avoidedFixed4sPings)
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

        val latencyPart = snapshot.lastPingLatencyMs?.let { " • ${it}ms" } ?: ""
        val contentText = "${snapshot.stateReason}$latencyPart • ${snapshot.avoidedFixed4sPings} saved"

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

    private fun nonNegativeDelta(prev: Long, curr: Long): Long {
        return if (prev >= 0L && curr >= prev) curr - prev else 0L
    }

    private fun readMobileRxBytes(): Long =
        TrafficStats.getMobileRxBytes().takeUnless { it == TrafficStats.UNSUPPORTED.toLong() } ?: -1L

    private fun readMobileTxBytes(): Long =
        TrafficStats.getMobileTxBytes().takeUnless { it == TrafficStats.UNSUPPORTED.toLong() } ?: -1L

    private fun readUidRxBytes(uid: Int): Long =
        TrafficStats.getUidRxBytes(uid).takeUnless { it == TrafficStats.UNSUPPORTED.toLong() } ?: -1L

    private fun readUidTxBytes(uid: Int): Long =
        TrafficStats.getUidTxBytes(uid).takeUnless { it == TrafficStats.UNSUPPORTED.toLong() } ?: -1L

    companion object {
        const val ACTION_START = "com.stabilizernano.app.ACTION_START"
        const val ACTION_STOP = "com.stabilizernano.app.ACTION_STOP"
        private const val CHANNEL_ID = "stabilizer_status_channel"
        private const val NOTIFICATION_ID = 1001
    }
}
