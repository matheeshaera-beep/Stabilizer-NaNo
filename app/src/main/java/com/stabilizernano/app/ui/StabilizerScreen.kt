package com.stabilizernano.app.ui

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.BatteryAlert
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Stop
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.stabilizernano.uniffi.LogEntry
import com.stabilizernano.uniffi.LogLevel
import com.stabilizernano.uniffi.StabilizerConfig
import com.stabilizernano.uniffi.StabilizerState
import com.stabilizernano.uniffi.TelemetrySnapshot
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun StabilizerScreen(
    telemetry: TelemetrySnapshot,
    config: StabilizerConfig,
    localLogs: List<LogEntry>,
    errorBanner: String?,
    isBatteryExempt: Boolean,
    onToggleRunning: () -> Unit,
    onSaveConfig: (StabilizerConfig) -> Unit,
    onRequestBatteryExemption: () -> Unit,
    onDismissError: () -> Unit,
    modifier: Modifier = Modifier
) {
    val snackbarHostState = remember { SnackbarHostState() }
    var showSettings by rememberSaveable { mutableStateOf(false) }

    var idleInput by remember(config.idleThresholdSecs) {
        mutableStateOf(config.idleThresholdSecs.toString())
    }
    var sleepInput by remember(config.offlineSleepSecs) {
        mutableStateOf(config.offlineSleepSecs.toString())
    }
    var pingUrlInput by remember(config.pingUrl) {
        mutableStateOf(config.pingUrl)
    }
    var rootEnabledInput by remember(config.rootRecoveryEnabled) {
        mutableStateOf(config.rootRecoveryEnabled)
    }

    LaunchedEffect(errorBanner) {
        if (!errorBanner.isNullOrBlank()) {
            snackbarHostState.showSnackbar(errorBanner)
            onDismissError()
        }
    }

    val mergedLogs = remember(telemetry.logs, localLogs) {
        (telemetry.logs + localLogs)
            .sortedByDescending { it.timestampMs }
            .take(100)
    }

    Scaffold(
        modifier = modifier.fillMaxSize(),
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(
                            text = "Stabilizer-NaNo",
                            style = MaterialTheme.typography.titleLarge,
                            fontWeight = FontWeight.SemiBold
                        )
                        Text(
                            text = "Idle-aware Rust + UniFFI network keeper",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                },
                actions = {
                    IconButton(onClick = { showSettings = !showSettings }) {
                        Icon(
                            imageVector = Icons.Rounded.Settings,
                            contentDescription = "Toggle settings"
                        )
                    }
                }
            )
        }
    ) { innerPadding ->
        LazyColumn(
            modifier = Modifier
                .fillMaxSize()
                .padding(innerPadding),
            contentPadding = PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp)
        ) {
            if (!isBatteryExempt) {
                item(key = "battery_banner") {
                    BatteryOptimizationCard(onRequestBatteryExemption = onRequestBatteryExemption)
                }
            }

            item(key = "control_card") {
                ControlStatusCard(
                    telemetry = telemetry,
                    onToggleRunning = onToggleRunning
                )
            }

            item(key = "metrics_row") {
                TelemetryMetricsRow(telemetry = telemetry)
            }

            if (showSettings) {
                item(key = "settings_card") {
                    SettingsCard(
                        idleInput = idleInput,
                        onIdleChange = { idleInput = it.filter(Char::isDigit) },
                        sleepInput = sleepInput,
                        onSleepChange = { sleepInput = it.filter(Char::isDigit) },
                        pingUrlInput = pingUrlInput,
                        onPingUrlChange = { pingUrlInput = it },
                        rootEnabled = rootEnabledInput,
                        onRootEnabledChange = { rootEnabledInput = it },
                        onApply = {
                            val idle = idleInput.toULongOrNull() ?: 0UL
                            val sleep = sleepInput.toULongOrNull() ?: 0UL
                            onSaveConfig(
                                config.copy(
                                    idleThresholdSecs = idle,
                                    offlineSleepSecs = sleep,
                                    pingUrl = pingUrlInput.trim(),
                                    rootRecoveryEnabled = rootEnabledInput
                                )
                            )
                        }
                    )
                }
            }

            item(key = "logs_header") {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        text = "Live Telemetry Logs",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold
                    )
                    Text(
                        text = "${mergedLogs.size} events",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }

            if (mergedLogs.isEmpty()) {
                item(key = "empty_logs") {
                    Card(
                        modifier = Modifier.fillMaxWidth(),
                        colors = CardDefaults.cardColors(
                            containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.45f)
                        )
                    ) {
                        Text(
                            text = "No events yet. Tap Start to begin idle-aware cellular monitoring.",
                            modifier = Modifier.padding(16.dp),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            } else {
                itemsIndexed(
                    items = mergedLogs,
                    key = { index, item -> "${item.timestampMs}-${item.level}-$index" }
                ) { _, entry ->
                    LogEntryRow(entry = entry)
                }
            }
        }
    }
}

