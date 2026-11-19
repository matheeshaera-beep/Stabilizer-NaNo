package com.stabilizernano.app.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.BatteryAlert
import androidx.compose.material.icons.rounded.BatteryChargingFull
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.Dashboard
import androidx.compose.material.icons.rounded.DeleteSweep
import androidx.compose.material.icons.rounded.NetworkCheck
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.SaveAlt
import androidx.compose.material.icons.rounded.Security
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Speed
import androidx.compose.material.icons.rounded.Stop
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material.icons.rounded.WifiTethering
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
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
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.stabilizernano.uniffi.LogEntry
import com.stabilizernano.uniffi.LogLevel
import com.stabilizernano.uniffi.StabilizerConfig
import com.stabilizernano.uniffi.StabilizerProfile
import com.stabilizernano.uniffi.StabilizerState
import com.stabilizernano.uniffi.TelemetrySnapshot
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

enum class StabilizerTab(val label: String, val icon: ImageVector) {
    MAIN("Main", Icons.Rounded.Dashboard),
    LOGS("Logs", Icons.Rounded.Terminal),
    SETTINGS("Settings", Icons.Rounded.Settings)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun StabilizerScreen(
    telemetry: TelemetrySnapshot,
    config: StabilizerConfig,
    localLogs: List<LogEntry>,
    errorBanner: String?,
    isBatteryExempt: Boolean,
    boostRemainingMinutes: Int,
    rootTestStatus: String?,
    calibrationSummary: String?,
    onToggleRunning: () -> Unit,
    onSelectProfile: (StabilizerProfile) -> Unit,
    onToggleBoost: () -> Unit,
    onProbeNow: () -> Unit,
    onCalibrateTail: () -> Unit,
    onTestRoot: () -> Unit,
    onSaveConfig: (StabilizerConfig) -> Unit,
    onClearLogs: () -> Unit,
    onExportLogsFile: (List<LogEntry>) -> Unit,
    onShowNotice: (String) -> Unit,
    onRequestBatteryExemption: () -> Unit,
    onDismissError: () -> Unit,
    modifier: Modifier = Modifier
) {
    val snackbarHostState = remember { SnackbarHostState() }
    var selectedTab by rememberSaveable { mutableStateOf(StabilizerTab.MAIN) }

    var tailInput by remember(config.radioTailSecs) {
        mutableStateOf(config.radioTailSecs.toString())
    }
    var idleInput by remember(config.idleThresholdSecs) {
        mutableStateOf(config.idleThresholdSecs.toString())
    }
    var pingUrlInput by remember(config.pingUrl) {
        mutableStateOf(config.pingUrl)
    }
    var rootEnabledInput by remember(config.rootRecoveryEnabled) {
        mutableStateOf(config.rootRecoveryEnabled)
    }
    var liveChargingInput by remember(config.liveOnCharging) {
        mutableStateOf(config.liveOnCharging)
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
            .take(160)
    }

    Scaffold(
        modifier = modifier.fillMaxSize(),
        containerColor = MaterialTheme.colorScheme.background,
        snackbarHost = {
            SnackbarHost(
                hostState = snackbarHostState,
                modifier = Modifier.padding(bottom = 96.dp)
            )
        },
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.background,
                    titleContentColor = MaterialTheme.colorScheme.onBackground
                ),
                title = {
                    Column {
                        Text(
                            text = "Stabilizer-NaNo",
                            style = MaterialTheme.typography.titleLarge,
                            fontWeight = FontWeight.Bold
                        )
                        Text(
                            text = "Cellular tail & stall-heal engine • Tail ${config.radioTailSecs}s",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            )
        }
    ) { innerPadding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(top = innerPadding.calculateTopPadding())
        ) {
            AnimatedContent(
                targetState = selectedTab,
                modifier = Modifier.fillMaxSize(),
                transitionSpec = {
                    val forward = targetState.ordinal >= initialState.ordinal
                    val enterSlide = slideInHorizontally(
                        animationSpec = spring(
                            dampingRatio = Spring.DampingRatioNoBouncy,
                            stiffness = Spring.StiffnessMediumLow
                        ),
                        initialOffsetX = { fullWidth -> if (forward) fullWidth / 10 else -fullWidth / 10 }
                    )
                    val exitSlide = slideOutHorizontally(
                        animationSpec = spring(
                            dampingRatio = Spring.DampingRatioNoBouncy,
                            stiffness = Spring.StiffnessMediumLow
                        ),
                        targetOffsetX = { fullWidth -> if (forward) -fullWidth / 10 else fullWidth / 10 }
                    )
                    (enterSlide + fadeIn() + scaleIn(initialScale = 0.97f))
                        .togetherWith(exitSlide + fadeOut() + scaleOut(targetScale = 0.97f))
                },
                label = "StabilizerTabTransition"
            ) { tab ->
                when (tab) {
                    StabilizerTab.MAIN -> MainTabContent(
                        telemetry = telemetry,
                        profile = config.profile,
                        isBatteryExempt = isBatteryExempt,
                        boostRemainingMinutes = boostRemainingMinutes,
                        onToggleRunning = onToggleRunning,
                        onSelectProfile = onSelectProfile,
                        onToggleBoost = onToggleBoost,
                        onProbeNow = onProbeNow,
                        onRequestBatteryExemption = onRequestBatteryExemption
                    )
                    StabilizerTab.LOGS -> LogsTabContent(
                        logs = mergedLogs,
                        onClearLogs = onClearLogs,
                        onExportLogsFile = { onExportLogsFile(mergedLogs) },
                        onShowNotice = onShowNotice
                    )
                    StabilizerTab.SETTINGS -> SettingsTabContent(
                        profile = config.profile,
                        onSelectProfile = onSelectProfile,
                        tailInput = tailInput,
                        onTailChange = { tailInput = it.filter(Char::isDigit) },
                        idleInput = idleInput,
                        onIdleChange = { idleInput = it.filter(Char::isDigit) },
                        pingUrlInput = pingUrlInput,
                        onPingUrlChange = { pingUrlInput = it },
                        rootEnabled = rootEnabledInput,
                        onRootEnabledChange = { rootEnabledInput = it },
                        liveOnCharging = liveChargingInput,
                        onLiveOnChargingChange = { liveChargingInput = it },
                        isCalibrating = telemetry.isCalibrating,
                        calibrationSummary = calibrationSummary,
                        rootTestStatus = rootTestStatus,
                        isBatteryExempt = isBatteryExempt,
                        onCalibrateTail = onCalibrateTail,
                        onTestRoot = onTestRoot,
                        onRequestBatteryExemption = onRequestBatteryExemption,
                        onApply = {
                            val tail = tailInput.toULongOrNull() ?: 8UL
                            val idle = idleInput.toULongOrNull() ?: 4UL
                            onSaveConfig(
                                config.copy(
                                    radioTailSecs = tail,
                                    idleThresholdSecs = idle,
                                    offlineSleepSecs = maxOf(config.offlineSleepSecs, idle),
                                    pingUrl = pingUrlInput.trim(),
                                    rootRecoveryEnabled = rootEnabledInput,
                                    liveOnCharging = liveChargingInput
                                )
                            )
                        }
                    )
                }
            }

            FloatingPillNavBar(
                selectedTab = selectedTab,
                onSelectTab = { selectedTab = it },
                modifier = Modifier
                    .align(Alignment.BottomCenter)
                    .navigationBarsPadding()
                    .padding(horizontal = 24.dp, vertical = 14.dp)
            )
        }
    }
}

