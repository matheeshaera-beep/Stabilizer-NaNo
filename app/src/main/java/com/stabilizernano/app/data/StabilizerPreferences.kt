package com.stabilizernano.app.data

import android.content.Context
import android.content.SharedPreferences
import com.stabilizernano.uniffi.StabilizerConfig
import com.stabilizernano.uniffi.defaultStabilizerConfig

class StabilizerPreferences(context: Context) {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    fun loadConfig(): StabilizerConfig {
        val defaults = defaultStabilizerConfig()
        val idleSecs = prefs.getLong(KEY_IDLE_THRESHOLD_SECS, defaults.idleThresholdSecs.toLong())
            .coerceAtLeast(1L)
        val offlineSleepSecs = prefs.getLong(KEY_OFFLINE_SLEEP_SECS, defaults.offlineSleepSecs.toLong())
            .coerceAtLeast(idleSecs)
        val pingUrl = prefs.getString(KEY_PING_URL, defaults.pingUrl)
            ?.trim()
            ?.takeIf { it.startsWith("http://") || it.startsWith("https://") }
            ?: defaults.pingUrl
        val rootRecovery = prefs.getBoolean(KEY_ROOT_RECOVERY_ENABLED, defaults.rootRecoveryEnabled)
        val recoveryTimeout = prefs.getLong(
            KEY_RECOVERY_TIMEOUT_SECS,
            defaults.recoveryTimeoutSecs.toLong()
        ).coerceAtLeast(1L)

        return StabilizerConfig(
            idleThresholdSecs = idleSecs.toULong(),
            offlineSleepSecs = offlineSleepSecs.toULong(),
            pingUrl = pingUrl,
            rootRecoveryEnabled = rootRecovery,
            recoveryTimeoutSecs = recoveryTimeout.toULong()
        )
    }

    fun saveConfig(config: StabilizerConfig) {
        prefs.edit()
            .putLong(KEY_IDLE_THRESHOLD_SECS, config.idleThresholdSecs.toLong())
            .putLong(KEY_OFFLINE_SLEEP_SECS, config.offlineSleepSecs.toLong())
            .putString(KEY_PING_URL, config.pingUrl.trim())
            .putBoolean(KEY_ROOT_RECOVERY_ENABLED, config.rootRecoveryEnabled)
            .putLong(KEY_RECOVERY_TIMEOUT_SECS, config.recoveryTimeoutSecs.toLong())
            .apply()
    }

    companion object {
        private const val PREFS_NAME = "stabilizer_nano_prefs"
        private const val KEY_IDLE_THRESHOLD_SECS = "idle_threshold_secs"
        private const val KEY_OFFLINE_SLEEP_SECS = "offline_sleep_secs"
        private const val KEY_PING_URL = "ping_url"
        private const val KEY_ROOT_RECOVERY_ENABLED = "root_recovery_enabled"
        private const val KEY_RECOVERY_TIMEOUT_SECS = "recovery_timeout_secs"
    }
}
