package com.stabilizernano.app.service

import android.content.ComponentName
import android.content.Context
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import com.stabilizernano.app.R
import com.stabilizernano.app.data.StabilizerRepository
import com.stabilizernano.uniffi.StabilizerState

class StabilizerTileService : TileService() {

    override fun onStartListening() {
        super.onStartListening()
        StabilizerRepository.ensureInitialized(applicationContext)
        updateTileAppearance()
    }

    override fun onClick() {
        super.onClick()
        StabilizerRepository.ensureInitialized(applicationContext)
        StabilizerRepository.toggleService(applicationContext)
        updateTileAppearance()
    }

    private fun updateTileAppearance() {
        val tile = qsTile ?: return
        val snapshot = StabilizerRepository.telemetry.value

        tile.state = if (snapshot.isRunning) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.label = getString(R.string.tile_label)

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            tile.subtitle = if (!snapshot.isRunning) {
                "Stopped"
            } else {
                val base = when (snapshot.state) {
                    StabilizerState.ACTIVE -> "Active • 0 pings"
                    StabilizerState.ECO_TAIL_CHECK -> "Eco tail"
                    StabilizerState.QUIET -> "Quiet • 0 pings"
                    StabilizerState.LIVE_KEEPALIVE -> if (snapshot.boostActive) "Live Boost" else "Live"
                    StabilizerState.WARMUP -> "Warmup"
                    StabilizerState.RECOVERING -> "Recovering"
                    StabilizerState.OUTAGE -> "No carrier data"
                    StabilizerState.STOPPED -> "Starting"
                }
                val latency = snapshot.lastPingLatencyMs?.let { " • ${it}ms" } ?: ""
                "$base$latency"
            }
        }
        tile.updateTile()
    }

    companion object {
        fun requestTileUpdate(context: Context) {
            runCatching {
                requestListeningState(
                    context.applicationContext,
                    ComponentName(context.applicationContext, StabilizerTileService::class.java)
                )
            }
        }
    }
}
