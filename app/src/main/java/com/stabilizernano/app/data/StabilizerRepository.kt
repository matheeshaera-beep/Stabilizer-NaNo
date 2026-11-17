package com.stabilizernano.app.data

import android.content.Context
import android.content.Intent
import androidx.core.content.ContextCompat
import com.stabilizernano.app.service.RootRecoveryHelper
import com.stabilizernano.app.service.StabilizerForegroundService
import com.stabilizernano.app.service.StabilizerTileService
import com.stabilizernano.uniffi.LogEntry
import com.stabilizernano.uniffi.LogLevel
import com.stabilizernano.uniffi.StabilizerConfig
import com.stabilizernano.uniffi.StabilizerEngine
import com.stabilizernano.uniffi.StabilizerException
import com.stabilizernano.uniffi.StabilizerObserver
import com.stabilizernano.uniffi.StabilizerState
import com.stabilizernano.uniffi.TelemetrySnapshot
import com.stabilizernano.uniffi.defaultStabilizerConfig
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

interface CellularNetworkDelegate {
    fun bindCellularSocketFd(fd: Int): Boolean
    fun requestAndroidRevalidation()
    fun onBoostStateUpdated(active: Boolean)
    fun wakeSampler()
    fun onRootRecoveryTriggered()
}

object StabilizerRepository : StabilizerObserver {
    private const val MAX_LOCAL_LOGS = 50

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val initLock = Any()

    @Volatile
    private var appContext: Context? = null

    @Volatile
    private var preferences: StabilizerPreferences? = null

    @Volatile
    private var engine: StabilizerEngine? = null

    @Volatile
    private var networkDelegate: CellularNetworkDelegate? = null

    @Volatile
    private var lastNotifiedTileKey: Triple<Boolean, StabilizerState, Boolean>? = null

    private var boostTimerJob: Job? = null

    private val _config = MutableStateFlow(defaultStabilizerConfig())
    val config: StateFlow<StabilizerConfig> = _config.asStateFlow()

    private val _telemetry = MutableStateFlow(
        TelemetrySnapshot(
            state = StabilizerState.STOPPED,
            stateReason = "Stopped",
            isRunning = false,
            recommendedSampleMs = 5_000UL,
            lastPingLatencyMs = null,
            totalPings = 0UL,
            successfulPings = 0UL,
            failedPings = 0UL,
            totalProbeBytes = 0UL,
            avoidedFixed4sPings = 0UL,
            stallsDetected = 0UL,
            stallsHealed = 0UL,
            recoveriesTriggered = 0UL,
            lastRxBytesPerSec = 0UL,
            lastTxBytesPerSec = 0UL,
            hasInternetCapability = false,
            isValidated = false,
            isScreenOn = true,
            isWifiDefault = false,
            isTethering = false,
            hotspotClients = 0U,
            boostActive = false,
            isCalibrating = false,
            logs = emptyList()
        )
    )
    val telemetry: StateFlow<TelemetrySnapshot> = _telemetry.asStateFlow()

    private val _localLogs = MutableStateFlow<List<LogEntry>>(emptyList())
    val localLogs: StateFlow<List<LogEntry>> = _localLogs.asStateFlow()

    private val _errorBanner = MutableStateFlow<String?>(null)
    val errorBanner: StateFlow<String?> = _errorBanner.asStateFlow()

    private val _boostRemainingMinutes = MutableStateFlow(0)
    val boostRemainingMinutes: StateFlow<Int> = _boostRemainingMinutes.asStateFlow()

    private val _rootTestStatus = MutableStateFlow<String?>(null)
    val rootTestStatus: StateFlow<String?> = _rootTestStatus.asStateFlow()

    private val _calibrationSummary = MutableStateFlow<String?>(null)
    val calibrationSummary: StateFlow<String?> = _calibrationSummary.asStateFlow()

    fun ensureInitialized(context: Context): StabilizerEngine {
        val existing = engine
        if (existing != null) return existing

        synchronized(initLock) {
            engine?.let { return it }
            val appCtx = context.applicationContext
            appContext = appCtx
            val prefs = StabilizerPreferences(appCtx)
            preferences = prefs
            val loadedConfig = prefs.loadConfig()
            _config.value = loadedConfig

            val created = StabilizerEngine(loadedConfig, this)
            engine = created
            _telemetry.value = created.getTelemetry()
            return created
        }
    }

    fun registerNetworkDelegate(delegate: CellularNetworkDelegate?) {
        networkDelegate = delegate
    }

    fun startService(context: Context) {
        ensureInitialized(context)
        val intent = Intent(context, StabilizerForegroundService::class.java).apply {
            action = StabilizerForegroundService.ACTION_START
        }
        ContextCompat.startForegroundService(context, intent)
    }

    fun stopService(context: Context) {
        cancelBoost()
        val intent = Intent(context, StabilizerForegroundService::class.java).apply {
            action = StabilizerForegroundService.ACTION_STOP
        }
        context.startService(intent)
    }

