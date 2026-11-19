use std::collections::VecDeque;
use std::fmt;

use crate::pinger::ProbeTier;

/// Minimum downstream throughput (bytes/sec) treated as real RX user progress.
pub const ACTIVE_RX_THRESHOLD_BYTES_PER_SEC: u64 = 1_024;

/// Instant `Active` entry threshold (bytes/sec) for streaming / heavy downloads.
pub const HEAVY_DOWNLOAD_BYTES_PER_SEC: u64 = 4_096;

/// Minimum sustained outbound traffic (bytes/sec) while RX is stalled to flag a silent freeze.
pub const STALL_TX_THRESHOLD_BYTES_PER_SEC: u64 = 128;

/// Upper bound on RX (bytes/sec) considered "no progress" during a TX stall (slow uploads receive >= 64 B/s ACKs).
pub const STALL_RX_CEILING_BYTES_PER_SEC: u64 = 64;

/// Consecutive seconds of `TX >= 128 && RX < 64` before firing an instant stall-heal probe.
pub const STALL_CONSECUTIVE_SECS: u64 = 2;

/// Grace period (seconds) before dropping from `Active` to `Quiet` after a screen-off download stops.
pub const SCREEN_OFF_DOWNLOAD_GRACE_SECS: u64 = 8;

/// Maximum confirmation checks in `EcoTailCheck` before settling into `Quiet` (0 pings).
pub const MAX_ECO_TAIL_CHECKS: u8 = 2;

/// Minimum interval (seconds) between periodic L2 HTTP `HEAD` ground-truth checks while in `LiveKeepalive`.
pub const LIVE_L2_REVERIFY_SECS: u64 = 600;

/// Minimum cooldown (seconds) between root cellular recovery triggers.
pub const ROOT_RECOVERY_COOLDOWN_SECS: u64 = 120;

/// Sliding window (seconds) capping total root toggles (`MAX_ROOT_RECOVERIES_PER_WINDOW`).
pub const ROOT_RECOVERY_WINDOW_SECS: u64 = 900;

/// Maximum root cellular toggles allowed inside a 15-minute window.
pub const MAX_ROOT_RECOVERIES_PER_WINDOW: usize = 3;

/// User-selectable operating profile for Stabilizer-NaNo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum StabilizerProfile {
    /// Eco by default; automatically promotes to `LiveKeepalive` during calls, Boost, active hotspot/USB clients, or opted-in charging/gaming.
    Auto,
    /// Zero periodic keepalives after brief tail settle; wakes on real traffic, RX/TX stall, or radio topology events.
    Eco,
    /// Pure reactive mode: 0 pings while the cellular link is healthy; probes and recovers only on TX stalls, unvalidated links, or OS link blocks.
    HealOnly,
    /// Keeps the cellular modem inside its RRC tail window whenever screen is on (and safe battery/Wi-Fi checks pass).
    Live,
}

/// Operating state of the Stabilizer-NaNo V2 engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum StabilizerState {
    /// Engine is stopped.
    Stopped,
    /// Immediate single probe on start, screen wake/unlock, or link restore.
    Warmup,
    /// Real downstream (`rx`) traffic is moving (`>= 1 KB/s` hysteresis); sends **0 pings**.
    Active,
    /// Eco post-traffic confirmation window (`<= 2` L1 probes inside `radio_tail_secs`) before entering `Quiet`.
    EcoTailCheck,
    /// Zero-ping low-overhead standby (idle Eco, screen-off without clients, Wi-Fi default, Doze, or low battery).
    Quiet,
    /// Context-gated fast keepalive running slightly under `radio_tail_secs` with `±20%` jitter.
    LiveKeepalive,
    /// Non-root-first healing ladder triggered by TX stall or loss of `NET_CAPABILITY_VALIDATED`.
    Recovering,
    /// Carrier outage backoff (`30s -> 60s -> 120s -> 300s`) after consecutive failed recovery ladders.
    Outage,
}

impl fmt::Display for StabilizerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped => write!(f, "Stopped"),
            Self::Warmup => write!(f, "Warmup"),
            Self::Active => write!(f, "Active"),
            Self::EcoTailCheck => write!(f, "EcoTailCheck"),
            Self::Quiet => write!(f, "Quiet"),
            Self::LiveKeepalive => write!(f, "LiveKeepalive"),
            Self::Recovering => write!(f, "Recovering"),
            Self::Outage => write!(f, "Outage"),
        }
    }
}

/// Severity level for status logs surfaced to the Android UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Configuration parameters for [`StabilizerEngine`](crate::StabilizerEngine).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct StabilizerConfig {
    pub profile: StabilizerProfile,
    pub radio_tail_secs: u64,
    pub idle_threshold_secs: u64,
    pub offline_sleep_secs: u64,
    pub ping_url: String,
    pub root_recovery_enabled: bool,
    pub recovery_timeout_secs: u64,
    pub live_on_charging: bool,
}

impl Default for StabilizerConfig {
    fn default() -> Self {
        Self {
            profile: StabilizerProfile::Auto,
            radio_tail_secs: 8,
            idle_threshold_secs: 4,
            offline_sleep_secs: 60,
            ping_url: "https://connectivitycheck.gstatic.com/generate_204".to_owned(),
            root_recovery_enabled: false,
            recovery_timeout_secs: 10,
            live_on_charging: false,
        }
    }
}

impl StabilizerConfig {
    /// Validates user-supplied configuration bounds.
    pub fn validate(&self) -> Result<(), String> {
        if !(4..=20).contains(&self.radio_tail_secs) {
            return Err("radio_tail_secs must be between 4 and 20 seconds".to_owned());
        }
        if self.idle_threshold_secs == 0 {
            return Err("idle_threshold_secs must be at least 1 second".to_owned());
        }
        if self.offline_sleep_secs < self.idle_threshold_secs {
            return Err(
                "offline_sleep_secs must be greater than or equal to idle_threshold_secs"
                    .to_owned(),
            );
        }
        if self.recovery_timeout_secs == 0 {
            return Err("recovery_timeout_secs must be at least 1 second".to_owned());
        }
        let trimmed = self.ping_url.trim();
        let valid_scheme = trimmed.starts_with("https://") || trimmed.starts_with("http://");
        let min_len = if trimmed.starts_with("https://") {
            "https://".len()
        } else {
            "http://".len()
        };
        if !valid_scheme || trimmed.len() <= min_len {
            return Err("ping_url must be a valid http:// or https:// URL".to_owned());
        }
        Ok(())
    }
}

/// Real-time device & network context collected by Android.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct RuntimeContext {
    pub is_wifi_default: bool,
    pub in_call: bool,
    pub hotspot_clients: u32,
    pub is_usb_or_bt_tethering: bool,
    pub is_charging: bool,
    pub low_battery_or_saver: bool,
    pub is_doze: bool,
    pub boost_active: bool,
    pub live_app_active: bool,
}

impl RuntimeContext {
    pub fn has_active_tethering(&self) -> bool {
        self.hotspot_clients > 0 || self.is_usb_or_bt_tethering
    }
}

/// Reason a cellular probe was triggered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PingReason {
    Warmup,
    EcoTail,
    LiveKeepalive,
    LiveReverify,
    StallCheck,
    RadioEvent,
    Recovery,
    OutageCheck,
}

/// Side-effect action emitted by [`StateMachine`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    SendProbe { tier: ProbeTier, reason: PingReason },
    RequestAndroidRevalidation,
    TriggerRootRecovery,
    Log(LogLevel, String),
}

/// Deterministic input events processed by [`StateMachine`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Start,
    Stop,
    ScreenChanged {
        is_screen_on: bool,
        is_unlocked: bool,
    },
    ContextChanged(RuntimeContext),
    NetworkCapabilitiesChanged {
        has_internet: bool,
        is_validated: bool,
    },
    RadioTopologyChanged {
        reason: String,
    },
    TrafficObserved {
        rx_bytes_delta: u64,
        tx_bytes_delta: u64,
    },
    ProbeCompleted {
        tier: ProbeTier,
        reason: PingReason,
        succeeded: bool,
    },
    ProbeSkippedLocalBind {
        tier: ProbeTier,
        reason: PingReason,
    },
    TickElapsed {
        secs: u64,
    },
}

/// Pure, deterministic V2 state machine for Stabilizer-NaNo.
#[derive(Debug)]
pub struct StateMachine {
    config: StabilizerConfig,
    context: RuntimeContext,
    state: StabilizerState,
    state_reason: String,
    is_running: bool,
    is_screen_on: bool,
    is_unlocked: bool,
    has_internet: bool,
    is_validated: bool,

    uptime_secs: u64,
    idle_secs: u64,
    stall_secs: u64,
    secs_since_last_probe: u64,
    next_live_interval_secs: u64,
    secs_since_l2_verify: u64,
    eco_checks_sent: u8,

    recovering_secs: u64,
    android_revalidation_sent: bool,
    root_recovery_sent_this_episode: bool,
    post_root_settle_until_uptime_secs: u64,
    root_recovery_history_secs: VecDeque<u64>,
    consecutive_failed_ladders: u8,
    outage_backoff_step: u8,
    outage_wait_secs: u64,

    rx_history: [bool; 3],
    rx_history_idx: usize,
    recent_rx_active_this_tick: bool,
    was_heavy_rx_prev_tick: bool,
    warmup_just_entered: bool,
    last_rx_bytes_per_sec: u64,
    last_tx_bytes_per_sec: u64,

