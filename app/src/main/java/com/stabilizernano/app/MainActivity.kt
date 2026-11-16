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
import com.stabilizernano.app.data.StabilizerRepository
import com.stabilizernano.app.ui.StabilizerScreen
import com.stabilizernano.app.ui.theme.StabilizerTheme

class MainActivity : ComponentActivity() {

    private val notificationPermissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) {
        StabilizerRepository.startService(this)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        StabilizerRepository.ensureInitialized(applicationContext)

        setContent {
            StabilizerTheme {
                val telemetry by StabilizerRepository.telemetry.collectAsStateWithLifecycle()
                val config by StabilizerRepository.config.collectAsStateWithLifecycle()
                val localLogs by StabilizerRepository.localLogs.collectAsStateWithLifecycle()
                val errorBanner by StabilizerRepository.errorBanner.collectAsStateWithLifecycle()

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
                    errorBanner = errorBanner,
                    isBatteryExempt = isBatteryExempt,
                    onToggleRunning = { handleToggleService(telemetry.isRunning) },
                    onSaveConfig = { newConfig ->
                        StabilizerRepository.updateConfig(applicationContext, newConfig)
                    },
                    onRequestBatteryExemption = { requestBatteryOptimizationExemption() },
                    onDismissError = { StabilizerRepository.clearErrorBanner() }
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