@Composable
private fun BatteryOptimizationCard(
    onRequestBatteryExemption: () -> Unit
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.tertiaryContainer
        )
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Icon(
                imageVector = Icons.Rounded.BatteryAlert,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onTertiaryContainer
            )
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = "Allow unrestricted background use",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onTertiaryContainer
                )
                Text(
                    text = "Prevents Doze from pausing cellular monitoring.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onTertiaryContainer
                )
            }
            FilledTonalButton(onClick = onRequestBatteryExemption) {
                Text("Exempt")
            }
        }
    }
}

@Composable
private fun ControlStatusCard(
    telemetry: TelemetrySnapshot,
    onToggleRunning: () -> Unit
) {
    val (badgeBg, badgeFg, stateTitle) = stateVisuals(telemetry.state)
    val animatedBg by animateColorAsState(targetValue = badgeBg, label = "stateBadgeBg")

    ElevatedCard(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp)
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Surface(
                    color = animatedBg,
                    shape = RoundedCornerShape(50)
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        Box(
                            modifier = Modifier
                                .size(9.dp)
                                .clip(CircleShape)
                                .background(badgeFg)
                        )
                        Text(
                            text = stateTitle,
                            color = badgeFg,
                            style = MaterialTheme.typography.labelLarge,
                            fontWeight = FontWeight.Bold
                        )
                    }
                }

                Button(
                    onClick = onToggleRunning,
                    colors = if (telemetry.isRunning) {
                        ButtonDefaults.buttonColors(
                            containerColor = MaterialTheme.colorScheme.error,
                            contentColor = MaterialTheme.colorScheme.onError
                        )
                    } else {
                        ButtonDefaults.buttonColors()
                    }
                ) {
                    Icon(
                        imageVector = if (telemetry.isRunning) Icons.Rounded.Stop else Icons.Rounded.PlayArrow,
                        contentDescription = null
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(if (telemetry.isRunning) "Stop" else "Start")
                }
            }

            Text(
                text = stateDescription(telemetry.state),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                AssistChip(
                    onClick = {},
                    label = {
                        Text(
                            if (telemetry.hasInternetCapability) "Cellular Ready" else "No Cellular"
                        )
                    },
                    leadingIcon = {
                        Icon(
                            imageVector = if (telemetry.hasInternetCapability) {
                                Icons.Rounded.CheckCircle
                            } else {
                                Icons.Rounded.Warning
                            },
                            contentDescription = null,
                            modifier = Modifier.size(16.dp)
                        )
                    }
                )
                AssistChip(
                    onClick = {},
                    label = {
                        Text(if (telemetry.isValidated) "Validated" else "Unvalidated")
                    },
                    leadingIcon = {
                        Icon(
                            imageVector = Icons.Rounded.Bolt,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp)
                        )
                    }
                )
            }
        }
    }
}

@Composable
private fun TelemetryMetricsRow(telemetry: TelemetrySnapshot) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(10.dp)
    ) {
        MetricCard(
            label = "Last Latency",
            value = telemetry.lastPingLatencyMs?.let { "${it} ms" } ?: "-- ms",
            subtext = "HEAD probe",
            modifier = Modifier.weight(1f)
        )
        MetricCard(
            label = "Total Pings",
            value = telemetry.totalPings.toString(),
            subtext = "${telemetry.successfulPings} ok / ${telemetry.failedPings} err",
            modifier = Modifier.weight(1f)
        )
        MetricCard(
            label = "User Traffic",
            value = formatBytesPerSec(telemetry.lastTrafficBytesPerSec),
            subtext = "Excludes self",
            modifier = Modifier.weight(1f)
        )
    }
}