    rng_state: u64,
    stalls_detected: u64,
    stalls_healed: u64,
    recoveries_triggered: u64,
    android_revalidations: u64,
    avoided_ping_accum_secs: u64,
    avoided_fixed_4s_pings: u64,
}

impl StateMachine {
    /// Creates a new stopped state machine with the given validated configuration.
    pub fn new(config: StabilizerConfig) -> Self {
        let mut sm = Self {
            config,
            context: RuntimeContext::default(),
            state: StabilizerState::Stopped,
            state_reason: "Stopped".to_owned(),
            is_running: false,
            is_screen_on: true,
            is_unlocked: true,
            has_internet: false,
            is_validated: false,
            uptime_secs: 0,
            idle_secs: 0,
            stall_secs: 0,
            secs_since_last_probe: 0,
            next_live_interval_secs: 6,
            secs_since_l2_verify: 0,
            eco_checks_sent: 0,
            recovering_secs: 0,
            android_revalidation_sent: false,
            root_recovery_sent_this_episode: false,
            post_root_settle_until_uptime_secs: 0,
            root_recovery_history_secs: VecDeque::new(),
            consecutive_failed_ladders: 0,
            outage_backoff_step: 0,
            outage_wait_secs: 0,
            rx_history: [false; 3],
            rx_history_idx: 0,
            recent_rx_active_this_tick: false,
            was_heavy_rx_prev_tick: false,
            warmup_just_entered: false,
            last_rx_bytes_per_sec: 0,
            last_tx_bytes_per_sec: 0,
            rng_state: 0x9E37_79B9_7F4A_7C15,
            stalls_detected: 0,
            stalls_healed: 0,
            recoveries_triggered: 0,
            android_revalidations: 0,
            avoided_ping_accum_secs: 0,
            avoided_fixed_4s_pings: 0,
        };
        sm.recompute_next_live_interval();
        sm
    }

    pub fn state(&self) -> StabilizerState {
        self.state
    }

    pub fn state_reason(&self) -> &str {
        &self.state_reason
    }

    pub fn is_running(&self) -> bool {
        self.is_running
    }

    pub fn is_screen_on(&self) -> bool {
        self.is_screen_on
    }

    pub fn is_tethering(&self) -> bool {
        self.context.has_active_tethering()
    }

    pub fn hotspot_clients(&self) -> u32 {
        self.context.hotspot_clients
    }

    pub fn has_internet(&self) -> bool {
        self.has_internet
    }

    pub fn is_validated(&self) -> bool {
        self.is_validated
    }

    pub fn last_rx_bytes_per_sec(&self) -> u64 {
        self.last_rx_bytes_per_sec
    }

    pub fn last_tx_bytes_per_sec(&self) -> u64 {
        self.last_tx_bytes_per_sec
    }

    pub fn stalls_detected(&self) -> u64 {
        self.stalls_detected
    }

    pub fn stalls_healed(&self) -> u64 {
        self.stalls_healed
    }

    pub fn recoveries_triggered(&self) -> u64 {
        self.recoveries_triggered
    }

    pub fn avoided_fixed_4s_pings(&self) -> u64 {
        self.avoided_fixed_4s_pings
    }

    pub fn context(&self) -> &RuntimeContext {
        &self.context
    }

    pub fn config(&self) -> &StabilizerConfig {
        &self.config
    }

    pub fn update_config(&mut self, config: StabilizerConfig) -> Vec<Action> {
        self.config = config;
        self.recompute_next_live_interval();
        let mut actions = Vec::new();
        self.reconcile_mode_change(&mut actions);
        actions
    }

    /// Returns `true` when Wi-Fi is default or screen-off/Doze rules force zero-ping `Quiet` standby.
    pub fn is_quiet_forced(&self) -> bool {
        if self.context.is_wifi_default && !self.context.has_active_tethering() {
            return true;
        }
        if (self.context.is_doze || !self.is_screen_on)
            && !self.context.has_active_tethering()
            && !self.context.in_call
            && !self.context.boost_active
            && !self.recent_rx_active_this_tick
        {
            return true;
        }
        false
    }

    /// Returns `true` when pure reactive Heal-Only mode is active (0 pings while healthy,
    /// unless the user explicitly activates temporary `Boost`).
    pub fn is_heal_only(&self) -> bool {
        self.config.profile == StabilizerProfile::HealOnly && !self.context.boost_active
    }

    /// Returns `true` when context and profile justify keeping the cellular radio tail warm.
    pub fn is_live_eligible(&self) -> bool {
        if self.context.is_wifi_default && !self.context.has_active_tethering() {
            return false;
        }
        if (self.context.is_doze || !self.is_screen_on)
            && !self.context.has_active_tethering()
            && !self.context.in_call
            && !self.context.boost_active
        {
            return false;
        }

        // Explicit Boost 30m override honors keepalive intent even in HealOnly or Battery Saver.
        if self.context.boost_active {
            return true;
        }

        if self.config.profile == StabilizerProfile::HealOnly {
            return false;
        }

        // In-call, active tethering, or explicit Live profile honor keepalive intent
        // even when Android Battery Saver is active.
        if self.context.in_call || self.context.has_active_tethering() {
            return true;
        }

        match self.config.profile {
            StabilizerProfile::Eco | StabilizerProfile::HealOnly => false,
            StabilizerProfile::Live => true,
            StabilizerProfile::Auto => {
                !self.context.low_battery_or_saver
                    && (self.context.live_app_active
                        || (self.config.live_on_charging
                            && self.context.is_charging
                            && self.is_screen_on))
            }
        }
    }

    fn reconcile_mode_change(&mut self, actions: &mut Vec<Action>) {
        if !self.is_running || !self.has_internet || !self.is_validated {
            return;
        }
        if self.context.is_wifi_default && !self.context.has_active_tethering() {
            return;
        }

        if self.is_live_eligible() {
            if matches!(
                self.state,
                StabilizerState::Quiet | StabilizerState::EcoTailCheck | StabilizerState::Warmup
            ) {
                self.secs_since_last_probe = 0;
                self.recompute_next_live_interval();
                let label = self.live_reason_label();
                self.transition_to(StabilizerState::LiveKeepalive, &label, actions);
                actions.push(Action::SendProbe {
                    tier: ProbeTier::L1Dns,
                    reason: PingReason::LiveKeepalive,
                });
            } else if self.state == StabilizerState::LiveKeepalive {
                self.state_reason = self.live_reason_label();
            }
        } else if self.state == StabilizerState::LiveKeepalive {
            if self.is_quiet_forced() || self.context.low_battery_or_saver || self.is_heal_only() {
                let label = self.quiet_reason_label();
                self.transition_to(StabilizerState::Quiet, &label, actions);
            } else {
                self.eco_checks_sent = 0;
                self.secs_since_last_probe = 0;
                self.transition_to(
                    StabilizerState::EcoTailCheck,
                    "Switched to Eco mode",
                    actions,
                );
            }
        } else if self.is_heal_only()
            && matches!(
                self.state,
                StabilizerState::EcoTailCheck | StabilizerState::Warmup
            )
        {
            let label = self.quiet_reason_label();
            self.transition_to(StabilizerState::Quiet, &label, actions);
        } else if self.state == StabilizerState::Quiet {
            self.state_reason = self.quiet_reason_label();
        }
    }

