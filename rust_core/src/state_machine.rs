use std::fmt;

/// Operating state of the idle-aware network stabilizer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum StabilizerState {
    /// Engine is stopped.
    Stopped,
    /// Real mobile data traffic is actively moving; sends 0 keepalive pings.
    Active,
    /// Mobile traffic has been idle for `idle_threshold_secs`; sends 1 lightweight HEAD ping per idle window.
    IdlePing,
    /// Screen just turned on or unlocked; sends 1 instant HEAD ping immediately.
    Warmup,
    /// Screen is off with no background transfer, or screen is on in an offline app past `offline_sleep_secs`; sends 0 pings.
    Sleeping,
    /// `NET_CAPABILITY_VALIDATED` was lost while active; fires wake-up ping and optionally escalates to root recovery after timeout.
    Recovering,
}

impl fmt::Display for StabilizerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped => write!(f, "Stopped"),
            Self::Active => write!(f, "Active"),
            Self::IdlePing => write!(f, "IdlePing"),
            Self::Warmup => write!(f, "Warmup"),
            Self::Sleeping => write!(f, "Sleeping"),
            Self::Recovering => write!(f, "Recovering"),
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
    pub idle_threshold_secs: u64,
    pub offline_sleep_secs: u64,
    pub ping_url: String,
    pub root_recovery_enabled: bool,
    pub recovery_timeout_secs: u64,
}

impl Default for StabilizerConfig {
    fn default() -> Self {
        Self {
            idle_threshold_secs: 4,
            offline_sleep_secs: 120,
            ping_url: "https://connectivitycheck.gstatic.com/generate_204".to_owned(),
            root_recovery_enabled: false,
            recovery_timeout_secs: 10,
        }
    }
}

impl StabilizerConfig {
    /// Validates user-supplied configuration bounds.
    pub fn validate(&self) -> Result<(), String> {
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

/// Reason a keepalive `HEAD` ping was triggered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PingReason {
    Warmup,
    IdleKeepalive,
    Recovery,
}

/// Side-effect action emitted by the deterministic state machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    SendPing(PingReason),
    TriggerRootRecovery,
    Log(LogLevel, String),
}

/// Input events processed by [`StateMachine`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Start,
    Stop,
    ScreenChanged {
        is_screen_on: bool,
        is_unlocked: bool,
    },
    NetworkCapabilitiesChanged {
        has_internet: bool,
        is_validated: bool,
    },
    TrafficObserved {
        external_bytes_delta: u64,
    },
    Tick1s,
}

/// Pure, deterministic idle-aware state machine for mobile data stabilization.
#[derive(Debug)]
pub struct StateMachine {
    config: StabilizerConfig,
    state: StabilizerState,
    is_running: bool,
    is_screen_on: bool,
    is_unlocked: bool,
    has_internet: bool,
    is_validated: bool,
    idle_secs: u64,
    secs_since_last_ping: u64,
    recovering_secs: u64,
    root_recovery_triggered: bool,
    last_traffic_bytes_per_sec: u64,
    recent_traffic_this_tick: bool,
}

impl StateMachine {
    /// Creates a new stopped state machine with the given validated configuration.
    pub fn new(config: StabilizerConfig) -> Self {
        Self {
            config,
            state: StabilizerState::Stopped,
            is_running: false,
            is_screen_on: true,
            is_unlocked: true,
            has_internet: false,
            is_validated: false,
            idle_secs: 0,
            secs_since_last_ping: 0,
            recovering_secs: 0,
            root_recovery_triggered: false,
            last_traffic_bytes_per_sec: 0,
            recent_traffic_this_tick: false,
        }
    }

    pub fn state(&self) -> StabilizerState {
        self.state
    }

    pub fn is_running(&self) -> bool {
        self.is_running
    }

    pub fn is_screen_on(&self) -> bool {
        self.is_screen_on
    }

    pub fn has_internet(&self) -> bool {
        self.has_internet
    }

    pub fn is_validated(&self) -> bool {
        self.is_validated
    }

    pub fn last_traffic_bytes_per_sec(&self) -> u64 {
        self.last_traffic_bytes_per_sec
    }

    pub fn config(&self) -> &StabilizerConfig {
        &self.config
    }

