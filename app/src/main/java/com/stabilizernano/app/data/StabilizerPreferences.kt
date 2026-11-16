package com.stabilizernano.app.data

import android.content.Context
import android.content.SharedPreferences
import com.stabilizernano.uniffi.StabilizerConfig
import com.stabilizernano.uniffi.StabilizerProfile
import com.stabilizernano.uniffi.defaultStabilizerConfig

class StabilizerPreferences(context: Context) {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    fun loadConfig(): StabilizerConfig {
        val defaults = defaultStabilizerConfig()
        val profileName = prefs.getString(KEY_PROFILE, defaults.profile.name)
        val profile = runCatching {
            StabilizerProfile.valueOf(profileName ?: defaults.profile.name)
        }.getOrDefault(defaults.profile)

        val radioTailSecs = prefs.getLong(KEY_RADIO_TAIL_SECS, defaults.radioTailSecs.toLong())
            .coerceIn(4L, 20L)
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
        val liveOnCharging = prefs.getBoolean(KEY_LIVE_ON_CHARGING, defaults.liveOnCharging)

        return StabilizerConfig(
            profile = profile,
            radioTailSecs = radioTailSecs.toULong(),
            idleThresholdSecs = idleSecs.toULong(),
            offlineSleepSecs = offlineSleepSecs.toULong(),
            pingUrl = pingUrl,
            rootRecoveryEnabled = rootRecovery,
            recoveryTimeoutSecs = recoveryTimeout.toULong(),
            liveOnCharging = liveOnCharging
        )
    }

    fun saveConfig(config: StabilizerConfig) {
        prefs.edit()
            .putString(KEY_PROFILE, config.profile.name)
            .putLong(KEY_RADIO_TAIL_SECS, config.radioTailSecs.toLong())
            .putLong(KEY_IDLE_THRESHOLD_SECS, config.idleThresholdSecs.toLong())
            .putLong(KEY_OFFLINE_SLEEP_SECS, config.offlineSleepSecs.toLong())
            .putString(KEY_PING_URL, config.pingUrl.trim())
            .putBoolean(KEY_ROOT_RECOVERY_ENABLED, config.rootRecoveryEnabled)
            .putLong(KEY_RECOVERY_TIMEOUT_SECS, config.recoveryTimeoutSecs.toLong())
            .putBoolean(KEY_LIVE_ON_CHARGING, config.liveOnCharging)
            .apply()
    }

    companion object {
        private const val PREFS_NAME = "stabilizer_nano_prefs"
        private const val KEY_PROFILE = "profile"
        private const val KEY_RADIO_TAIL_SECS = "radio_tail_secs"
        private const val KEY_IDLE_THRESHOLD_SECS = "idle_threshold_secs"
        private const val KEY_OFFLINE_SLEEP_SECS = "offline_sleep_secs"
        private const val KEY_PING_URL = "ping_url"
        private const val KEY_ROOT_RECOVERY_ENABLED = "root_recovery_enabled"
        private const val KEY_RECOVERY_TIMEOUT_SECS = "recovery_timeout_secs"
        private const val KEY_LIVE_ON_CHARGING = "live_on_charging"
    }
}