    /// Processes an [`Event`] and returns zero or more side-effect [`Action`]s.
    pub fn handle_event(&mut self, event: Event) -> Vec<Action> {
        let mut actions = Vec::new();

        match event {
            Event::Start => {
                if self.is_running {
                    return actions;
                }
                self.is_running = true;
                self.reset_episode_timers();
                self.consecutive_failed_ladders = 0;
                self.outage_backoff_step = 0;
                self.outage_wait_secs = 0;

                if self.context.is_wifi_default && !self.context.has_active_tethering() {
                    self.transition_to(
                        StabilizerState::Quiet,
                        "Wi-Fi default -> Quiet (0 pings)",
                        &mut actions,
                    );
                } else if self.is_quiet_forced() {
                    self.transition_to(
                        StabilizerState::Quiet,
                        "Screen off / Doze -> Quiet (0 pings)",
                        &mut actions,
                    );
                } else if self.has_internet && !self.is_validated {
                    self.enter_recovering("Started on unvalidated cellular link", &mut actions);
                } else if self.has_internet && self.is_heal_only() {
                    let label = self.quiet_reason_label();
                    self.transition_to(StabilizerState::Quiet, &label, &mut actions);
                } else if self.has_internet {
                    self.transition_to(
                        StabilizerState::Warmup,
                        "Stabilizer started -> Warmup L2 check",
                        &mut actions,
                    );
                    actions.push(Action::SendProbe {
                        tier: ProbeTier::L2HttpHead,
                        reason: PingReason::Warmup,
                    });
                } else {
                    self.transition_to(
                        StabilizerState::Quiet,
                        "Waiting for cellular link -> Quiet",
                        &mut actions,
                    );
                }
            }
            Event::Stop => {
                if !self.is_running {
                    return actions;
                }
                self.is_running = false;
                self.reset_episode_timers();
                self.last_rx_bytes_per_sec = 0;
                self.last_tx_bytes_per_sec = 0;
                self.rx_history = [false; 3];
                self.transition_to(StabilizerState::Stopped, "Stopped", &mut actions);
            }
            Event::ScreenChanged {
                is_screen_on,
                is_unlocked,
            } => {
                let was_screen_on = self.is_screen_on;
                let was_unlocked = self.is_unlocked;
                self.is_screen_on = is_screen_on;
                self.is_unlocked = is_unlocked;

                if !self.is_running {
                    return actions;
                }

                if !is_screen_on {
                    if !self.context.has_active_tethering()
                        && !self.context.in_call
                        && !self.context.boost_active
                        && !self.recent_rx_active_this_tick
                        && self.state != StabilizerState::Quiet
                        && self.state != StabilizerState::Outage
                    {
                        self.reset_recovery_episode();
                        self.transition_to(
                            StabilizerState::Quiet,
                            "Screen off -> Quiet (0 pings)",
                            &mut actions,
                        );
                    } else if self.context.has_active_tethering() && was_screen_on {
                        let msg = if self.is_heal_only() {
                            format!(
                                "Screen off while tethering ({} clients) -> Heal-Only monitoring",
                                self.context.hotspot_clients
                            )
                        } else {
                            format!(
                                "Screen off while tethering ({} clients) -> keeping Live",
                                self.context.hotspot_clients
                            )
                        };
                        actions.push(Action::Log(LogLevel::Info, msg));
                    }
                    return actions;
                }

                let turned_on = !was_screen_on && is_screen_on;
                let unlocked_while_quiet = is_screen_on
                    && !was_unlocked
                    && is_unlocked
                    && matches!(
                        self.state,
                        StabilizerState::Quiet | StabilizerState::EcoTailCheck
                    );

                if (turned_on || unlocked_while_quiet)
                    && !(self.context.is_wifi_default && !self.context.has_active_tethering())
                {
                    self.idle_secs = 0;
                    self.stall_secs = 0;
                    self.eco_checks_sent = 0;
                    self.secs_since_last_probe = 0;
                    self.recent_rx_active_this_tick = false;

                    if self.has_internet && !self.is_validated {
                        self.enter_recovering("Screen wake on unvalidated link", &mut actions);
                    } else if self.has_internet
                        && (!self.is_heal_only() || self.state == StabilizerState::Outage)
                    {
                        self.transition_to(
                            StabilizerState::Warmup,
                            "Screen wake -> Warmup check",
                            &mut actions,
                        );
                        actions.push(Action::SendProbe {
                            tier: ProbeTier::L1Dns,
                            reason: PingReason::Warmup,
                        });
                    } else if self.has_internet
                        && self.is_heal_only()
                        && self.state == StabilizerState::Quiet
                    {
                        self.state_reason = self.quiet_reason_label();
                    }
                }
            }
            Event::ContextChanged(new_ctx) => {
                let prev_tether = self.context.has_active_tethering();
                let prev_wifi = self.context.is_wifi_default;
                let prev_live = self.is_live_eligible();
                self.context = new_ctx;

                if !self.is_running {
                    return actions;
                }

                if self.context.is_wifi_default && !self.context.has_active_tethering() {
                    if self.state != StabilizerState::Quiet {
                        self.reset_recovery_episode();
                        self.transition_to(
                            StabilizerState::Quiet,
                            "Wi-Fi default -> Quiet (0 pings)",
                            &mut actions,
                        );
                    }
                    return actions;
                }

                if prev_wifi && !self.context.is_wifi_default && self.has_internet {
                    self.idle_secs = 0;
                    self.eco_checks_sent = 0;
                    if !self.is_validated {
                        self.enter_recovering(
                            "Switched to unvalidated cellular link",
                            &mut actions,
                        );
                    } else if self.is_heal_only() {
                        let label = self.quiet_reason_label();
                        self.transition_to(StabilizerState::Quiet, &label, &mut actions);
                    } else {
                        self.transition_to(
                            StabilizerState::Warmup,
                            "Switched to cellular -> Warmup check",
                            &mut actions,
                        );
                        actions.push(Action::SendProbe {
                            tier: ProbeTier::L1Dns,
                            reason: PingReason::RadioEvent,
                        });
                    }
                    return actions;
                }

                if !prev_tether
                    && self.context.has_active_tethering()
                    && self.state == StabilizerState::Quiet
                {
                    self.idle_secs = 0;
                    if self.is_heal_only() {
                        self.state_reason = self.quiet_reason_label();
                    } else {
                        self.transition_to(
                            StabilizerState::Warmup,
                            "Tethering client connected -> Warmup",
                            &mut actions,
                        );
                        actions.push(Action::SendProbe {
                            tier: ProbeTier::L1Dns,
                            reason: PingReason::Warmup,
                        });
                    }
                } else if self.is_quiet_forced()
                    && !matches!(self.state, StabilizerState::Quiet | StabilizerState::Outage)
                {
                    self.transition_to(
                        StabilizerState::Quiet,
                        &self.quiet_reason_label(),
                        &mut actions,
                    );
                } else {
                    let _ = prev_live;
                    self.reconcile_mode_change(&mut actions);
                }
            }
            Event::NetworkCapabilitiesChanged {
                has_internet,
                is_validated,
            } => {
                let had_internet = self.has_internet;
                let had_validated = self.is_validated;
                self.has_internet = has_internet;
                self.is_validated = is_validated;

                if !self.is_running {
                    return actions;
                }

                if self.context.is_wifi_default && !self.context.has_active_tethering() {
                    return actions;
                }

                if has_internet && !is_validated {
                    if self.uptime_secs >= self.post_root_settle_until_uptime_secs
                        && !self.is_quiet_forced()
                        && !matches!(
                            self.state,
                            StabilizerState::Recovering | StabilizerState::Outage
                        )
                    {
                        self.enter_recovering("Lost NET_CAPABILITY_VALIDATED", &mut actions);
                    }
                } else if has_internet && is_validated {
                    self.consecutive_failed_ladders = 0;
                    self.outage_backoff_step = 0;
                    self.outage_wait_secs = 0;
                    self.post_root_settle_until_uptime_secs = 0;
                    if !had_validated
                        && matches!(
                            self.state,
                            StabilizerState::Recovering | StabilizerState::Outage
                        )
                    {
                        self.reset_recovery_episode();
                        let next_state = if self.is_quiet_forced() || self.is_heal_only() {
                            StabilizerState::Quiet
                        } else {
                            StabilizerState::Active
                        };
                        self.transition_to(next_state, "Cellular link validated", &mut actions);
                    } else if !had_internet && !self.is_quiet_forced() {
                        if self.is_heal_only() {
                            let label = self.quiet_reason_label();
                            self.transition_to(StabilizerState::Quiet, &label, &mut actions);
                        } else {
                            self.transition_to(
                                StabilizerState::Warmup,
                                "Cellular internet restored -> Warmup",
                                &mut actions,
                            );
                            actions.push(Action::SendProbe {
                                tier: ProbeTier::L1Dns,
                                reason: PingReason::RadioEvent,
                            });
                        }
                    }
                } else if !has_internet && matches!(self.state, StabilizerState::Recovering) {
                    self.reset_recovery_episode();
                    self.transition_to(
                        StabilizerState::Quiet,
                        "Cellular internet lost -> Quiet",
                        &mut actions,
                    );
                }
            }
            Event::RadioTopologyChanged { reason } => {
                if !self.is_running || !self.has_internet {
                    return actions;
                }
                if self.context.is_wifi_default && !self.context.has_active_tethering() {
                    return actions;
                }
                if self.state == StabilizerState::Recovering {
                    return actions;
                }
                let is_blocked = reason.contains("blocked") && !reason.contains("unblocked");
                let is_link_disruption = is_blocked || reason.contains("Link");
                if self.state == StabilizerState::Active && !is_link_disruption {
                    return actions;
                }
                if self.is_heal_only() && self.state != StabilizerState::Outage && !is_blocked {
                    return actions;
                }
                self.secs_since_last_probe = 0;
                if self.state == StabilizerState::Outage {
                    self.outage_wait_secs = 0;
                    actions.push(Action::SendProbe {
                        tier: ProbeTier::L2HttpHead,
                        reason: PingReason::RadioEvent,
                    });
                } else if !self.is_quiet_forced() {
                    let msg = format!("Radio event ({reason}) -> fresh check");
                    self.transition_to(StabilizerState::Warmup, &msg, &mut actions);
                    actions.push(Action::SendProbe {
                        tier: ProbeTier::L1Dns,
                        reason: PingReason::RadioEvent,
                    });
                }
            }
            Event::TrafficObserved {
                rx_bytes_delta,
                tx_bytes_delta,
            } => {
                self.last_rx_bytes_per_sec = rx_bytes_delta;
                self.last_tx_bytes_per_sec = tx_bytes_delta;
                if !self.is_running {
                    return actions;
                }

                let rx_is_strong = rx_bytes_delta >= ACTIVE_RX_THRESHOLD_BYTES_PER_SEC
                    || (tx_bytes_delta >= ACTIVE_RX_THRESHOLD_BYTES_PER_SEC
                        && rx_bytes_delta >= STALL_RX_CEILING_BYTES_PER_SEC);
                self.rx_history[self.rx_history_idx] = rx_is_strong;
                self.rx_history_idx = (self.rx_history_idx + 1) % 3;
                let rx_hits = self.rx_history.iter().filter(|&&hit| hit).count();

                let enters_active = rx_bytes_delta >= HEAVY_DOWNLOAD_BYTES_PER_SEC
                    || (self.state == StabilizerState::Quiet
                        && rx_bytes_delta >= ACTIVE_RX_THRESHOLD_BYTES_PER_SEC * 2)
                    || rx_hits >= 2;

                if enters_active || (self.state == StabilizerState::Active && rx_is_strong) {
                    self.idle_secs = 0;
                    self.stall_secs = 0;
                    self.eco_checks_sent = 0;
                    self.recent_rx_active_this_tick = true;
                    if rx_bytes_delta >= HEAVY_DOWNLOAD_BYTES_PER_SEC {
                        self.was_heavy_rx_prev_tick = true;
                    }

                    if !matches!(
                        self.state,
                        StabilizerState::Active | StabilizerState::Recovering
                    ) && !(self.context.is_wifi_default && !self.context.has_active_tethering())
                    {
                        self.consecutive_failed_ladders = 0;
                        self.outage_backoff_step = 0;
                        let rx_kbps = rx_bytes_delta as f64 / 1024.0;
                        let tx_kbps = tx_bytes_delta as f64 / 1024.0;
                        let msg = if rx_bytes_delta >= ACTIVE_RX_THRESHOLD_BYTES_PER_SEC {
                            format!("RX active ({rx_kbps:.1} KB/s) -> 0 pings")
                        } else {
                            format!("Traffic active (RX {rx_kbps:.1} / TX {tx_kbps:.1} KB/s) -> 0 pings")
                        };
                        self.transition_to(StabilizerState::Active, &msg, &mut actions);
                    }
                    return actions;
                }

                self.recent_rx_active_this_tick = false;

                // Silent link-freeze check: sustained outbound attempts/retransmits with ~zero RX.
                let is_tx_stalled = tx_bytes_delta >= STALL_TX_THRESHOLD_BYTES_PER_SEC
                    && rx_bytes_delta < STALL_RX_CEILING_BYTES_PER_SEC;
                if is_tx_stalled {
                    // Sudden cliff from >= 4 KB/s download straight into TX-only freeze gets a 1s head start.
                    if self.was_heavy_rx_prev_tick {
                        self.stall_secs = self.stall_secs.max(1);
                    }
                } else {
                    self.stall_secs = 0;
                }
                self.was_heavy_rx_prev_tick = false;
            }
            Event::ProbeCompleted {
                tier,
                reason,
                succeeded,
            } => {
                if !self.is_running {
                    return actions;
                }
                if succeeded {
                    self.consecutive_failed_ladders = 0;
                    self.outage_backoff_step = 0;
                    self.outage_wait_secs = 0;
                    if reason == PingReason::StallCheck {
                        self.stalls_healed = self.stalls_healed.saturating_add(1);
                    }
                    if matches!(
                        self.state,
                        StabilizerState::Recovering | StabilizerState::Outage
                    ) {
                        self.reset_recovery_episode();
                        let next_state = if self.is_quiet_forced() || self.is_heal_only() {
                            StabilizerState::Quiet
                        } else if self.is_live_eligible() {
                            StabilizerState::LiveKeepalive
                        } else {
                            StabilizerState::EcoTailCheck
                        };
                        self.transition_to(
                            next_state,
                            &format!("{tier:?} probe succeeded -> link healthy"),
                            &mut actions,
                        );
                    } else if self.is_heal_only() && self.state == StabilizerState::Warmup {
                        let label = self.quiet_reason_label();
                        self.transition_to(StabilizerState::Quiet, &label, &mut actions);
                    }
                } else {
                    // Any live/tail/warmup probe failure escalates into Recovering immediately,
                    // even if a residual traffic tick flickered state into Active while in flight.
                    if matches!(
                        self.state,
                        StabilizerState::Warmup
                            | StabilizerState::EcoTailCheck
                            | StabilizerState::LiveKeepalive
                            | StabilizerState::Active
                    ) && !self.context.is_wifi_default
                        && self.has_internet
                    {
                        self.enter_recovering(
                            "Probe failed -> verifying cellular link",
                            &mut actions,
                        );
                    } else if self.state == StabilizerState::Recovering
                        && !self.android_revalidation_sent
                    {
                        self.android_revalidation_sent = true;
                        self.android_revalidations = self.android_revalidations.saturating_add(1);
                        actions.push(Action::Log(
                            LogLevel::Warn,
                            "Fresh probe failed -> requesting Android network re-validation"
                                .to_owned(),
                        ));
                        actions.push(Action::RequestAndroidRevalidation);
                    }
                }
            }
            Event::ProbeSkippedLocalBind { tier: _, reason: _ } => {
                if !self.is_running {
                    return actions;
                }
                if self.is_quiet_forced()
                    && !matches!(self.state, StabilizerState::Quiet | StabilizerState::Outage)
                {
                    self.transition_to(
                        StabilizerState::Quiet,
                        &self.quiet_reason_label(),
                        &mut actions,
                    );
                }
            }
            Event::TickElapsed { secs } => {
                let dt = secs.max(1);
                if !self.is_running {
                    return actions;
                }
                self.uptime_secs = self.uptime_secs.saturating_add(dt);
                self.secs_since_last_probe = self.secs_since_last_probe.saturating_add(dt);
                self.secs_since_l2_verify = self.secs_since_l2_verify.saturating_add(dt);

                // Count fixed-4s pings avoided while resting in Active, Quiet, or Outage.
                if matches!(
                    self.state,
                    StabilizerState::Active | StabilizerState::Quiet | StabilizerState::Outage
                ) {
                    self.avoided_ping_accum_secs = self.avoided_ping_accum_secs.saturating_add(dt);
                    let earned = self.avoided_ping_accum_secs / 4;
                    if earned > 0 {
                        self.avoided_ping_accum_secs %= 4;
                        self.avoided_fixed_4s_pings =
                            self.avoided_fixed_4s_pings.saturating_add(earned);
                    }
                }

                if self.context.is_wifi_default && !self.context.has_active_tethering() {
                    if self.state != StabilizerState::Quiet {
                        self.transition_to(
                            StabilizerState::Quiet,
                            "Wi-Fi default -> Quiet (0 pings)",
                            &mut actions,
                        );
                    }
                    return actions;
                }

                // Stall counter evaluation when TX >= 128 B/s and RX < 64 B/s.
                let is_tx_stalled = self.last_tx_bytes_per_sec >= STALL_TX_THRESHOLD_BYTES_PER_SEC
                    && self.last_rx_bytes_per_sec < STALL_RX_CEILING_BYTES_PER_SEC;
                if is_tx_stalled
                    && self.has_internet
                    && !matches!(
                        self.state,
                        StabilizerState::Recovering | StabilizerState::Outage
                    )
                {
                    self.stall_secs = self.stall_secs.saturating_add(dt);
                    if self.stall_secs >= STALL_CONSECUTIVE_SECS {
                        self.stall_secs = 0;
                        self.stalls_detected = self.stalls_detected.saturating_add(1);
                        self.enter_recovering(
                            "Silent TX stall detected (RX stalled 2s)",
                            &mut actions,
                        );
                        return actions;
                    }
                } else {
                    self.stall_secs = 0;
                }

                // Recovering ladder progression.
                if self.state == StabilizerState::Recovering {
                    self.handle_recovering_tick(dt, &mut actions);
                    return actions;
                }

                // Carrier Outage exponential backoff (30s -> 60s -> 120s -> 300s).
                if self.state == StabilizerState::Outage {
                    self.outage_wait_secs = self.outage_wait_secs.saturating_add(dt);
                    let target_backoff = outage_backoff_secs(self.outage_backoff_step);
                    if self.outage_wait_secs >= target_backoff {
                        self.outage_wait_secs = 0;
                        self.outage_backoff_step =
                            self.outage_backoff_step.saturating_add(1).min(3);
                        self.state_reason = format!(
                            "No carrier data -> next retry in {}s",
                            outage_backoff_secs(self.outage_backoff_step)
                        );
                        actions.push(Action::SendProbe {
                            tier: ProbeTier::L1Dns,
                            reason: PingReason::OutageCheck,
                        });
                    }
                    return actions;
                }

                // Active RX traffic stays silent (0 pings).
                if self.recent_rx_active_this_tick {
                    self.recent_rx_active_this_tick = false;
                    self.idle_secs = 0;
                    if self.state != StabilizerState::Active {
                        self.transition_to(
                            StabilizerState::Active,
                            "RX traffic active -> 0 pings",
                            &mut actions,
                        );
                    }
                    return actions;
                }

                let idle_dt = if self.warmup_just_entered {
                    self.warmup_just_entered = false;
                    dt.min(1)
                } else {
                    dt
                };
                self.idle_secs = self.idle_secs.saturating_add(idle_dt);

                // Screen-off heavy download grace period before entering Quiet.
                if self.state == StabilizerState::Active
                    && !self.is_screen_on
                    && !self.context.has_active_tethering()
                {
                    let grace = self
                        .config
                        .idle_threshold_secs
                        .max(SCREEN_OFF_DOWNLOAD_GRACE_SECS);
                    if self.idle_secs >= grace {
                        self.transition_to(
                            StabilizerState::Quiet,
                            "Screen-off transfer finished -> Quiet (0 pings)",
                            &mut actions,
                        );
                    }
                    return actions;
                }

                if self.is_quiet_forced() {
                    if self.state != StabilizerState::Quiet {
                        self.transition_to(
                            StabilizerState::Quiet,
                            &self.quiet_reason_label(),
                            &mut actions,
                        );
                    } else {
                        self.state_reason = self.quiet_reason_label();
                    }
                    return actions;
                }

                if !self.has_internet {
                    return actions;
                }

                if !self.is_validated {
                    if self.uptime_secs >= self.post_root_settle_until_uptime_secs {
                        self.enter_recovering("Unvalidated cellular link", &mut actions);
                    }
                    return actions;
                }

                // Decide Live vs Eco once idle beyond idle_threshold_secs.
                if self.idle_secs < self.config.idle_threshold_secs {
                    return actions;
                }

                if self.is_live_eligible() {
                    let label = self.live_reason_label();
                    if self.state != StabilizerState::LiveKeepalive {
                        // Align probe cadence with idle time since traffic stopped so the first
                        // keepalive lands inside `radio_tail_secs` instead of waiting idle + full interval.
                        self.secs_since_last_probe = self.secs_since_last_probe.max(self.idle_secs);
                        self.transition_to(StabilizerState::LiveKeepalive, &label, &mut actions);
                    } else {
                        self.state_reason = label;
                    }

                    if self.secs_since_last_probe >= self.next_live_interval_secs {
                        self.secs_since_last_probe = 0;
                        self.recompute_next_live_interval();
                        let (tier, reason) = if self.secs_since_l2_verify >= LIVE_L2_REVERIFY_SECS {
                            self.secs_since_l2_verify = 0;
                            (ProbeTier::L2HttpHead, PingReason::LiveReverify)
                        } else {
                            (ProbeTier::L1Dns, PingReason::LiveKeepalive)
                        };
                        actions.push(Action::SendProbe { tier, reason });
                    }
                    return actions;
                }

                // Eco mode: send at most MAX_ECO_TAIL_CHECKS inside radio_tail_secs, then Quiet (0 pings).
                // Under low battery / Battery Saver, skip tail checks and enter Quiet immediately once idle.
                if self.state == StabilizerState::Quiet {
                    self.state_reason = self.quiet_reason_label();
                    return actions;
                }

                if self.context.low_battery_or_saver || self.is_heal_only() {
                    let label = self.quiet_reason_label();
                    self.transition_to(StabilizerState::Quiet, &label, &mut actions);
                    return actions;
                }

                if self.eco_checks_sent < MAX_ECO_TAIL_CHECKS {
                    let eco_step_secs = (self.config.radio_tail_secs / 2).max(3);
                    let elapsed_for_eco = if self.eco_checks_sent == 0 {
                        self.secs_since_last_probe.max(self.idle_secs)
                    } else {
                        self.secs_since_last_probe
                    };
                    if elapsed_for_eco >= eco_step_secs {
                        self.secs_since_last_probe = 0;
                        self.eco_checks_sent = self.eco_checks_sent.saturating_add(1);
                        let check_num = self.eco_checks_sent;
                        self.transition_to(
                            StabilizerState::EcoTailCheck,
                            &format!("Eco tail check {check_num}/{MAX_ECO_TAIL_CHECKS}"),
                            &mut actions,
                        );
                        actions.push(Action::SendProbe {
                            tier: ProbeTier::L1Dns,
                            reason: PingReason::EcoTail,
                        });
                    }
                } else {
                    self.transition_to(
                        StabilizerState::Quiet,
                        &self.quiet_reason_label(),
                        &mut actions,
                    );
                }
            }
        }

        actions
    }