    pub fn update_config(&mut self, config: StabilizerConfig) {
        self.config = config;
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
                self.idle_secs = 0;
                self.secs_since_last_ping = 0;
                self.recovering_secs = 0;
                self.root_recovery_triggered = false;
                self.recent_traffic_this_tick = false;

                if !self.is_screen_on {
                    self.transition_to(
                        StabilizerState::Sleeping,
                        "Started while screen is off -> Sleeping (0 pings)",
                        &mut actions,
                    );
                } else if self.has_internet && !self.is_validated {
                    self.transition_to(
                        StabilizerState::Recovering,
                        "Started with unvalidated cellular link -> Recovering",
                        &mut actions,
                    );
                    actions.push(Action::SendPing(PingReason::Recovery));
                } else if self.has_internet {
                    self.transition_to(
                        StabilizerState::Warmup,
                        "Stabilizer started -> Warmup HEAD ping",
                        &mut actions,
                    );
                    actions.push(Action::SendPing(PingReason::Warmup));
                } else {
                    self.transition_to(
                        StabilizerState::Active,
                        "Stabilizer started -> waiting for cellular internet capability",
                        &mut actions,
                    );
                }
            }
            Event::Stop => {
                if !self.is_running {
                    return actions;
                }
                self.is_running = false;
                self.idle_secs = 0;
                self.secs_since_last_ping = 0;
                self.recovering_secs = 0;
                self.root_recovery_triggered = false;
                self.recent_traffic_this_tick = false;
                self.last_traffic_bytes_per_sec = 0;
                self.transition_to(StabilizerState::Stopped, "Stabilizer stopped", &mut actions);
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
                    self.recovering_secs = 0;
                    self.root_recovery_triggered = false;
                    if !self.recent_traffic_this_tick {
                        self.transition_to(
                            StabilizerState::Sleeping,
                            "Screen off with no active transfer -> Sleeping (0 pings)",
                            &mut actions,
                        );
                    }
                    return actions;
                }

                let turned_on = !was_screen_on && is_screen_on;
                let unlocked_while_idle_or_sleeping = is_screen_on
                    && !was_unlocked
                    && is_unlocked
                    && matches!(
                        self.state,
                        StabilizerState::Sleeping | StabilizerState::IdlePing
                    );