@Composable
private fun FloatingPillNavBar(
    selectedTab: StabilizerTab,
    onSelectTab: (StabilizerTab) -> Unit,
    modifier: Modifier = Modifier
) {
    val tabs = StabilizerTab.entries
    Surface(
        modifier = modifier
            .widthIn(max = 420.dp)
            .fillMaxWidth()
            .height(66.dp),
        shape = CircleShape,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        tonalElevation = 6.dp,
        shadowElevation = 10.dp,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
    ) {
        BoxWithConstraints(
            modifier = Modifier
                .fillMaxSize()
                .padding(6.dp)
        ) {
            val tabWidth = maxWidth / tabs.size
            val indicatorOffset by animateDpAsState(
                targetValue = tabWidth * selectedTab.ordinal,
                animationSpec = spring(
                    dampingRatio = Spring.DampingRatioMediumBouncy,
                    stiffness = Spring.StiffnessMediumLow
                ),
                label = "FloatingNavIndicatorOffset"
            )

            Box(
                modifier = Modifier
                    .offset(x = indicatorOffset)
                    .width(tabWidth)
                    .fillMaxHeight()
                    .clip(CircleShape)
                    .background(MaterialTheme.colorScheme.primaryContainer)
            )

            Row(modifier = Modifier.fillMaxSize()) {
                tabs.forEach { tab ->
                    val isSelected = tab == selectedTab
                    val iconScale by animateFloatAsState(
                        targetValue = if (isSelected) 1.08f else 0.96f,
                        animationSpec = spring(
                            dampingRatio = Spring.DampingRatioMediumBouncy,
                            stiffness = Spring.StiffnessMedium
                        ),
                        label = "NavIconScale_${tab.name}"
                    )
                    val contentColor by animateColorAsState(
                        targetValue = if (isSelected) {
                            MaterialTheme.colorScheme.onPrimaryContainer
                        } else {
                            MaterialTheme.colorScheme.onSurfaceVariant
                        },
                        animationSpec = spring(stiffness = Spring.StiffnessMediumLow),
                        label = "NavContentColor_${tab.name}"
                    )

                    Row(
                        modifier = Modifier
                            .weight(1f)
                            .fillMaxHeight()
                            .clip(CircleShape)
                            .clickable(
                                interactionSource = remember { MutableInteractionSource() },
                                indication = null
                            ) { onSelectTab(tab) },
                        horizontalArrangement = Arrangement.Center,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Icon(
                            imageVector = tab.icon,
                            contentDescription = tab.label,
                            tint = contentColor,
                            modifier = Modifier
                                .size(20.dp)
                                .scale(iconScale)
                        )
                        Spacer(modifier = Modifier.width(6.dp))
                        Text(
                            text = tab.label,
                            color = contentColor,
                            style = MaterialTheme.typography.labelLarge,
                            fontWeight = if (isSelected) FontWeight.Bold else FontWeight.Medium,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun MainTabContent(
    telemetry: TelemetrySnapshot,
    profile: StabilizerProfile,
    isBatteryExempt: Boolean,
    boostRemainingMinutes: Int,
    onToggleRunning: () -> Unit,
    onSelectProfile: (StabilizerProfile) -> Unit,
    onToggleBoost: () -> Unit,
    onProbeNow: () -> Unit,
    onRequestBatteryExemption: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 16.dp)
            .padding(top = 8.dp, bottom = 108.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp)
    ) {
        if (!isBatteryExempt) {
            BatteryOptimizationCard(onRequestBatteryExemption = onRequestBatteryExemption)
        }

        ControlStatusCard(
            telemetry = telemetry,
            profile = profile,
            boostRemainingMinutes = boostRemainingMinutes,
            onToggleRunning = onToggleRunning,
            onSelectProfile = onSelectProfile,
            onToggleBoost = onToggleBoost,
            onProbeNow = onProbeNow
        )

        TelemetryMetricsRow(telemetry = telemetry)

        DailyEfficiencyCard(telemetry = telemetry)
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ControlStatusCard(
    telemetry: TelemetrySnapshot,
    profile: StabilizerProfile,
    boostRemainingMinutes: Int,
    onToggleRunning: () -> Unit,
    onSelectProfile: (StabilizerProfile) -> Unit,
    onToggleBoost: () -> Unit,
    onProbeNow: () -> Unit
) {
    val (badgeBg, badgeFg, stateTitle) = stateVisuals(telemetry.state)
    val animatedBg by animateColorAsState(
        targetValue = badgeBg,
        animationSpec = spring(
            dampingRatio = Spring.DampingRatioMediumBouncy,
            stiffness = Spring.StiffnessMediumLow
        ),
        label = "stateBadgeBg"
    )
    val animatedFg by animateColorAsState(
        targetValue = badgeFg,
        animationSpec = spring(stiffness = Spring.StiffnessMediumLow),
        label = "stateBadgeFg"
    )

    val interactionSource = remember { MutableInteractionSource() }
    val isPressed by interactionSource.collectIsPressedAsState()
    val buttonScale by animateFloatAsState(
        targetValue = if (isPressed) 0.94f else 1f,
        animationSpec = spring(
            dampingRatio = Spring.DampingRatioMediumBouncy,
            stiffness = Spring.StiffnessMedium
        ),
        label = "StartStopScale"
    )

    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(24.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainer
        ),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(18.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp)
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Surface(
                    color = animatedBg,
                    shape = CircleShape
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 14.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        Box(
                            modifier = Modifier
                                .size(9.dp)
                                .clip(CircleShape)
                                .background(animatedFg)
                        )
                        Text(
                            text = stateTitle,
                            color = animatedFg,
                            style = MaterialTheme.typography.labelLarge,
                            fontWeight = FontWeight.Bold
                        )
                    }
                }

                Button(
                    onClick = onToggleRunning,
                    interactionSource = interactionSource,
                    modifier = Modifier.scale(buttonScale),
                    shape = CircleShape,
                    contentPadding = PaddingValues(horizontal = 20.dp, vertical = 12.dp),
                    colors = if (telemetry.isRunning) {
                        ButtonDefaults.buttonColors(
                            containerColor = MaterialTheme.colorScheme.errorContainer,
                            contentColor = MaterialTheme.colorScheme.onErrorContainer
                        )
                    } else {
                        ButtonDefaults.buttonColors(
                            containerColor = MaterialTheme.colorScheme.primary,
                            contentColor = MaterialTheme.colorScheme.onPrimary
                        )
                    }
                ) {
                    Icon(
                        imageVector = if (telemetry.isRunning) Icons.Rounded.Stop else Icons.Rounded.PlayArrow,
                        contentDescription = null
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        text = if (telemetry.isRunning) "Stop" else "Start",
                        fontWeight = FontWeight.SemiBold
                    )
                }
            }

            Text(
                text = telemetry.stateReason.ifBlank { stateDescription(telemetry.state, telemetry.isTethering) },
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurface
            )

            // Quick Profile Selector + 30m Live Boost + Manual Probe
            FlowRow(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                StabilizerProfile.entries.forEach { entry ->
                    FilterChip(
                        selected = profile == entry,
                        onClick = { onSelectProfile(entry) },
                        label = { Text(profileLabel(entry)) },
                        colors = FilterChipDefaults.filterChipColors(
                            selectedContainerColor = MaterialTheme.colorScheme.primaryContainer,
                            selectedLabelColor = MaterialTheme.colorScheme.onPrimaryContainer
                        )
                    )
                }

                FilterChip(
                    selected = boostRemainingMinutes > 0,
                    onClick = onToggleBoost,
                    leadingIcon = {
                        Icon(
                            imageVector = Icons.Rounded.Bolt,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp)
                        )
                    },
                    label = {
                        Text(
                            if (boostRemainingMinutes > 0) "Boost ${boostRemainingMinutes}m" else "Boost 30m"
                        )
                    },
                    colors = FilterChipDefaults.filterChipColors(
                        selectedContainerColor = MaterialTheme.colorScheme.tertiaryContainer,
                        selectedLabelColor = MaterialTheme.colorScheme.onTertiaryContainer
                    )
                )

                OutlinedButton(
                    onClick = onProbeNow,
                    shape = CircleShape,
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp)
                ) {
                    Icon(
                        imageVector = Icons.Rounded.NetworkCheck,
                        contentDescription = null,
                        modifier = Modifier.size(16.dp)
                    )
                    Spacer(modifier = Modifier.width(4.dp))
                    Text("Probe", style = MaterialTheme.typography.labelMedium)
                }
            }

            FlowRow(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                StatusChip(
                    text = if (telemetry.hasInternetCapability) "Cellular Ready" else "No Cellular",
                    icon = if (telemetry.hasInternetCapability) Icons.Rounded.CheckCircle else Icons.Rounded.Warning,
                    highlighted = telemetry.hasInternetCapability
                )
                StatusChip(
                    text = if (telemetry.isValidated) "Validated" else "Unvalidated",
                    icon = Icons.Rounded.Bolt,
                    highlighted = telemetry.isValidated
                )
                if (telemetry.isWifiDefault) {
                    StatusChip(
                        text = "Wi-Fi Default",
                        icon = Icons.Rounded.Wifi,
                        highlighted = true
                    )
                }
                val clients = telemetry.hotspotClients.toInt()
                StatusChip(
                    text = when {
                        clients > 0 -> "Hotspot ($clients clients)"
                        telemetry.isTethering -> "USB/BT Tether"
                        else -> "Tethering Off"
                    },
                    icon = Icons.Rounded.WifiTethering,
                    highlighted = telemetry.isTethering,
                    accentColor = MaterialTheme.colorScheme.tertiaryContainer,
                    accentContentColor = MaterialTheme.colorScheme.onTertiaryContainer
                )
            }
        }
    }
}

@Composable
private fun StatusChip(
    text: String,
    icon: ImageVector,
    highlighted: Boolean,
    accentColor: Color = MaterialTheme.colorScheme.primaryContainer,
    accentContentColor: Color = MaterialTheme.colorScheme.onPrimaryContainer
) {
    val bg by animateColorAsState(
        targetValue = if (highlighted) accentColor else MaterialTheme.colorScheme.surfaceContainerHigh,
        animationSpec = spring(stiffness = Spring.StiffnessMediumLow),
        label = "StatusChipBg"
    )
    val fg by animateColorAsState(
        targetValue = if (highlighted) accentContentColor else MaterialTheme.colorScheme.onSurfaceVariant,
        animationSpec = spring(stiffness = Spring.StiffnessMediumLow),
        label = "StatusChipFg"
    )
    Surface(
        shape = CircleShape,
        color = bg,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 7.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp)
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = fg,
                modifier = Modifier.size(15.dp)
            )
            Text(
                text = text,
                color = fg,
                style = MaterialTheme.typography.labelMedium,
                fontWeight = FontWeight.Medium
            )
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
            label = "Last RTT",
            value = telemetry.lastPingLatencyMs?.let { "${it} ms" } ?: "-- ms",
            subtext = "${telemetry.successfulPings} ok / ${telemetry.failedPings} err",
            modifier = Modifier.weight(1f)
        )
        MetricCard(
            label = "Probes Saved",
            value = telemetry.avoidedFixed4sPings.toString(),
            subtext = "${telemetry.totalPings} sent",
            modifier = Modifier.weight(1f)
        )
        MetricCard(
            label = "RX / TX",
            value = formatBytesPerSec(telemetry.lastRxBytesPerSec),
            subtext = "TX ${formatBytesPerSec(telemetry.lastTxBytesPerSec)}",
            modifier = Modifier.weight(1f)
        )
    }
}