    fn handle_recovering_tick(&mut self, dt: u64, actions: &mut Vec<Action>) {
        if self.is_quiet_forced() && !self.context.has_active_tethering() {
            self.reset_recovery_episode();
            self.transition_to(
                StabilizerState::Quiet,
                "Paused recovery while screen off -> Quiet",
                actions,
            );
            return;
        }

        self.recovering_secs = self.recovering_secs.saturating_add(dt);

        // Step 2: non-root Android re-validation after 3s.
        if !self.android_revalidation_sent && self.recovering_secs >= 3 {
            self.android_revalidation_sent = true;
            self.android_revalidations = self.android_revalidations.saturating_add(1);
            actions.push(Action::Log(
                LogLevel::Warn,
                "Recovery step 2 -> calling Android reportNetworkConnectivity(false)".to_owned(),
            ));
            actions.push(Action::RequestAndroidRevalidation);
        }

        // Step 3: gated Shizuku/root toggle after recovery_timeout_secs (3x grace if tethering clients).
        let effective_timeout = if self.context.has_active_tethering() {
            self.config.recovery_timeout_secs.saturating_mul(3)
        } else {
            self.config.recovery_timeout_secs
        };

        if self.recovering_secs >= effective_timeout && !self.root_recovery_sent_this_episode {
            if self.can_trigger_root_recovery() {
                self.root_recovery_sent_this_episode = true;
                self.post_root_settle_until_uptime_secs = self.uptime_secs.saturating_add(10);
                self.root_recovery_history_secs.push_back(self.uptime_secs);
                self.recoveries_triggered = self.recoveries_triggered.saturating_add(1);
                actions.push(Action::Log(
                    LogLevel::Warn,
                    format!(
                        "Link unvalidated for {}s -> running gated cellular data toggle",
                        self.recovering_secs
                    ),
                ));
                actions.push(Action::TriggerRootRecovery);
            } else if self.recovering_secs >= effective_timeout + 5 {
                // Ladder exhausted (cooldown, call active, or root disabled) -> advance outage tracking.
                self.escalate_recovery_failure(actions);
            }
            return;
        }

        // Step 4: if root ran and link still hasn't recovered within +8s, count failed ladder -> Outage.
        if self.root_recovery_sent_this_episode && self.recovering_secs >= effective_timeout + 8 {
            self.escalate_recovery_failure(actions);
        }
    }