                if turned_on || unlocked_while_idle_or_sleeping {
                    self.idle_secs = 0;
                    self.secs_since_last_ping = 0;
                    self.recovering_secs = 0;
                    self.root_recovery_triggered = false;

                    if self.has_internet && !self.is_validated {
                        self.transition_to(
                            StabilizerState::Recovering,
                            "Screen wake with unvalidated cellular link -> Recovering",
                            &mut actions,
                        );
                        actions.push(Action::SendPing(PingReason::Recovery));
                    } else if self.has_internet {
                        self.transition_to(
                            StabilizerState::Warmup,
                            "Screen wake/unlock -> Warmup instant HEAD ping",
                            &mut actions,
                        );
                        actions.push(Action::SendPing(PingReason::Warmup));
                    } else {
                        self.transition_to(
                            StabilizerState::Warmup,
                            "Screen wake/unlock -> waiting for cellular internet",
                            &mut actions,
                        );
                    }
                }
            }
            Event::NetworkCapabilitiesChanged {
                has_internet,
                is_validated,
            } => {
                let had_internet = self.has_internet;
                self.has_internet = has_internet;
                self.is_validated = is_validated;

                if !self.is_running {
                    return actions;
                }

                if has_internet && !is_validated {
                    if self.state != StabilizerState::Sleeping
                        && self.state != StabilizerState::Recovering
                    {
                        self.recovering_secs = 0;
                        self.root_recovery_triggered = false;
                        self.secs_since_last_ping = 0;
                        self.transition_to(
                            StabilizerState::Recovering,
                            "Lost NET_CAPABILITY_VALIDATED -> Recovering wake-up ping",
                            &mut actions,
                        );
                        actions.push(Action::SendPing(PingReason::Recovery));
                    }
                } else if has_internet && is_validated {
                    if self.state == StabilizerState::Recovering {
                        self.recovering_secs = 0;
                        self.root_recovery_triggered = false;
                        self.idle_secs = 0;
                        let next_state = if !self.is_screen_on && !self.recent_traffic_this_tick {
                            StabilizerState::Sleeping
                        } else {
                            StabilizerState::Active
                        };
                        self.transition_to(
                            next_state,
                            "NET_CAPABILITY_VALIDATED restored",
                            &mut actions,
                        );
                    } else if !had_internet && self.state == StabilizerState::Warmup {
                        self.secs_since_last_ping = 0;
                        actions.push(Action::SendPing(PingReason::Warmup));
                    }
                } else if !has_internet && self.state == StabilizerState::Recovering {
                    self.recovering_secs = 0;
                    self.root_recovery_triggered = false;
                    let next_state = if self.is_screen_on {
                        StabilizerState::Active
                    } else {
                        StabilizerState::Sleeping
                    };
                    self.transition_to(
                        next_state,
                        "Cellular internet capability lost; pausing recovery",
                        &mut actions,
                    );
                }
            }
            Event::TrafficObserved {
                external_bytes_delta,
            } => {
                self.last_traffic_bytes_per_sec = external_bytes_delta;
                if !self.is_running {
                    return actions;
                }

                if external_bytes_delta > 0 {
                    self.idle_secs = 0;
                    self.recent_traffic_this_tick = true;
                    if self.state != StabilizerState::Recovering
                        && self.state != StabilizerState::Active
                    {
                        self.transition_to(
                            StabilizerState::Active,
                            "Real mobile traffic detected -> Active (0 pings)",
                            &mut actions,
                        );
                    }
                }
            }
            Event::Tick1s => {
                if !self.is_running {
                    return actions;
                }

                self.secs_since_last_ping = self.secs_since_last_ping.saturating_add(1);

                if self.state == StabilizerState::Recovering {
                    if self.recent_traffic_this_tick {
                        self.recent_traffic_this_tick = false;
                        self.idle_secs = 0;
                    } else {
                        self.idle_secs = self.idle_secs.saturating_add(1);
                        if !self.is_screen_on && self.idle_secs >= self.config.idle_threshold_secs {
                            self.recovering_secs = 0;
                            self.root_recovery_triggered = false;
                            self.transition_to(
                                StabilizerState::Sleeping,
                                "Screen off during recovery -> Sleeping (0 pings)",
                                &mut actions,
                            );
                            return actions;
                        }
                    }

                    self.recovering_secs = self.recovering_secs.saturating_add(1);
                    if self.has_internet
                        && !self.is_validated
                        && self.recovering_secs >= self.config.recovery_timeout_secs
                        && self.config.root_recovery_enabled
                        && !self.root_recovery_triggered
                    {
                        self.root_recovery_triggered = true;
                        actions.push(Action::Log(
                            LogLevel::Warn,
                            format!(
                                "Network unvalidated for {}s -> requesting root cellular recovery",
                                self.recovering_secs
                            ),
                        ));
                        actions.push(Action::TriggerRootRecovery);
                    }
                    return actions;
                }

                if self.recent_traffic_this_tick {
                    self.recent_traffic_this_tick = false;
                    self.idle_secs = 0;
                    if self.state != StabilizerState::Active {
                        self.transition_to(
                            StabilizerState::Active,
                            "Real mobile traffic active -> Active (0 pings)",
                            &mut actions,
                        );
                    }
                    return actions;
                }

                self.idle_secs = self.idle_secs.saturating_add(1);

                if !self.is_screen_on {
                    if self.state != StabilizerState::Sleeping
                        && self.idle_secs >= self.config.idle_threshold_secs
                    {
                        self.transition_to(
                            StabilizerState::Sleeping,
                            "Background transfer ended while screen off -> Sleeping (0 pings)",
                            &mut actions,
                        );
                    }
                    return actions;
                }

                if !self.has_internet {
                    return actions;
                }

                if !self.is_validated {
                    self.recovering_secs = 0;
                    self.root_recovery_triggered = false;
                    self.secs_since_last_ping = 0;
                    self.transition_to(
                        StabilizerState::Recovering,
                        "Unvalidated link while active -> Recovering",
                        &mut actions,
                    );
                    actions.push(Action::SendPing(PingReason::Recovery));
                    return actions;
                }

                if self.idle_secs >= self.config.offline_sleep_secs {
                    if self.state != StabilizerState::Sleeping {
                        self.transition_to(
                            StabilizerState::Sleeping,
                            "Offline app detected (no real traffic) -> Sleeping (0 pings)",
                            &mut actions,
                        );
                    }
                    return actions;
                }

                if self.state == StabilizerState::Sleeping {
                    return actions;
                }

                if self.idle_secs >= self.config.idle_threshold_secs
                    && self.secs_since_last_ping >= self.config.idle_threshold_secs
                {
                    self.secs_since_last_ping = 0;
                    if self.state != StabilizerState::IdlePing {
                        self.transition_to(
                            StabilizerState::IdlePing,
                            "Traffic idle threshold reached -> IdlePing HEAD probe",
                            &mut actions,
                        );
                    }
                    actions.push(Action::SendPing(PingReason::IdleKeepalive));
                }
            }
        }

        actions
    }

    fn transition_to(&mut self, next: StabilizerState, reason: &str, actions: &mut Vec<Action>) {
        if self.state != next {
            self.state = next;
            actions.push(Action::Log(LogLevel::Info, reason.to_owned()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_started_machine() -> StateMachine {
        let mut sm = StateMachine::new(StabilizerConfig::default());
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

    fn count_pings(actions: &[Action]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, Action::SendPing(_)))
            .count()
    }

    #[test]
    fn config_validation_rejects_invalid_values() {
        let mut cfg = StabilizerConfig::default();
        assert!(cfg.validate().is_ok());

        cfg.idle_threshold_secs = 0;
        assert!(cfg.validate().is_err());

        cfg = StabilizerConfig::default();
        cfg.offline_sleep_secs = 2;
        assert!(cfg.validate().is_err());

        cfg = StabilizerConfig::default();
        cfg.ping_url = "ftp://example.com".to_owned();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn active_traffic_sends_zero_pings() {
        let mut sm = default_started_machine();
        assert_eq!(sm.state(), StabilizerState::Warmup);

        for _ in 0..20 {
            let mut actions = sm.handle_event(Event::TrafficObserved {
                external_bytes_delta: 4096,
            });
            actions.extend(sm.handle_event(Event::Tick1s));
            assert_eq!(sm.state(), StabilizerState::Active);
            assert_eq!(count_pings(&actions), 0);
        }
    }

    #[test]
    fn idle_threshold_triggers_single_head_ping_after_4s() {
        let mut sm = default_started_machine();
        sm.handle_event(Event::TrafficObserved {
            external_bytes_delta: 1024,
        });
        sm.handle_event(Event::Tick1s);
        assert_eq!(sm.state(), StabilizerState::Active);

        for second in 1..=3 {
            let actions = sm.handle_event(Event::Tick1s);
            assert_eq!(
                sm.state(),
                StabilizerState::Active,
                "should remain Active at second {second}"
            );
            assert_eq!(count_pings(&actions), 0);
        }

        // 4th idle second triggers IdlePing + exactly 1 HEAD ping
        let actions = sm.handle_event(Event::Tick1s);
        assert_eq!(sm.state(), StabilizerState::IdlePing);
        assert_eq!(
            actions
                .iter()
                .filter(|a| **a == Action::SendPing(PingReason::IdleKeepalive))
                .count(),
            1
        );

        // Subsequent 3 seconds in IdlePing send 0 pings until next idle window
        for _ in 0..3 {
            let idle_actions = sm.handle_event(Event::Tick1s);
            assert_eq!(count_pings(&idle_actions), 0);
        }

        // Real traffic immediately restores Active with 0 pings
        let resume_actions = sm.handle_event(Event::TrafficObserved {
            external_bytes_delta: 512,
        });
        assert_eq!(sm.state(), StabilizerState::Active);
        assert_eq!(count_pings(&resume_actions), 0);
    }

    #[test]
    fn offline_app_auto_sleeps_after_120s_with_zero_pings_until_traffic_or_wake() {
        let mut sm = default_started_machine();
        sm.handle_event(Event::TrafficObserved {
            external_bytes_delta: 2048,
        });
        sm.handle_event(Event::Tick1s);
        assert_eq!(sm.state(), StabilizerState::Active);

        for _ in 1..120 {
            sm.handle_event(Event::Tick1s);
            assert_ne!(sm.state(), StabilizerState::Sleeping);
        }

        // At 120s of zero real non-ping traffic while screen is on -> Sleeping
        let sleep_actions = sm.handle_event(Event::Tick1s);
        assert_eq!(sm.state(), StabilizerState::Sleeping);
        assert_eq!(count_pings(&sleep_actions), 0);

        // Remains Sleeping with 0 pings for another 60s
        for _ in 0..60 {
            let actions = sm.handle_event(Event::Tick1s);
            assert_eq!(sm.state(), StabilizerState::Sleeping);
            assert_eq!(count_pings(&actions), 0);
        }

        // Real traffic wakes Sleeping -> Active (0 pings)
        let traffic_actions = sm.handle_event(Event::TrafficObserved {
            external_bytes_delta: 100,
        });
        assert_eq!(sm.state(), StabilizerState::Active);
        assert_eq!(count_pings(&traffic_actions), 0);
    }

    #[test]
    fn screen_off_sleeps_immediately_unless_background_transfer_active() {
        let mut sm = default_started_machine();

        // Screen off with no active transfer -> Sleeping immediately
        let off_actions = sm.handle_event(Event::ScreenChanged {
            is_screen_on: false,
            is_unlocked: false,
        });
        assert_eq!(sm.state(), StabilizerState::Sleeping);
        assert_eq!(count_pings(&off_actions), 0);

        // Background transfer starts while screen is off -> Active (0 pings)
        let bg_actions = sm.handle_event(Event::TrafficObserved {
            external_bytes_delta: 8192,
        });
        assert_eq!(sm.state(), StabilizerState::Active);
        assert_eq!(count_pings(&bg_actions), 0);
        sm.handle_event(Event::Tick1s);
        assert_eq!(sm.state(), StabilizerState::Active);

        // Once background transfer stops for idle_threshold_secs (4s) while screen off -> Sleeping
        for _ in 0..3 {
            sm.handle_event(Event::Tick1s);
            assert_eq!(sm.state(), StabilizerState::Active);
        }
        let end_bg_actions = sm.handle_event(Event::Tick1s);
        assert_eq!(sm.state(), StabilizerState::Sleeping);
        assert_eq!(count_pings(&end_bg_actions), 0);
    }

    #[test]
    fn screen_on_and_unlock_trigger_warmup_ping_immediately() {
        let mut sm = default_started_machine();
        sm.handle_event(Event::ScreenChanged {
            is_screen_on: false,
            is_unlocked: false,
        });
        assert_eq!(sm.state(), StabilizerState::Sleeping);

        // Screen turns on -> Warmup + 1 instant HEAD ping
        let wake_actions = sm.handle_event(Event::ScreenChanged {
            is_screen_on: true,
            is_unlocked: false,
        });
        assert_eq!(sm.state(), StabilizerState::Warmup);
        assert_eq!(
            wake_actions
                .iter()
                .filter(|a| **a == Action::SendPing(PingReason::Warmup))
                .count(),
            1
        );

        // If user was in IdlePing and unlocks screen -> Warmup + 1 instant HEAD ping
        for _ in 0..4 {
            sm.handle_event(Event::Tick1s);
        }
        assert_eq!(sm.state(), StabilizerState::IdlePing);

        let unlock_actions = sm.handle_event(Event::ScreenChanged {
            is_screen_on: true,
            is_unlocked: true,
        });
        assert_eq!(sm.state(), StabilizerState::Warmup);
        assert_eq!(
            unlock_actions
                .iter()
                .filter(|a| **a == Action::SendPing(PingReason::Warmup))
                .count(),
            1
        );
    }

    #[test]
    fn losing_validation_enters_recovering_and_triggers_root_after_timeout() {
        let cfg = StabilizerConfig {
            root_recovery_enabled: true,
            recovery_timeout_secs: 10,
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

        // Validation lost while active -> Recovering + immediate wake-up ping
        let loss_actions = sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: false,
        });
        assert_eq!(sm.state(), StabilizerState::Recovering);
        assert_eq!(
            loss_actions
                .iter()
                .filter(|a| **a == Action::SendPing(PingReason::Recovery))
                .count(),
            1
        );

        // 9 seconds unvalidated -> no root recovery yet
        for _ in 1..10 {
            let tick_actions = sm.handle_event(Event::Tick1s);
            assert!(!tick_actions.contains(&Action::TriggerRootRecovery));
        }

        // 10th second unvalidated -> triggers root recovery exactly once
        let timeout_actions = sm.handle_event(Event::Tick1s);
        assert!(timeout_actions.contains(&Action::TriggerRootRecovery));

        let extra_tick = sm.handle_event(Event::Tick1s);
        assert!(!extra_tick.contains(&Action::TriggerRootRecovery));

        // Validation returns -> exits Recovering back to Active
        sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: true,
        });
        assert_eq!(sm.state(), StabilizerState::Active);
    }

    #[test]
    fn root_recovery_not_triggered_when_disabled() {
        let cfg = StabilizerConfig {
            root_recovery_enabled: false,
            recovery_timeout_secs: 10,
            ..StabilizerConfig::default()
        };
        let mut sm = StateMachine::new(cfg);
        sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: true,
        });
        sm.handle_event(Event::Start);
        sm.handle_event(Event::NetworkCapabilitiesChanged {
            has_internet: true,
            is_validated: false,
        });
        assert_eq!(sm.state(), StabilizerState::Recovering);

        for _ in 0..15 {
            let actions = sm.handle_event(Event::Tick1s);
            assert!(!actions.contains(&Action::TriggerRootRecovery));
        }
    }
}
