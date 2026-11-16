package com.stabilizernano.app

import android.Manifest
import android.annotation.SuppressLint
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import com.stabilizernano.app.data.StabilizerRepository
import com.stabilizernano.app.ui.StabilizerScreen
import com.stabilizernano.app.ui.formatLogsForExport
import com.stabilizernano.app.ui.theme.StabilizerTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

class MainActivity : ComponentActivity() {

    private var pendingExportContent: String? = null
    private val statusNotice = MutableStateFlow<String?>(null)

    private val notificationPermissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) {
        StabilizerRepository.startService(this)
    }

    private val createLogFileLauncher = registerForActivityResult(
        ActivityResultContracts.CreateDocument("text/plain")
    ) { uri ->
        val content = pendingExportContent
        pendingExportContent = null
        if (uri == null || content == null) return@registerForActivityResult

        lifecycleScope.launch {
            val result = withContext(Dispatchers.IO) {
                runCatching {
                    contentResolver.openOutputStream(uri)?.bufferedWriter(Charsets.UTF_8)?.use {
                        it.write(content)
                    } ?: error("Unable to open destination file")
                }
            }
            statusNotice.value = result.fold(
                onSuccess = { "Saved telemetry logs to file" },
                onFailure = { err -> "Failed to save logs: ${err.message}" }
            )
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.auto(
                android.graphics.Color.TRANSPARENT,
                android.graphics.Color.TRANSPARENT
            ),
            navigationBarStyle = SystemBarStyle.auto(
                android.graphics.Color.TRANSPARENT,
                android.graphics.Color.TRANSPARENT
            )
        )
        StabilizerRepository.ensureInitialized(applicationContext)

        setContent {
            StabilizerTheme {
                val telemetry by StabilizerRepository.telemetry.collectAsStateWithLifecycle()
                val config by StabilizerRepository.config.collectAsStateWithLifecycle()
                val localLogs by StabilizerRepository.localLogs.collectAsStateWithLifecycle()
                val errorBanner by StabilizerRepository.errorBanner.collectAsStateWithLifecycle()
                val uiNotice by statusNotice.collectAsStateWithLifecycle()

                var isBatteryExempt by remember { mutableStateOf(checkBatteryOptimizationExempt()) }
                val lifecycleOwner = LocalLifecycleOwner.current
                DisposableEffect(lifecycleOwner) {
                    val observer = LifecycleEventObserver { _, event ->
                        if (event == Lifecycle.Event.ON_RESUME) {
                            isBatteryExempt = checkBatteryOptimizationExempt()
                        }
                    }
                    lifecycleOwner.lifecycle.addObserver(observer)
                    onDispose {
                        lifecycleOwner.lifecycle.removeObserver(observer)
                    }
                }

                StabilizerScreen(
                    telemetry = telemetry,
                    config = config,
                    localLogs = localLogs,
                    errorBanner = errorBanner ?: uiNotice,
                    isBatteryExempt = isBatteryExempt,
                    onToggleRunning = { handleToggleService(telemetry.isRunning) },
                    onSaveConfig = { newConfig ->
                        val res = StabilizerRepository.updateConfig(applicationContext, newConfig)
                        if (res.isSuccess) {
                            statusNotice.value = "Settings saved and applied"
                        }
                    },
                    onClearLogs = {
                        StabilizerRepository.clearLogs()
                        statusNotice.value = "Logs cleared"
                    },
                    onExportLogsFile = { logsToExport ->
                        if (logsToExport.isEmpty()) {
                            statusNotice.value = "No logs available to export"
                        } else {
                            pendingExportContent = formatLogsForExport(logsToExport)
                            val stamp = SimpleDateFormat("yyyyMMdd-HHmmss", Locale.US)
                                .format(Date())
                            createLogFileLauncher.launch("stabilizer-nano-logs-$stamp.txt")
                        }
                    },
                    onShowNotice = { msg -> statusNotice.value = msg },
                    onRequestBatteryExemption = { requestBatteryOptimizationExemption() },
                    onDismissError = {
                        StabilizerRepository.clearErrorBanner()
                        statusNotice.value = null
                    }
                )
            }
        }
    }

    private fun handleToggleService(isCurrentlyRunning: Boolean) {
        if (!isCurrentlyRunning && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val granted = ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.POST_NOTIFICATIONS
            ) == PackageManager.PERMISSION_GRANTED
            if (!granted) {
                notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
                return
            }
        }
        StabilizerRepository.toggleService(this)
    }

    private fun checkBatteryOptimizationExempt(): Boolean {
        val powerManager = getSystemService(PowerManager::class.java)
        return powerManager?.isIgnoringBatteryOptimizations(packageName) ?: true
    }

    @SuppressLint("BatteryLife")
    private fun requestBatteryOptimizationExemption() {
        val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS).apply {
            data = Uri.parse("package:$packageName")
        }
        runCatching { startActivity(intent) }.onFailure {
            startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
        }
    }
}