    fun toggleService(context: Context) {
        if (_telemetry.value.isRunning) {
            stopService(context)
        } else {
            startService(context)
        }
    }

    fun toggleBoost(context: Context, minutes: Int = 30) {
        if (_boostRemainingMinutes.value > 0) {
            cancelBoost()
        } else {
            if (!_telemetry.value.isRunning) {
                startService(context)
            }
            startBoost(minutes)
        }
    }

    fun startBoost(minutes: Int = 30) {
        boostTimerJob?.cancel()
        _boostRemainingMinutes.value = minutes
        networkDelegate?.onBoostStateUpdated(true)
        appendLocalLog(LogLevel.INFO, "Live Boost enabled for ${minutes}m")

        boostTimerJob = scope.launch {
            var remaining = minutes
            while (remaining > 0) {
                delay(60_000L)
                remaining -= 1
                _boostRemainingMinutes.value = remaining
            }
            networkDelegate?.onBoostStateUpdated(false)
            appendLocalLog(LogLevel.INFO, "Live Boost timer expired -> returning to Auto")
        }
    }

    fun cancelBoost() {
        val wasActive = _boostRemainingMinutes.value > 0
        boostTimerJob?.cancel()
        boostTimerJob = null
        _boostRemainingMinutes.value = 0
        if (wasActive) {
            networkDelegate?.onBoostStateUpdated(false)
            appendLocalLog(LogLevel.INFO, "Live Boost cancelled")
        }
    }

    fun calibrateRadioTail(context: Context) {
        val instance = ensureInitialized(context)
        _calibrationSummary.value = "Calibrating carrier tail (3s..60s gaps)..."
        instance.calibrateRadioTail()
    }

    fun probeNow(context: Context) {
        ensureInitialized(context).probeNow()
    }

    fun testRootAccess() {
        _rootTestStatus.value = "Checking su access..."
        scope.launch {
            val res = RootRecoveryHelper.testRootAccess()
            res.onSuccess { msg ->
                _rootTestStatus.value = msg
                appendLocalLog(LogLevel.INFO, msg)
            }.onFailure { err ->
                val msg = err.message ?: "Root denied or su unavailable"
                _rootTestStatus.value = "Failed: $msg"
                appendLocalLog(LogLevel.WARN, "Root test failed: $msg")
            }
        }
    }

    fun clearLogs() {
        _localLogs.value = emptyList()
        engine?.clearLogs()
    }

    fun updateConfig(context: Context, newConfig: StabilizerConfig): Result<Unit> {
        val instance = ensureInitialized(context)
        if (_config.value == newConfig) {
            _errorBanner.value = null
            return Result.success(Unit)
        }
        return try {
            instance.updateConfig(newConfig)
            preferences?.saveConfig(newConfig)
            _config.value = newConfig
            _errorBanner.value = null
            networkDelegate?.wakeSampler()
            Result.success(Unit)
        } catch (ex: StabilizerException) {
            val msg = ex.message ?: "Invalid stabilizer configuration"
            _errorBanner.value = msg
            Result.failure(ex)
        }
    }

    fun clearErrorBanner() {
        _errorBanner.value = null
    }

    fun appendLocalLog(level: LogLevel, message: String) {
        val entry = LogEntry(
            timestampMs = System.currentTimeMillis().toULong(),
            level = level,
            message = message
        )
        _localLogs.update { current ->
            (current + entry).takeLast(MAX_LOCAL_LOGS)
        }
    }

    override fun onTelemetryUpdated(snapshot: TelemetrySnapshot) {
        _telemetry.value = snapshot
        val tileKey = Triple(snapshot.isRunning, snapshot.state, snapshot.boostActive)
        if (lastNotifiedTileKey != tileKey) {
            lastNotifiedTileKey = tileKey
            appContext?.let { StabilizerTileService.requestTileUpdate(it) }
        }
    }

    override fun bindSocketToCellular(fd: Int): Boolean {
        return networkDelegate?.bindCellularSocketFd(fd) ?: true
    }

    override fun onRequestAndroidRevalidation() {
        appendLocalLog(LogLevel.WARN, "Step 2: triggering Android network re-validation...")
        networkDelegate?.requestAndroidRevalidation()
    }

    override fun onRequestRootRecovery() {
        appendLocalLog(LogLevel.WARN, "Step 3: executing gated root cellular toggle...")
        networkDelegate?.onRootRecoveryTriggered()
        scope.launch {
            val result = RootRecoveryHelper.toggleMobileDataAsRoot()
            result.onSuccess { msg ->
                appendLocalLog(LogLevel.INFO, msg)
            }.onFailure { err ->
                val message = err.message ?: "Root command failed"
                appendLocalLog(LogLevel.ERROR, "Root recovery failed: $message")
                _errorBanner.value = "Root recovery failed: $message"
            }
        }
    }

    override fun onCalibrationFinished(recommendedTailSecs: ULong, summary: String) {
        _calibrationSummary.value = summary
        val updated = _config.value.copy(radioTailSecs = recommendedTailSecs)
        _config.value = updated
        preferences?.saveConfig(updated)
    }
}
