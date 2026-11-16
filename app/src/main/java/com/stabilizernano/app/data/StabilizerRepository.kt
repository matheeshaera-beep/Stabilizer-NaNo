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
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

object StabilizerRepository : StabilizerObserver {
    private const val MAX_LOCAL_LOGS = 40

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val initLock = Any()

    @Volatile
    private var appContext: Context? = null

    @Volatile
    private var preferences: StabilizerPreferences? = null

    @Volatile
    private var engine: StabilizerEngine? = null

    @Volatile
    private var lastNotifiedTileState: Pair<Boolean, StabilizerState>? = null

    private val _config = MutableStateFlow(defaultStabilizerConfig())
    val config: StateFlow<StabilizerConfig> = _config.asStateFlow()

    private val _telemetry = MutableStateFlow(
        TelemetrySnapshot(
            state = StabilizerState.STOPPED,
            isRunning = false,
            lastPingLatencyMs = null,
            totalPings = 0UL,
            successfulPings = 0UL,
            failedPings = 0UL,
            lastTrafficBytesPerSec = 0UL,
            hasInternetCapability = false,
            isValidated = false,
            isScreenOn = true,
            isTethering = false,
            logs = emptyList()
        )
    )
    val telemetry: StateFlow<TelemetrySnapshot> = _telemetry.asStateFlow()

    private val _localLogs = MutableStateFlow<List<LogEntry>>(emptyList())
    val localLogs: StateFlow<List<LogEntry>> = _localLogs.asStateFlow()

    private val _errorBanner = MutableStateFlow<String?>(null)
    val errorBanner: StateFlow<String?> = _errorBanner.asStateFlow()

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

    fun startService(context: Context) {
        ensureInitialized(context)
        val intent = Intent(context, StabilizerForegroundService::class.java).apply {
            action = StabilizerForegroundService.ACTION_START
        }
        ContextCompat.startForegroundService(context, intent)
    }

    fun stopService(context: Context) {
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

    fun clearLogs() {
        _localLogs.value = emptyList()
        engine?.clearLogs()
    }

    fun updateConfig(context: Context, newConfig: StabilizerConfig): Result<Unit> {
        val instance = ensureInitialized(context)
        return try {
            instance.updateConfig(newConfig)
            preferences?.saveConfig(newConfig)
            _config.value = newConfig
            _errorBanner.value = null
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
        val tileKey = snapshot.isRunning to snapshot.state
        if (lastNotifiedTileState != tileKey) {
            lastNotifiedTileState = tileKey
            appContext?.let { StabilizerTileService.requestTileUpdate(it) }
        }
    }

    override fun onRequestRootRecovery() {
        appendLocalLog(LogLevel.WARN, "Executing root cellular recovery (svc data toggle)...")
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
}