    fn escalate_recovery_failure(&mut self, actions: &mut Vec<Action>) {
        self.consecutive_failed_ladders = self.consecutive_failed_ladders.saturating_add(1);
        self.reset_recovery_episode();

        if self.consecutive_failed_ladders >= 2 {
            self.outage_backoff_step = 0;
            self.outage_wait_secs = 0;
            self.transition_to(
                StabilizerState::Outage,
                "No carrier data -> Outage backoff (30s)",
                actions,
            );
        } else {
            self.transition_to(
                StabilizerState::Recovering,
                "Recovery attempt 2 -> fresh L2 probe",
                actions,
            );
            actions.push(Action::SendProbe {
                tier: ProbeTier::L2HttpHead,
                reason: PingReason::Recovery,
            });
        }
    }

    fn can_trigger_root_recovery(&mut self) -> bool {
        if !self.config.root_recovery_enabled {
            return false;
        }
        if self.context.in_call {
            return false;
        }
        while let Some(&oldest) = self.root_recovery_history_secs.front() {
            if self.uptime_secs.saturating_sub(oldest) >= ROOT_RECOVERY_WINDOW_SECS {
                self.root_recovery_history_secs.pop_front();
            } else {
                break;
            }
        }
        if self.root_recovery_history_secs.len() >= MAX_ROOT_RECOVERIES_PER_WINDOW {
            return false;
        }
        if let Some(&last) = self.root_recovery_history_secs.back() {
            if self.uptime_secs.saturating_sub(last) < ROOT_RECOVERY_COOLDOWN_SECS {
                return false;
            }
        }
        true
    }

    fn enter_recovering(&mut self, reason: &str, actions: &mut Vec<Action>) {
        self.reset_recovery_episode();
        self.secs_since_last_probe = 0;
        self.transition_to(
            StabilizerState::Recovering,
            &format!("{reason} -> Recovering L2 probe"),
            actions,
        );
        actions.push(Action::SendProbe {
            tier: ProbeTier::L2HttpHead,
            reason: PingReason::StallCheck,
        });
    }

    fn reset_episode_timers(&mut self) {
        self.idle_secs = 0;
        self.stall_secs = 0;
        self.secs_since_last_probe = 0;
        self.secs_since_l2_verify = 0;
        self.eco_checks_sent = 0;
        self.recent_rx_active_this_tick = false;
        self.was_heavy_rx_prev_tick = false;
        self.post_root_settle_until_uptime_secs = 0;
        self.reset_recovery_episode();
    }

    fn reset_recovery_episode(&mut self) {
        self.recovering_secs = 0;
        self.android_revalidation_sent = false;
        self.root_recovery_sent_this_episode = false;
    }

    fn recompute_next_live_interval(&mut self) {
        let base = self.config.radio_tail_secs.saturating_sub(2).clamp(4, 10);
        // Deterministic xorshift64 ±20% jitter.
        self.rng_state ^= self.rng_state << 13;
        self.rng_state ^= self.rng_state >> 7;
        self.rng_state ^= self.rng_state << 17;
        let pct_offset = (self.rng_state % 41) as i64 - 20; // [-20, +20]
        let jittered = ((base as i64) * (100 + pct_offset) + 50) / 100;
        self.next_live_interval_secs = (jittered.max(3)) as u64;
    }

    fn quiet_reason_label(&self) -> String {
        if self.context.is_wifi_default && !self.context.has_active_tethering() {
            "Quiet · Wi-Fi default · 0 pings".to_owned()
        } else if self.context.is_doze {
            "Quiet · Doze · 0 pings".to_owned()
        } else if !self.is_screen_on && !self.context.has_active_tethering() {
            "Quiet · screen off · 0 pings".to_owned()
        } else if self.is_heal_only() {
            "Quiet · heal-only · 0 pings".to_owned()
        } else if self.context.low_battery_or_saver {
            "Quiet · battery saver · 0 pings".to_owned()
        } else if !self.is_screen_on {
            "Quiet · screen off · 0 pings".to_owned()
        } else {
            format!("Quiet · idle {}s · 0 pings", self.idle_secs)
        }
    }