@Composable
private fun MetricCard(
    label: String,
    value: String,
    subtext: String,
    modifier: Modifier = Modifier
) {
    Card(
        modifier = modifier,
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.55f)
        )
    ) {
        Column(
            modifier = Modifier.padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp)
        ) {
            Text(
                text = label,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
            Text(
                text = value,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
            Text(
                text = subtext,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
        }
    }
}

@Composable
private fun SettingsCard(
    idleInput: String,
    onIdleChange: (String) -> Unit,
    sleepInput: String,
    onSleepChange: (String) -> Unit,
    pingUrlInput: String,
    onPingUrlChange: (String) -> Unit,
    rootEnabled: Boolean,
    onRootEnabledChange: (Boolean) -> Unit,
    onApply: () -> Unit
) {
    ElevatedCard(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Text(
                text = "Stabilizer Settings",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold
            )

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                OutlinedTextField(
                    value = idleInput,
                    onValueChange = onIdleChange,
                    label = { Text("Idle threshold (s)") },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    singleLine = true,
                    modifier = Modifier.weight(1f)
                )
                OutlinedTextField(
                    value = sleepInput,
                    onValueChange = onSleepChange,
                    label = { Text("Offline sleep (s)") },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    singleLine = true,
                    modifier = Modifier.weight(1f)
                )
            }

            OutlinedTextField(
                value = pingUrlInput,
                onValueChange = onPingUrlChange,
                label = { Text("HEAD probe URL") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth()
            )

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Column(modifier = Modifier.weight(1f).padding(end = 12.dp)) {
                    Text(
                        text = "Root Auto-Recovery",
                        style = MaterialTheme.typography.bodyLarge,
                        fontWeight = FontWeight.Medium
                    )
                    Text(
                        text = "Runs su svc data disable/enable if unvalidated for 10s while active.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                Switch(
                    checked = rootEnabled,
                    onCheckedChange = onRootEnabledChange
                )
            }

            Button(
                onClick = onApply,
                modifier = Modifier.align(Alignment.End)
            ) {
                Text("Save & Apply")
            }
        }
    }
}

@Composable
private fun LogEntryRow(entry: LogEntry) {
    val levelColor = when (entry.level) {
        LogLevel.INFO -> MaterialTheme.colorScheme.primary
        LogLevel.WARN -> MaterialTheme.colorScheme.tertiary
        LogLevel.ERROR -> MaterialTheme.colorScheme.error
    }
    val formattedTime = remember(entry.timestampMs) {
        SimpleDateFormat("HH:mm:ss", Locale.US).format(Date(entry.timestampMs.toLong()))
    }

    Column(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.Top
        ) {
            Text(
                text = formattedTime,
                style = MaterialTheme.typography.labelSmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Text(
                text = entry.level.name,
                style = MaterialTheme.typography.labelSmall,
                fontWeight = FontWeight.Bold,
                color = levelColor
            )
            Text(
                text = entry.message,
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.weight(1f)
            )
        }
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.4f))
    }
}

@Composable
private fun stateVisuals(state: StabilizerState): Triple<Color, Color, String> {
    return when (state) {
        StabilizerState.ACTIVE -> Triple(
            MaterialTheme.colorScheme.tertiaryContainer,
            MaterialTheme.colorScheme.onTertiaryContainer,
            "ACTIVE"
        )
        StabilizerState.IDLE_PING -> Triple(
            MaterialTheme.colorScheme.primaryContainer,
            MaterialTheme.colorScheme.onPrimaryContainer,
            "IDLE PING"
        )
        StabilizerState.WARMUP -> Triple(
            MaterialTheme.colorScheme.secondaryContainer,
            MaterialTheme.colorScheme.onSecondaryContainer,
            "WARMUP"
        )
        StabilizerState.SLEEPING -> Triple(
            MaterialTheme.colorScheme.surfaceVariant,
            MaterialTheme.colorScheme.onSurfaceVariant,
            "SLEEPING"
        )
        StabilizerState.RECOVERING -> Triple(
            MaterialTheme.colorScheme.errorContainer,
            MaterialTheme.colorScheme.onErrorContainer,
            "RECOVERING"
        )
        StabilizerState.STOPPED -> Triple(
            MaterialTheme.colorScheme.surfaceVariant,
            MaterialTheme.colorScheme.onSurfaceVariant,
            "STOPPED"
        )
    }
}

private fun stateDescription(state: StabilizerState): String {
    return when (state) {
        StabilizerState.ACTIVE -> "Real mobile traffic is moving. Sending 0 keepalive pings."
        StabilizerState.IDLE_PING -> "Traffic is idle past threshold. Sending 1 lightweight HEAD probe per idle window."
        StabilizerState.WARMUP -> "Screen woke or unlocked. Warming cellular radio with 1 instant HEAD ping."
        StabilizerState.SLEEPING -> "Zero-ping sleep mode (screen off or offline app). Resumes on real traffic or screen wake."
        StabilizerState.RECOVERING -> "NET_CAPABILITY_VALIDATED lost while active. Probing link recovery."
        StabilizerState.STOPPED -> "Stabilizer service is stopped."
    }
}

private fun formatBytesPerSec(bytesPerSec: ULong): String {
    val bytes = bytesPerSec.toLong()
    return when {
        bytes < 1024L -> "$bytes B/s"
        bytes < 1024L * 1024L -> String.format(Locale.US, "%.1f KB/s", bytes / 1024.0)
        else -> String.format(Locale.US, "%.2f MB/s", bytes / (1024.0 * 1024.0))
    }
}