@Composable
private fun DailyEfficiencyCard(telemetry: TelemetrySnapshot) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(22.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainer
        ),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp)
        ) {
            Text(
                text = "Session Efficiency & Healing",
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                MiniStat("Probe Bytes", formatBytesTotal(telemetry.totalProbeBytes))
                MiniStat("Stalls Healed", "${telemetry.stallsHealed}/${telemetry.stallsDetected}")
                MiniStat("Recoveries", telemetry.recoveriesTriggered.toString())
                MiniStat("Poll Rate", "${telemetry.recommendedSampleMs / 1000UL}s")
            }
        }
    }
}

@Composable
private fun MiniStat(label: String, value: String) {
    Column(horizontalAlignment = Alignment.Start) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        Text(
            text = value,
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.Bold,
            color = MaterialTheme.colorScheme.onSurface
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
        shape = RoundedCornerShape(20.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerHigh
        ),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
    ) {
        Column(
            modifier = Modifier.padding(14.dp),
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
                color = MaterialTheme.colorScheme.onSurface,
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
private fun LogsTabContent(
    logs: List<LogEntry>,
    onClearLogs: () -> Unit,
    onExportLogsFile: () -> Unit,
    onShowNotice: (String) -> Unit
) {
    val clipboardManager = LocalClipboardManager.current

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(horizontal = 16.dp)
            .padding(top = 8.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        Card(
            modifier = Modifier.fillMaxWidth(),
            shape = RoundedCornerShape(20.dp),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainer
            ),
            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = "Live Telemetry Logs",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold
                    )
                    Text(
                        text = "${logs.size} events",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }

                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilledTonalIconButton(
                        onClick = {
                            if (logs.isEmpty()) {
                                onShowNotice("No logs to copy")
                            } else {
                                clipboardManager.setText(AnnotatedString(formatLogsForExport(logs)))
                                onShowNotice("Copied ${logs.size} log entries")
                            }
                        }
                    ) {
                        Icon(
                            imageVector = Icons.Rounded.ContentCopy,
                            contentDescription = "Copy logs"
                        )
                    }

                    FilledTonalIconButton(onClick = onExportLogsFile) {
                        Icon(
                            imageVector = Icons.Rounded.SaveAlt,
                            contentDescription = "Save logs to file"
                        )
                    }

                    FilledTonalIconButton(onClick = onClearLogs) {
                        Icon(
                            imageVector = Icons.Rounded.DeleteSweep,
                            contentDescription = "Clear logs"
                        )
                    }
                }
            }
        }

        if (logs.isEmpty()) {
            Card(
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(20.dp),
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surfaceContainer
                ),
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
            ) {
                Text(
                    text = "No events yet. Start the stabilizer from the Main tab to record probes, stalls, and state transitions.",
                    modifier = Modifier.padding(18.dp),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        } else {
            LazyColumn(
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(bottom = 108.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                itemsIndexed(
                    items = logs,
                    key = { index, item -> "${item.timestampMs}-${item.level}-$index" }
                ) { _, entry ->
                    LogEntryCard(entry = entry)
                }
            }
        }
    }
}

@Composable
private fun LogEntryCard(entry: LogEntry) {
    val (pillBg, pillFg) = when (entry.level) {
        LogLevel.INFO -> MaterialTheme.colorScheme.primaryContainer to MaterialTheme.colorScheme.onPrimaryContainer
        LogLevel.WARN -> MaterialTheme.colorScheme.tertiaryContainer to MaterialTheme.colorScheme.onTertiaryContainer
        LogLevel.ERROR -> MaterialTheme.colorScheme.errorContainer to MaterialTheme.colorScheme.onErrorContainer
    }
    val formattedTime = remember(entry.timestampMs) {
        SimpleDateFormat("HH:mm:ss", Locale.US).format(Date(entry.timestampMs.toLong()))
    }

    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(16.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainer
        ),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp, vertical = 10.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text(
                text = formattedTime,
                style = MaterialTheme.typography.labelSmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Surface(
                color = pillBg,
                shape = RoundedCornerShape(6.dp)
            ) {
                Text(
                    text = entry.level.name,
                    modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                    style = MaterialTheme.typography.labelSmall,
                    fontWeight = FontWeight.Bold,
                    color = pillFg
                )
            }
            Text(
                text = entry.message,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.weight(1f)
            )
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun SettingsTabContent(
    profile: StabilizerProfile,
    onSelectProfile: (StabilizerProfile) -> Unit,
    tailInput: String,
    onTailChange: (String) -> Unit,
    idleInput: String,
    onIdleChange: (String) -> Unit,
    pingUrlInput: String,
    onPingUrlChange: (String) -> Unit,
    rootEnabled: Boolean,
    onRootEnabledChange: (Boolean) -> Unit,
    liveOnCharging: Boolean,
    onLiveOnChargingChange: (Boolean) -> Unit,
    isCalibrating: Boolean,
    calibrationSummary: String?,
    rootTestStatus: String?,
    isBatteryExempt: Boolean,
    onCalibrateTail: () -> Unit,
    onTestRoot: () -> Unit,
    onRequestBatteryExemption: () -> Unit,
    onApply: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 16.dp)
            .padding(top = 8.dp, bottom = 108.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp)
    ) {
        // Carrier Radio Tail Calibration Card
        Card(
            modifier = Modifier.fillMaxWidth(),
            shape = RoundedCornerShape(24.dp),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainer
            ),
            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(18.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = "Carrier Radio Tail Calibration",
                            style = MaterialTheme.typography.titleMedium,
                            fontWeight = FontWeight.SemiBold
                        )
                        Text(
                            text = "Tests 3s, 6s, 10s, 15s, 30s, 60s idle gaps to measure your carrier's RRC sleep threshold.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    Spacer(modifier = Modifier.width(8.dp))
                    FilledTonalButton(
                        onClick = onCalibrateTail,
                        enabled = !isCalibrating,
                        shape = CircleShape
                    ) {
                        if (isCalibrating) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(16.dp),
                                strokeWidth = 2.dp
                            )
                            Spacer(modifier = Modifier.width(6.dp))
                            Text("Running")
                        } else {
                            Icon(
                                imageVector = Icons.Rounded.Speed,
                                contentDescription = null,
                                modifier = Modifier.size(18.dp)
                            )
                            Spacer(modifier = Modifier.width(6.dp))
                            Text("Calibrate")
                        }
                    }
                }
                if (!calibrationSummary.isNullOrBlank()) {
                    Text(
                        text = calibrationSummary,
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary
                    )
                }
            }
        }

        Card(
            modifier = Modifier.fillMaxWidth(),
            shape = RoundedCornerShape(24.dp),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainer
            ),
            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(18.dp),
                verticalArrangement = Arrangement.spacedBy(14.dp)
            ) {
                Text(
                    text = "Profile & Probe Parameters",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold
                )

                FlowRow(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    StabilizerProfile.entries.forEach { entry ->
                        FilterChip(
                            selected = profile == entry,
                            onClick = { onSelectProfile(entry) },
                            label = { Text(profileLabel(entry)) }
                        )
                    }
                }

                Text(
                    text = profileDescription(profile),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    OutlinedTextField(
                        value = tailInput,
                        onValueChange = onTailChange,
                        label = { Text("Radio tail (4-20s)") },
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        singleLine = true,
                        modifier = Modifier.weight(1f)
                    )
                    OutlinedTextField(
                        value = idleInput,
                        onValueChange = onIdleChange,
                        label = { Text("Active exit (s)") },
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        singleLine = true,
                        modifier = Modifier.weight(1f)
                    )
                }

                OutlinedTextField(
                    value = pingUrlInput,
                    onValueChange = onPingUrlChange,
                    label = { Text("L2 HEAD probe URL") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = "Live while charging",
                            style = MaterialTheme.typography.titleSmall,
                            fontWeight = FontWeight.Medium
                        )
                        Text(
                            text = "Allow Live keepalives in Auto mode while plugged in with screen on.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    Switch(
                        checked = liveOnCharging,
                        onCheckedChange = {
                            onLiveOnChargingChange(it)
                            onApply()
                        }
                    )
                }

                Button(
                    onClick = onApply,
                    shape = CircleShape,
                    modifier = Modifier.align(Alignment.End)
                ) {
                    Text("Save & Apply")
                }
            }
        }

        Card(
            modifier = Modifier.fillMaxWidth(),
            shape = RoundedCornerShape(24.dp),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainer
            ),
            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(18.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Column(
                        modifier = Modifier
                            .weight(1f)
                            .padding(end = 12.dp)
                    ) {
                        Text(
                            text = "Root Auto-Recovery",
                            style = MaterialTheme.typography.titleSmall,
                            fontWeight = FontWeight.SemiBold
                        )
                        Spacer(modifier = Modifier.height(4.dp))
                        Text(
                            text = "Fallback after Android re-validation (rate-limited: 2m cooldown, max 3/15m, paused during calls).",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    Switch(
                        checked = rootEnabled,
                        onCheckedChange = {
                            onRootEnabledChange(it)
                            onApply()
                        }
                    )
                }

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        text = rootTestStatus ?: "Tap Test Root to trigger Magisk/su prompt immediately.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier
                            .weight(1f)
                            .padding(end = 12.dp)
                    )
                    FilledTonalButton(
                        onClick = onTestRoot,
                        shape = CircleShape
                    ) {
                        Icon(
                            imageVector = Icons.Rounded.Security,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp)
                        )
                        Spacer(modifier = Modifier.width(6.dp))
                        Text("Test Root")
                    }
                }
            }
        }

        Card(
            modifier = Modifier.fillMaxWidth(),
            shape = RoundedCornerShape(24.dp),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceContainer
            ),
            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(18.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Icon(
                    imageVector = if (isBatteryExempt) Icons.Rounded.BatteryChargingFull else Icons.Rounded.BatteryAlert,
                    contentDescription = null,
                    tint = if (isBatteryExempt) MaterialTheme.colorScheme.tertiary else MaterialTheme.colorScheme.primary
                )
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = "Battery Optimization",
                        style = MaterialTheme.typography.titleSmall,
                        fontWeight = FontWeight.SemiBold
                    )
                    Text(
                        text = if (isBatteryExempt) {
                            "Unrestricted background execution is enabled."
                        } else {
                            "Exempt this app so stall checks and foreground service stay reliable."
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                FilledTonalButton(
                    onClick = onRequestBatteryExemption,
                    shape = CircleShape
                ) {
                    Text(if (isBatteryExempt) "Manage" else "Exempt")
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
        shape = RoundedCornerShape(20.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.tertiaryContainer
        ),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
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
                    text = "Prevents aggressive OEM background kills.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onTertiaryContainer
                )
            }
            FilledTonalButton(
                onClick = onRequestBatteryExemption,
                shape = CircleShape
            ) {
                Text("Exempt")
            }
        }
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
        StabilizerState.LIVE_KEEPALIVE -> Triple(
            MaterialTheme.colorScheme.primaryContainer,
            MaterialTheme.colorScheme.onPrimaryContainer,
            "LIVE"
        )
        StabilizerState.ECO_TAIL_CHECK -> Triple(
            MaterialTheme.colorScheme.secondaryContainer,
            MaterialTheme.colorScheme.onSecondaryContainer,
            "ECO TAIL"
        )
        StabilizerState.WARMUP -> Triple(
            MaterialTheme.colorScheme.secondaryContainer,
            MaterialTheme.colorScheme.onSecondaryContainer,
            "WARMUP"
        )
        StabilizerState.QUIET -> Triple(
            MaterialTheme.colorScheme.surfaceContainerHighest,
            MaterialTheme.colorScheme.onSurfaceVariant,
            "QUIET"
        )
        StabilizerState.RECOVERING -> Triple(
            MaterialTheme.colorScheme.errorContainer,
            MaterialTheme.colorScheme.onErrorContainer,
            "RECOVERING"
        )
        StabilizerState.OUTAGE -> Triple(
            MaterialTheme.colorScheme.errorContainer,
            MaterialTheme.colorScheme.onErrorContainer,
            "OUTAGE"
        )
        StabilizerState.STOPPED -> Triple(
            MaterialTheme.colorScheme.surfaceContainerHighest,
            MaterialTheme.colorScheme.onSurfaceVariant,
            "STOPPED"
        )
    }
}