    fn live_reason_label(&self) -> String {
        let base = self.config.radio_tail_secs.saturating_sub(2).clamp(4, 10);
        if self.context.hotspot_clients > 0 {
            format!(
                "Live · hotspot ({}) · ~{base}s",
                self.context.hotspot_clients
            )
        } else if self.context.is_usb_or_bt_tethering {
            format!("Live · USB/BT tether · ~{base}s")
        } else if self.context.in_call {
            format!("Live · in call · ~{base}s")
        } else if self.context.boost_active {
            format!("Live · Boost active · ~{base}s")
        } else if self.context.live_app_active {
            format!("Live · game/app · ~{base}s")
        } else {
            format!("Live · profile · ~{base}s")
        }
    }

    fn transition_to(&mut self, next: StabilizerState, reason: &str, actions: &mut Vec<Action>) {
        self.state_reason = reason.to_owned();
        if self.state != next {
            self.state = next;
            self.warmup_just_entered = next == StabilizerState::Warmup;
            if matches!(next, StabilizerState::Quiet | StabilizerState::Stopped) {
                self.rx_history = [false; 3];
                self.rx_history_idx = 0;
            }
            actions.push(Action::Log(LogLevel::Info, reason.to_owned()));
        }
    }
}

fn outage_backoff_secs(step: u8) -> u64 {
    match step {
        0 => 30,
        1 => 60,
        2 => 120,
        _ => 300,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started_machine(profile: StabilizerProfile) -> StateMachine {
        let cfg = StabilizerConfig {
            profile,
            ..StabilizerConfig::default()
        };
        let mut sm = StateMachine::new(cfg);
        sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: true,
        });
        sm.handle_event(Event::ScreenChanged {
            is_screen_on: true,
            is_unlocked: true,
        });
        sm.handle_event(Event::Start);
        sm
    }

    fn count_probes(actions: &[Action]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, Action::SendProbe { .. }))
            .count()
    }

    #[test]
    fn eco_mode_sends_at_most_two_tail_checks_then_zero_in_quiet() {
        let mut sm = started_machine(StabilizerProfile::Eco);

        // 2 seconds of real RX settles into Active via 2-of-3 hysteresis
        for _ in 0..2 {
            sm.handle_event(Event::TrafficObserved {
                rx_bytes_delta: 2_048,
                tx_bytes_delta: 200,
            });
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            assert_eq!(count_probes(&a), 0);
        }
        assert_eq!(sm.state(), StabilizerState::Active);

        // Idle for 2 minutes on screen -> <= 2 probes total, then Quiet (0 pings)
        let mut total_probes = 0;
        for _ in 0..120 {
            sm.handle_event(Event::TrafficObserved {
                rx_bytes_delta: 120,
                tx_bytes_delta: 40,
            });
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            total_probes += count_probes(&a);
        }

        assert_eq!(sm.state(), StabilizerState::Quiet);
        assert!(
            total_probes <= MAX_ECO_TAIL_CHECKS as usize,
            "Eco sent {total_probes} probes; expected <= {MAX_ECO_TAIL_CHECKS}"
        );
        assert!(sm.avoided_fixed_4s_pings() > 15);
    }

    #[test]
    fn tx_stall_detection_heals_silent_freeze_without_false_upload_trigger() {
        let mut sm = started_machine(StabilizerProfile::Auto);
        sm.handle_event(Event::TrafficObserved {
            rx_bytes_delta: 8_192,
            tx_bytes_delta: 1_024,
        });
        sm.handle_event(Event::TickElapsed { secs: 1 });
        assert_eq!(sm.state(), StabilizerState::Active);

        // Slow upload receiving TCP ACKs (>= 64 B/s RX) does NOT false-stall.
        for _ in 0..10 {
            sm.handle_event(Event::TrafficObserved {
                rx_bytes_delta: 120,
                tx_bytes_delta: 2_048,
            });
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            assert_ne!(sm.state(), StabilizerState::Recovering);
            assert_eq!(count_probes(&a), 0);
        }

        // Silent freeze: TX retransmitting (>= 256 B/s) while RX < 64 B/s for 3s -> immediate L2 probe!
        let mut triggered = false;
        for _ in 0..3 {
            sm.handle_event(Event::TrafficObserved {
                rx_bytes_delta: 0,
                tx_bytes_delta: 600,
            });
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            if a.iter().any(|act| {
                matches!(
                    act,
                    Action::SendProbe {
                        tier: ProbeTier::L2HttpHead,
                        reason: PingReason::StallCheck
                    }
                )
            }) {
                triggered = true;
            }
        }
        assert!(
            triggered,
            "TX stall must trigger instant L2 StallCheck probe"
        );
        assert_eq!(sm.state(), StabilizerState::Recovering);
        assert_eq!(sm.stalls_detected(), 1);

        // Probe succeeds -> increments stalls_healed and exits Recovering
        sm.handle_event(Event::ProbeCompleted {
            tier: ProbeTier::L2HttpHead,
            reason: PingReason::StallCheck,
            succeeded: true,
        });
        assert_eq!(sm.stalls_healed(), 1);
        assert_ne!(sm.state(), StabilizerState::Recovering);
    }

    #[test]
    fn hotspot_with_zero_clients_sleeps_while_one_client_runs_live_even_screen_off() {
        let mut sm = started_machine(StabilizerProfile::Auto);

        // Hotspot ON with 0 clients & screen off -> Quiet (0 pings)
        sm.handle_event(Event::ContextChanged(RuntimeContext {
            hotspot_clients: 0,
            ..RuntimeContext::default()
        }));
        sm.handle_event(Event::ScreenChanged {
            is_screen_on: false,
            is_unlocked: false,
        });
        assert_eq!(sm.state(), StabilizerState::Quiet);
        for _ in 0..20 {
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            assert_eq!(count_probes(&a), 0);
        }

        // Client connects -> LiveKeepalive even with screen off
        sm.handle_event(Event::ContextChanged(RuntimeContext {
            hotspot_clients: 1,
            ..RuntimeContext::default()
        }));
        let mut live_probes = 0;
        for _ in 0..25 {
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            live_probes += count_probes(&a);
        }
        assert_eq!(sm.state(), StabilizerState::LiveKeepalive);
        assert!(live_probes >= 2);

        // Wifi becomes default -> Quiet (0 pings)
        sm.handle_event(Event::ContextChanged(RuntimeContext {
            is_wifi_default: true,
            hotspot_clients: 0,
            ..RuntimeContext::default()
        }));
        assert_eq!(sm.state(), StabilizerState::Quiet);
    }

    #[test]
    fn recovery_ladder_respects_call_guard_cooldown_max_3_per_15m_and_enters_outage() {
        let cfg = StabilizerConfig {
            profile: StabilizerProfile::Live,
            root_recovery_enabled: true,
            recovery_timeout_secs: 5,
            ..StabilizerConfig::default()
        };
        let mut sm = StateMachine::new(cfg);
        sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: true,
        });
        sm.handle_event(Event::Start);

        // During a voice call, root toggle MUST NEVER fire
        sm.handle_event(Event::ContextChanged(RuntimeContext {
            in_call: true,
            ..RuntimeContext::default()
        }));
        sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: false,
        });
        let mut saw_reval = false;
        let mut saw_root = false;
        for _ in 0..20 {
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            if a.contains(&Action::RequestAndroidRevalidation) {
                saw_reval = true;
            }
            if a.contains(&Action::TriggerRootRecovery) {
                saw_root = true;
            }
        }
        assert!(saw_reval, "Android non-root revalidation should still run");
        assert!(!saw_root, "Root recovery must be blocked during calls");
        assert_eq!(sm.state(), StabilizerState::Outage);

        // Restore validation, leave call, and verify root cooldown + max 3 per 15m + Outage
        sm.handle_event(Event::ContextChanged(RuntimeContext::default()));
        sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: true,
        });

        let mut root_count = 0;
        for episode in 0..5 {
            sm.handle_event(Event::NetworkCapabilitiesChanged {
                has_internet: true,
                is_validated: false,
            });
            for _ in 0..16 {
                let a = sm.handle_event(Event::TickElapsed { secs: 1 });
                if a.contains(&Action::TriggerRootRecovery) {
                    root_count += 1;
                }
            }
            if episode < 4 {
                // Re-validate and advance 130s only for first 3 episodes; test cooldown on 4th
                sm.handle_event(Event::NetworkCapabilitiesChanged {
                    has_internet: true,
                    is_validated: true,
                });
                let step = if episode < 2 { 130 } else { 30 };
                sm.handle_event(Event::TickElapsed { secs: step });
            }
        }
        assert_eq!(
            root_count, 3,
            "Root recovery must enforce 120s cooldown and max 3 per 15m"
        );
    }

    #[test]
    fn fuzzed_fsm_invariants_hold_across_2000_random_traces() {
        let mut seed = 0xCAFE_BABE_1234_5678u64;
        let mut next_u64 = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        for _ in 0..2_000 {
            let profile = match next_u64() % 4 {
                0 => StabilizerProfile::Auto,
                1 => StabilizerProfile::Eco,
                2 => StabilizerProfile::HealOnly,
                _ => StabilizerProfile::Live,
            };
            let mut sm = StateMachine::new(StabilizerConfig {
                profile,
                root_recovery_enabled: (next_u64() % 2) == 0,
                ..StabilizerConfig::default()
            });
            sm.handle_event(Event::Start);

            for _ in 0..100 {
                let ev = match next_u64() % 7 {
                    0 => Event::ScreenChanged {
                        is_screen_on: (next_u64() % 2) == 0,
                        is_unlocked: (next_u64() % 2) == 0,
                    },
                    1 => Event::ContextChanged(RuntimeContext {
                        is_wifi_default: (next_u64() % 4) == 0,
                        in_call: (next_u64() % 8) == 0,
                        hotspot_clients: (next_u64() % 3) as u32,
                        is_usb_or_bt_tethering: (next_u64() % 6) == 0,
                        is_charging: (next_u64() % 2) == 0,
                        low_battery_or_saver: (next_u64() % 5) == 0,
                        is_doze: (next_u64() % 6) == 0,
                        boost_active: (next_u64() % 5) == 0,
                        live_app_active: (next_u64() % 6) == 0,
                    }),
                    2 => Event::NetworkCapabilitiesChanged {
                        has_internet: (next_u64() % 5) != 0,
                        is_validated: (next_u64() % 3) != 0,
                    },
                    3 => Event::TrafficObserved {
                        rx_bytes_delta: next_u64() % 12_000,
                        tx_bytes_delta: next_u64() % 2_000,
                    },
                    4 => Event::ProbeCompleted {
                        tier: ProbeTier::L1Dns,
                        reason: PingReason::LiveKeepalive,
                        succeeded: (next_u64() % 4) != 0,
                    },
                    5 => Event::RadioTopologyChanged {
                        reason: "5G handover".to_owned(),
                    },
                    _ => Event::TickElapsed {
                        secs: 1 + (next_u64() % 5),
                    },
                };

                let actions = sm.handle_event(ev);
                // Invariant 1: Never emit SendProbe when resting in Active or Quiet.
                if matches!(sm.state(), StabilizerState::Active | StabilizerState::Quiet) {
                    assert_eq!(
                        count_probes(&actions),
                        0,
                        "violated 0-probe invariant in {:?}",
                        sm.state()
                    );
                }
                // Invariant 2: Never trigger root recovery while in a call.
                if sm.context().in_call {
                    assert!(
                        !actions.contains(&Action::TriggerRootRecovery),
                        "root recovery triggered during call"
                    );
                }
            }
        }
    }

    #[test]
    fn battery_saver_preserves_active_hysteresis_and_avoids_flapping() {
        let mut sm = started_machine(StabilizerProfile::Auto);
        sm.handle_event(Event::ContextChanged(RuntimeContext {
            low_battery_or_saver: true,
            ..RuntimeContext::default()
        }));

        // Screen wake while in Battery Saver must keep Warmup probe intact (not snap to Quiet on 1s pause).
        sm.handle_event(Event::ScreenChanged {
            is_screen_on: false,
            is_unlocked: false,
        });
        let wake_actions = sm.handle_event(Event::ScreenChanged {
            is_screen_on: true,
            is_unlocked: true,
        });
        assert_eq!(sm.state(), StabilizerState::Warmup);
        assert_eq!(count_probes(&wake_actions), 1);

        // Active download burst + 2-second micro-pauses must stay Active (0 flapping, 0 pings).
        sm.handle_event(Event::TrafficObserved {
            rx_bytes_delta: 69_000,
            tx_bytes_delta: 1_200,
        });
        sm.handle_event(Event::TickElapsed { secs: 1 });
        assert_eq!(sm.state(), StabilizerState::Active);

        for _ in 0..2 {
            sm.handle_event(Event::TrafficObserved {
                rx_bytes_delta: 100,
                tx_bytes_delta: 20,
            });
            let pause_actions = sm.handle_event(Event::TickElapsed { secs: 1 });
            assert_eq!(
                sm.state(),
                StabilizerState::Active,
                "micro-pause < idle_threshold_secs must stay Active"
            );
            assert_eq!(count_probes(&pause_actions), 0);
        }

        // After full idle_threshold_secs (4s), settles ONCE into Quiet with 0 EcoTailCheck pings.
        let mut extra_tail_probes = 0;
        for _ in 0..4 {
            sm.handle_event(Event::TrafficObserved {
                rx_bytes_delta: 40,
                tx_bytes_delta: 0,
            });
            let idle_actions = sm.handle_event(Event::TickElapsed { secs: 1 });
            extra_tail_probes += count_probes(&idle_actions);
        }
        assert_eq!(sm.state(), StabilizerState::Quiet);
        assert!(sm.state_reason().contains("battery saver"));
        assert_eq!(extra_tail_probes, 0);

        // A single tiny upload/ACK residue after Quiet must NOT ghost-wake Active.
        sm.handle_event(Event::TrafficObserved {
            rx_bytes_delta: 500,
            tx_bytes_delta: 1_200,
        });
        sm.handle_event(Event::TickElapsed { secs: 10 });
        assert_eq!(sm.state(), StabilizerState::Quiet);
    }

    #[test]
    fn live_and_boost_override_battery_saver_and_warmup_survives_wake_tick() {
        let mut sm = started_machine(StabilizerProfile::Auto);
        sm.handle_event(Event::ContextChanged(RuntimeContext {
            low_battery_or_saver: true,
            ..RuntimeContext::default()
        }));

        // Screen off -> Screen wake -> immediate 7s tick leftover from Quiet must NOT kill Warmup.
        sm.handle_event(Event::ScreenChanged {
            is_screen_on: false,
            is_unlocked: false,
        });
        sm.handle_event(Event::ScreenChanged {
            is_screen_on: true,
            is_unlocked: true,
        });
        assert_eq!(sm.state(), StabilizerState::Warmup);
        sm.handle_event(Event::TickElapsed { secs: 7 });
        assert_eq!(
            sm.state(),
            StabilizerState::Warmup,
            "first post-wake tick must preserve Warmup window"
        );

        // Settle to Quiet · battery saver, then turn on Boost 30m -> immediately enters LiveKeepalive + probes!
        for _ in 0..4 {
            sm.handle_event(Event::TickElapsed { secs: 1 });
        }
        assert_eq!(sm.state(), StabilizerState::Quiet);

        let boost_actions = sm.handle_event(Event::ContextChanged(RuntimeContext {
            low_battery_or_saver: true,
            boost_active: true,
            ..RuntimeContext::default()
        }));
        assert_eq!(sm.state(), StabilizerState::LiveKeepalive);
        assert_eq!(count_probes(&boost_actions), 1);

        // Cancel Boost back to Auto -> returns cleanly to Quiet · battery saver.
        sm.handle_event(Event::ContextChanged(RuntimeContext {
            low_battery_or_saver: true,
            boost_active: false,
            ..RuntimeContext::default()
        }));
        assert_eq!(sm.state(), StabilizerState::Quiet);

        // Explicitly switch profile to Live while Battery Saver is ON -> immediately enters LiveKeepalive!
        let live_cfg = StabilizerConfig {
            profile: StabilizerProfile::Live,
            ..StabilizerConfig::default()
        };
        let cfg_actions = sm.update_config(live_cfg);
        assert_eq!(sm.state(), StabilizerState::LiveKeepalive);
        assert_eq!(count_probes(&cfg_actions), 1);
    }

    #[test]
    fn probe_failure_during_active_flicker_still_enters_recovering() {
        let mut sm = started_machine(StabilizerProfile::Live);
        // Move into Active via strong RX.
        sm.handle_event(Event::TrafficObserved {
            rx_bytes_delta: 6_000,
            tx_bytes_delta: 300,
        });
        sm.handle_event(Event::TickElapsed { secs: 1 });
        assert_eq!(sm.state(), StabilizerState::Active);

        // An in-flight LiveKeepalive/Warmup probe timing out must still trigger Recovering.
        let fail_actions = sm.handle_event(Event::ProbeCompleted {
            tier: ProbeTier::L1Dns,
            reason: PingReason::LiveKeepalive,
            succeeded: false,
        });
        assert_eq!(sm.state(), StabilizerState::Recovering);
        assert!(fail_actions.iter().any(|a| matches!(
            a,
            Action::SendProbe {
                tier: ProbeTier::L2HttpHead,
                reason: PingReason::StallCheck
            }
        )));
    }

    #[test]
    fn heavy_rx_cliff_triggers_fast_stall_without_slow_upload_false_positive() {
        let mut sm = started_machine(StabilizerProfile::Auto);
        // Heavy download burst (>= 4 KB/s).
        sm.handle_event(Event::TrafficObserved {
            rx_bytes_delta: 16_384,
            tx_bytes_delta: 500,
        });
        sm.handle_event(Event::TickElapsed { secs: 1 });
        assert_eq!(sm.state(), StabilizerState::Active);

        // Slow upload with steady TCP ACKs (>= 64 B/s RX) does NOT false-stall even above 128 B/s TX.
        for _ in 0..5 {
            sm.handle_event(Event::TrafficObserved {
                rx_bytes_delta: 80,
                tx_bytes_delta: 400,
            });
            sm.handle_event(Event::TickElapsed { secs: 1 });
            assert_ne!(sm.state(), StabilizerState::Recovering);
        }

        // Re-establish heavy download burst then drop straight into a 150 B/s TX-only freeze.
        sm.handle_event(Event::TrafficObserved {
            rx_bytes_delta: 12_000,
            tx_bytes_delta: 400,
        });
        sm.handle_event(Event::TickElapsed { secs: 1 });

        sm.handle_event(Event::TrafficObserved {
            rx_bytes_delta: 0,
            tx_bytes_delta: 150,
        });
        let a = sm.handle_event(Event::TickElapsed { secs: 1 });
        assert_eq!(sm.state(), StabilizerState::Recovering);
        assert!(a.iter().any(|act| matches!(
            act,
            Action::SendProbe {
                tier: ProbeTier::L2HttpHead,
                reason: PingReason::StallCheck
            }
        )));
    }

    #[test]
    fn post_root_settle_window_prevents_premature_unvalidated_recovery() {
        let cfg = StabilizerConfig {
            profile: StabilizerProfile::Live,
            root_recovery_enabled: true,
            recovery_timeout_secs: 5,
            ..StabilizerConfig::default()
        };
        let mut sm = StateMachine::new(cfg);
        sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: true,
        });
        sm.handle_event(Event::ScreenChanged {
            is_screen_on: true,
            is_unlocked: true,
        });
        sm.handle_event(Event::Start);

        // Drop validation to trigger Step 3 root toggle at t=5s.
        sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: false,
        });
        let mut root_triggered = false;
        for _ in 0..5 {
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            if a.contains(&Action::TriggerRootRecovery) {
                root_triggered = true;
            }
        }
        assert!(root_triggered);

        // If an L2 probe succeeds while Android validation is still catching up during the 10s
        // settle window, Losing/Staying !is_validated must NOT immediately yank back into Recovering.
        sm.handle_event(Event::ProbeCompleted {
            tier: ProbeTier::L2HttpHead,
            reason: PingReason::Recovery,
            succeeded: true,
        });
        assert_ne!(sm.state(), StabilizerState::Recovering);

        for _ in 0..5 {
            sm.handle_event(Event::TickElapsed { secs: 1 });
            assert_ne!(
                sm.state(),
                StabilizerState::Recovering,
                "must let modem reattach inside post-root settle window"
            );
        }

        // Once the 10s settle window passes while still !is_validated, recovery resumes cleanly.
        for _ in 0..6 {
            sm.handle_event(Event::TickElapsed { secs: 1 });
        }
        assert_eq!(sm.state(), StabilizerState::Recovering);
    }

    #[test]
    fn live_keepalive_first_probe_aligns_with_radio_tail() {
        let cfg = StabilizerConfig {
            profile: StabilizerProfile::Live,
            radio_tail_secs: 8,
            idle_threshold_secs: 4,
            ..StabilizerConfig::default()
        };
        let mut sm = StateMachine::new(cfg);
        sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: true,
        });
        sm.handle_event(Event::ScreenChanged {
            is_screen_on: true,
            is_unlocked: true,
        });
        sm.handle_event(Event::Start);

        // Drive Active via RX burst.
        sm.handle_event(Event::TrafficObserved {
            rx_bytes_delta: 8_000,
            tx_bytes_delta: 400,
        });
        sm.handle_event(Event::TickElapsed { secs: 1 });
        assert_eq!(sm.state(), StabilizerState::Active);

        // Wait idle: the first LiveKeepalive probe must fire within radio_tail_secs (<= 8s),
        // rather than idle_threshold_secs (4s) + full interval (~6s) = 10s.
        let mut secs_until_first_probe = None;
        for sec in 1..=8 {
            sm.handle_event(Event::TrafficObserved {
                rx_bytes_delta: 0,
                tx_bytes_delta: 0,
            });
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            if count_probes(&a) > 0 && secs_until_first_probe.is_none() {
                secs_until_first_probe = Some(sec);
            }
        }
        let fired_at =
            secs_until_first_probe.expect("LiveKeepalive probe should fire within 8s tail");
        assert!(
            fired_at <= 8,
            "first keepalive fired at {fired_at}s (past 8s RRC tail)"
        );
    }

    #[test]
    fn heal_only_sends_zero_pings_when_healthy_and_heals_drops_immediately() {
        let mut sm = StateMachine::new(StabilizerConfig {
            profile: StabilizerProfile::HealOnly,
            ..StabilizerConfig::default()
        });
        sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: true,
        });
        sm.handle_event(Event::ScreenChanged {
            is_screen_on: true,
            is_unlocked: true,
        });

        // 1. Start on healthy cellular link -> Quiet with 0 pings (no Warmup).
        let start_actions = sm.handle_event(Event::Start);
        assert_eq!(count_probes(&start_actions), 0);
        assert_eq!(sm.state(), StabilizerState::Quiet);
        assert!(sm.state_reason().contains("heal-only"));

        // 2. Screen off -> screen wake/unlock -> 0 pings.
        assert_eq!(
            count_probes(&sm.handle_event(Event::ScreenChanged {
                is_screen_on: false,
                is_unlocked: false,
            })),
            0
        );
        assert_eq!(
            count_probes(&sm.handle_event(Event::ScreenChanged {
                is_screen_on: true,
                is_unlocked: true,
            })),
            0
        );
        assert_eq!(sm.state(), StabilizerState::Quiet);

        // 3. Video/browsing traffic burst followed by 60s idle -> Active -> Quiet with 0 EcoTail pings.
        sm.handle_event(Event::TrafficObserved {
            rx_bytes_delta: 40_000,
            tx_bytes_delta: 1_200,
        });
        assert_eq!(
            count_probes(&sm.handle_event(Event::TickElapsed { secs: 1 })),
            0
        );
        assert_eq!(sm.state(), StabilizerState::Active);

        for _ in 0..60 {
            sm.handle_event(Event::TrafficObserved {
                rx_bytes_delta: 20,
                tx_bytes_delta: 10,
            });
            assert_eq!(
                count_probes(&sm.handle_event(Event::TickElapsed { secs: 1 })),
                0
            );
        }
        assert_eq!(sm.state(), StabilizerState::Quiet);

        // 4. Wi-Fi -> cellular switch, RAT transition, and hotspot client -> still 0 healthy pings.
        sm.handle_event(Event::ContextChanged(RuntimeContext {
            is_wifi_default: true,
            ..RuntimeContext::default()
        }));
        assert_eq!(
            count_probes(&sm.handle_event(Event::ContextChanged(RuntimeContext {
                is_wifi_default: false,
                hotspot_clients: 1,
                ..RuntimeContext::default()
            }))),
            0
        );
        assert_eq!(
            count_probes(&sm.handle_event(Event::RadioTopologyChanged {
                reason: "RAT transition".to_owned(),
            })),
            0
        );
        assert_eq!(
            count_probes(&sm.handle_event(Event::RadioTopologyChanged {
                reason: "Cellular unblocked".to_owned(),
            })),
            0
        );

        // 5. Real drop #1: silent TX stall from Quiet -> immediate L2 StallCheck probe!
        sm.handle_event(Event::TrafficObserved {
            rx_bytes_delta: 0,
            tx_bytes_delta: 200,
        });
        let stall_actions = sm.handle_event(Event::TickElapsed { secs: 2 });
        assert_eq!(sm.state(), StabilizerState::Recovering);
        assert_eq!(count_probes(&stall_actions), 1);

        // Once healed, returns straight back to Quiet (0 follow-up EcoTail pings).
        sm.handle_event(Event::ProbeCompleted {
            tier: ProbeTier::L2HttpHead,
            reason: PingReason::StallCheck,
            succeeded: true,
        });
        assert_eq!(sm.state(), StabilizerState::Quiet);
        assert_eq!(sm.stalls_healed(), 1);

        // 6. Real drop #2: Lost NET_CAPABILITY_VALIDATED while Quiet on screen -> immediate recovery!
        let unval_actions = sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: false,
        });
        assert_eq!(sm.state(), StabilizerState::Recovering);
        assert_eq!(count_probes(&unval_actions), 1);

        sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: true,
        });
        assert_eq!(sm.state(), StabilizerState::Quiet);

        // 7. Real drop #3: OS blocked status -> immediate check!
        let blocked_actions = sm.handle_event(Event::RadioTopologyChanged {
            reason: "Cellular blocked by OS/modem".to_owned(),
        });
        assert_eq!(count_probes(&blocked_actions), 1);
        sm.handle_event(Event::ProbeCompleted {
            tier: ProbeTier::L1Dns,
            reason: PingReason::RadioEvent,
            succeeded: true,
        });
        assert_eq!(sm.state(), StabilizerState::Quiet);

        // 8. Explicit Boost 30m temporarily enables LiveKeepalive, then returns cleanly to Quiet · heal-only.
        let boost_on = sm.handle_event(Event::ContextChanged(RuntimeContext {
            boost_active: true,
            ..RuntimeContext::default()
        }));
        assert_eq!(sm.state(), StabilizerState::LiveKeepalive);
        assert_eq!(count_probes(&boost_on), 1);

        let boost_off = sm.handle_event(Event::ContextChanged(RuntimeContext {
            boost_active: false,
            ..RuntimeContext::default()
        }));
        assert_eq!(sm.state(), StabilizerState::Quiet);
        assert_eq!(count_probes(&boost_off), 0);
        assert!(sm.state_reason().contains("heal-only"));
    }
}