private fun profileLabel(profile: StabilizerProfile): String {
    return when (profile) {
        StabilizerProfile.AUTO -> "Auto"
        StabilizerProfile.ECO -> "Eco"
        StabilizerProfile.HEAL_ONLY -> "Heal Only"
        StabilizerProfile.LIVE -> "Live"
    }
}

private fun profileDescription(profile: StabilizerProfile): String {
    return when (profile) {
        StabilizerProfile.AUTO -> "Eco by default; switches to Live during calls, hotspot/tethering, Boost, or charging (if enabled)."
        StabilizerProfile.ECO -> "Brief tail confirmation after traffic pauses, then 0 pings in Quiet standby."
        StabilizerProfile.HEAL_ONLY -> "Lowest battery: 0 pings while healthy; probes & recovers only when a stall or drop occurs."
        StabilizerProfile.LIVE -> "Keeps the cellular modem inside its carrier tail while the screen is on."
    }
}

private fun stateDescription(state: StabilizerState, isTethering: Boolean): String {
    return when (state) {
        StabilizerState.ACTIVE -> "Real RX traffic moving. 0 probes sent."
        StabilizerState.ECO_TAIL_CHECK -> "Brief post-traffic confirmation inside carrier tail window."
        StabilizerState.QUIET -> "0 pings standby; wakes instantly on traffic, TX stalls, or radio events."
        StabilizerState.LIVE_KEEPALIVE -> if (isTethering) {
            "Hotspot/USB clients active; keeping modem tail warm."
        } else {
            "Live keepalive keeping modem inside carrier RRC tail."
        }
        StabilizerState.WARMUP -> "Link woke up; verifying cellular path."
        StabilizerState.RECOVERING -> "Healing silent stall or unvalidated cellular link."
        StabilizerState.OUTAGE -> "Carrier outage detected; backing off exponentially."
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

private fun formatBytesTotal(totalBytes: ULong): String {
    val bytes = totalBytes.toLong()
    return when {
        bytes < 1024L -> "$bytes B"
        bytes < 1024L * 1024L -> String.format(Locale.US, "%.1f KB", bytes / 1024.0)
        else -> String.format(Locale.US, "%.2f MB", bytes / (1024.0 * 1024.0))
    }
}

fun formatLogsForExport(logs: List<LogEntry>): String {
    val formatter = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.US)
    return buildString {
        appendLine("Stabilizer-NaNo V2 Telemetry Logs")
        appendLine("Exported: ${formatter.format(Date())}")
        appendLine("Entries: ${logs.size}")
        appendLine("----------------------------------------")
        for (entry in logs.asReversed()) {
            val ts = formatter.format(Date(entry.timestampMs.toLong()))
            appendLine("[$ts] [${entry.level.name}] ${entry.message}")
        }
    }
}
