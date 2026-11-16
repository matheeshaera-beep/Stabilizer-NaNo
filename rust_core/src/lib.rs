pub mod pinger;
pub mod state_machine;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pinger::{CellularPinger, ProbeTier, SocketBinder};
use state_machine::{Action, Event, PingReason, StateMachine};
pub use state_machine::{
    LogLevel, RuntimeContext, StabilizerConfig, StabilizerProfile, StabilizerState,
};
use thiserror::Error;
use tokio::runtime::Runtime;
use tokio::sync::watch;

uniffi::setup_scaffolding!();

const MAX_LOG_ENTRIES: usize = 120;

/// Single status log entry emitted by the Rust core.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LogEntry {
    pub timestamp_ms: u64,
    pub level: LogLevel,
    pub message: String,
}

/// Live telemetry snapshot exposed to Android UI and services.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TelemetrySnapshot {
    pub state: StabilizerState,
    pub state_reason: String,
    pub is_running: bool,
    pub recommended_sample_ms: u64,
    pub last_ping_latency_ms: Option<u64>,
    pub total_pings: u64,
    pub successful_pings: u64,
    pub failed_pings: u64,
    pub total_probe_bytes: u64,
    pub avoided_fixed_4s_pings: u64,
    pub stalls_detected: u64,
    pub stalls_healed: u64,
    pub recoveries_triggered: u64,
    pub last_rx_bytes_per_sec: u64,
    pub last_tx_bytes_per_sec: u64,
    pub has_internet_capability: bool,
    pub is_validated: bool,
    pub is_screen_on: bool,
    pub is_wifi_default: bool,
    pub is_tethering: bool,
    pub hotspot_clients: u32,
    pub boost_active: bool,
    pub is_calibrating: bool,
    pub logs: Vec<LogEntry>,
}

/// Errors returned across the UniFFI boundary.
#[derive(Debug, Error, uniffi::Error)]
pub enum StabilizerError {
    #[error("Invalid configuration: {reason}")]
    InvalidConfig { reason: String },
    #[error("Runtime initialization failed: {reason}")]
    RuntimeInit { reason: String },
}

/// Foreign callback implemented in Kotlin for telemetry updates, cellular socket binding,
/// Android re-validation, root recovery, and radio-tail calibration completion.
#[uniffi::export(with_foreign)]
pub trait StabilizerObserver: Send + Sync {
    fn on_telemetry_updated(&self, snapshot: TelemetrySnapshot);
    fn bind_socket_to_cellular(&self, fd: i32) -> bool;
    fn on_request_android_revalidation(&self);
    fn on_request_root_recovery(&self);
    fn on_calibration_finished(&self, recommended_tail_secs: u64, summary: String);
}

/// Returns the default V2 configuration for [`StabilizerEngine`].
#[uniffi::export]
pub fn default_stabilizer_config() -> StabilizerConfig {
    StabilizerConfig::default()
}

/// Returns a default runtime context snapshot.
#[uniffi::export]
pub fn default_runtime_context() -> RuntimeContext {
    RuntimeContext::default()
}

struct ObserverSocketBinder {
    observer: Arc<dyn StabilizerObserver>,
}

impl SocketBinder for ObserverSocketBinder {
    fn bind_cellular_fd(&self, fd: i32) -> bool {
        self.observer.bind_socket_to_cellular(fd)
    }
}

struct EngineInner {
    sm: StateMachine,
    last_ping_latency_ms: Option<u64>,
    total_pings: u64,
    successful_pings: u64,
    failed_pings: u64,
    total_probe_bytes: u64,
    is_calibrating: bool,
    logs: VecDeque<LogEntry>,
    shut_down: bool,
}

impl EngineInner {
    fn new(config: StabilizerConfig) -> Self {
        Self {
            sm: StateMachine::new(config),
            last_ping_latency_ms: None,
            total_pings: 0,
            successful_pings: 0,
            failed_pings: 0,
            total_probe_bytes: 0,
            is_calibrating: false,
            logs: VecDeque::with_capacity(MAX_LOG_ENTRIES),
            shut_down: false,
        }
    }

    fn push_log(&mut self, level: LogLevel, message: impl Into<String>) {
        if self.logs.len() >= MAX_LOG_ENTRIES {
            self.logs.pop_front();
        }
        self.logs.push_back(LogEntry {
            timestamp_ms: now_unix_ms(),
            level,
            message: message.into(),
        });
    }

    fn recommended_sample_ms(&self) -> u64 {
        if !self.sm.is_running() {
            return 5_000;
        }
        match self.sm.state() {
            StabilizerState::Quiet | StabilizerState::Outage => 10_000,
            StabilizerState::LiveKeepalive | StabilizerState::Recovering
                if !self.sm.is_screen_on() =>
            {
                5_000
            }
            _ => 1_000,
        }
    }

    fn snapshot(&self) -> TelemetrySnapshot {
        let ctx = self.sm.context();
        TelemetrySnapshot {
            state: self.sm.state(),
            state_reason: self.sm.state_reason().to_owned(),
            is_running: self.sm.is_running(),
            recommended_sample_ms: self.recommended_sample_ms(),
            last_ping_latency_ms: self.last_ping_latency_ms,
            total_pings: self.total_pings,
            successful_pings: self.successful_pings,
            failed_pings: self.failed_pings,
            total_probe_bytes: self.total_probe_bytes,
            avoided_fixed_4s_pings: self.sm.avoided_fixed_4s_pings(),
            stalls_detected: self.sm.stalls_detected(),
            stalls_healed: self.sm.stalls_healed(),
            recoveries_triggered: self.sm.recoveries_triggered(),
            last_rx_bytes_per_sec: self.sm.last_rx_bytes_per_sec(),
            last_tx_bytes_per_sec: self.sm.last_tx_bytes_per_sec(),
            has_internet_capability: self.sm.has_internet(),
            is_validated: self.sm.is_validated(),
            is_screen_on: self.sm.is_screen_on(),
            is_wifi_default: ctx.is_wifi_default,
            is_tethering: self.sm.is_tethering(),
            hotspot_clients: self.sm.hotspot_clients(),
            boost_active: ctx.boost_active,
            is_calibrating: self.is_calibrating,
            logs: self.logs.iter().cloned().collect(),
        }
    }
}

/// Thread-safe Rust core engine bridged to Kotlin via UniFFI.
#[derive(uniffi::Object)]
pub struct StabilizerEngine {
    inner: Arc<Mutex<EngineInner>>,
    observer: Arc<dyn StabilizerObserver>,
    pinger: CellularPinger,
    runtime: Runtime,
    shutdown_tx: watch::Sender<bool>,
    probe_in_flight: Arc<AtomicBool>,
    calibrating_flag: Arc<AtomicBool>,
}

#[uniffi::export]
impl StabilizerEngine {
    /// Creates a new [`StabilizerEngine`] with a single-threaded Tokio runtime (`current_thread`)
    /// and cellular-bound `rustls` probe engine.
    #[uniffi::constructor]
    pub fn new(
        config: StabilizerConfig,
        observer: Arc<dyn StabilizerObserver>,
    ) -> Result<Arc<Self>, StabilizerError> {
        config
            .validate()
            .map_err(|reason| StabilizerError::InvalidConfig { reason })?;

        let binder = Arc::new(ObserverSocketBinder {
            observer: Arc::clone(&observer),
        });
        let pinger = CellularPinger::new(binder)
            .map_err(|reason| StabilizerError::RuntimeInit { reason })?;

        // Single-threaded Tokio runtime (current_thread) run on one dedicated background thread
        // to eliminate multi-worker scheduler wakeups and minimize battery overhead.
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .thread_name("stabilizer-nano-rt")
            .enable_all()
            .build()
            .map_err(|err| StabilizerError::RuntimeInit {
                reason: format!("failed to create Tokio runtime: {err}"),
            })?;

        let engine = Arc::new(Self {
            inner: Arc::new(Mutex::new(EngineInner::new(config))),
            observer,
            pinger,
            runtime,
            shutdown_tx,
            probe_in_flight: Arc::new(AtomicBool::new(false)),
            calibrating_flag: Arc::new(AtomicBool::new(false)),
        });

        // Drive the `current_thread` Tokio reactor on a background thread until shutdown.
        let rt_handle = engine.runtime.handle().clone();
        std::thread::Builder::new()
            .name("stabilizer-nano-tokio".to_owned())
            .spawn(move || {
                rt_handle.block_on(async move {
                    while shutdown_rx.changed().await.is_ok() {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                });
            })
            .map_err(|err| StabilizerError::RuntimeInit {
                reason: format!("failed to spawn reactor thread: {err}"),
            })?;

        Ok(engine)
    }

    /// Starts the stabilization loop.
    pub fn start(&self) {
        self.dispatch_event(Event::Start, true);
    }

    /// Stops the stabilization loop without shutting down the engine instance.
    pub fn stop(&self) {
        self.dispatch_event(Event::Stop, true);
    }

    /// Updates stabilization configuration live.
    pub fn update_config(&self, config: StabilizerConfig) -> Result<(), StabilizerError> {
        config
            .validate()
            .map_err(|reason| StabilizerError::InvalidConfig { reason })?;

        let snapshot = {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| StabilizerError::RuntimeInit {
                    reason: "engine lock poisoned".to_owned(),
                })?;
            guard.sm.update_config(config);
            guard.push_log(LogLevel::Info, "Updated V2 configuration");
            guard.snapshot()
        };

        self.observer.on_telemetry_updated(snapshot);
        Ok(())
    }

    /// Updates screen power and keyguard unlock status.
    pub fn on_screen_state_changed(&self, is_screen_on: bool, is_unlocked: bool) {
        self.dispatch_event(
            Event::ScreenChanged {
                is_screen_on,
                is_unlocked,
            },
            true,
        );
    }

    /// Updates device/network context (Wi-Fi default, call mode, hotspot clients, battery, Doze, Boost, Live app).
    pub fn on_context_changed(&self, context: RuntimeContext) {
        self.dispatch_event(Event::ContextChanged(context), true);
    }

    /// Updates cellular internet and validation capabilities.
    pub fn on_network_capabilities_changed(&self, has_internet: bool, is_validated: bool) {
        self.dispatch_event(
            Event::NetworkCapabilitiesChanged {
                has_internet,
                is_validated,
            },
            true,
        );
    }

    /// Fires on carrier radio topology events (RAT LTE<->5G handover, signal recovery, SIM/link change).
    pub fn on_radio_topology_changed(&self, reason: String) {
        self.dispatch_event(Event::RadioTopologyChanged { reason }, true);
    }

    /// Feeds elapsed sampling seconds plus separate RX & TX deltas (after subtracting own UID bytes).
    pub fn on_sampling_tick(&self, elapsed_secs: u64, rx_bytes_delta: u64, tx_bytes_delta: u64) {
        let secs = elapsed_secs.max(1);
        let rx_bps = rx_bytes_delta / secs;
        let tx_bps = tx_bytes_delta / secs;
        self.dispatch_event(
            Event::TrafficObserved {
                rx_bytes_delta: rx_bps,
                tx_bytes_delta: tx_bps,
            },
            false,
        );
        self.dispatch_event(Event::TickElapsed { secs }, true);
    }

    /// Runs the automated P0 carrier RRC tail calibration (`3s, 6s, 10s, 15s, 30s, 60s` idle gaps)
    /// and notifies `on_calibration_finished` with the detected `radio_tail_secs`.
    pub fn calibrate_radio_tail(&self) {
        if self
            .calibrating_flag
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let snapshot = {
            let mut guard = match self.inner.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            guard.is_calibrating = true;
            guard.push_log(
                LogLevel::Info,
                "Calibrating carrier radio tail (gaps: 3s, 6s, 10s, 15s, 30s, 60s)...",
            );
            guard.snapshot()
        };
        self.observer.on_telemetry_updated(snapshot);

        let pinger = self.pinger.clone();
        let inner = Arc::clone(&self.inner);
        let observer = Arc::clone(&self.observer);
        let calibrating_flag = Arc::clone(&self.calibrating_flag);
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        self.runtime.spawn(async move {
            let gaps: [u64; 6] = [3, 6, 10, 15, 30, 60];
            let mut results: Vec<(u64, Option<u64>)> = Vec::with_capacity(gaps.len());

            // Initial warm probe so baseline is genuinely inside RRC connected state.
            let _ = pinger.probe_l1().await;

            for gap in gaps {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            calibrating_flag.store(false, Ordering::Release);
                            return;
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(gap)) => {}
                }

                let outcome = pinger.probe_l1().await;
                let rtt = outcome.as_ref().ok().map(|o| o.latency_ms);
                results.push((gap, rtt));

                let snap = {
                    let mut guard = match inner.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
                    if guard.shut_down {
                        calibrating_flag.store(false, Ordering::Release);
                        return;
                    }
                    match &outcome {
                        Ok(o) => {
                            guard.total_pings = guard.total_pings.saturating_add(1);
                            guard.successful_pings = guard.successful_pings.saturating_add(1);
                            guard.total_probe_bytes =
                                guard.total_probe_bytes.saturating_add(o.bytes_estimated);
                            guard.push_log(
                                LogLevel::Info,
                                format!(
                                    "Calibrate gap {gap}s -> {}ms ({})",
                                    o.latency_ms, o.target
                                ),
                            );
                        }
                        Err(err) => {
                            guard.total_pings = guard.total_pings.saturating_add(1);
                            guard.failed_pings = guard.failed_pings.saturating_add(1);
                            guard.push_log(
                                LogLevel::Warn,
                                format!("Calibrate gap {gap}s failed: {err}"),
                            );
                        }
                    }
                    guard.snapshot()
                };
                observer.on_telemetry_updated(snap);
            }

            let recommended = estimate_tail_from_samples(&results);
            let curve = results
                .iter()
                .map(|(g, r)| match r {
                    Some(ms) => format!("{g}s:{ms}ms"),
                    None => format!("{g}s:err"),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let summary = format!("Estimated radio_tail_secs = {recommended}s ({curve})");

            let final_snap = {
                let mut guard = match inner.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                guard.is_calibrating = false;
                let mut updated = guard.sm.config().clone();
                updated.radio_tail_secs = recommended;
                guard.sm.update_config(updated);
                guard.push_log(LogLevel::Info, summary.clone());
                guard.snapshot()
            };

            calibrating_flag.store(false, Ordering::Release);
            observer.on_telemetry_updated(final_snap);
            observer.on_calibration_finished(recommended, summary);
        });
    }

    /// Triggers one immediate fresh L2 `HEAD` probe manually.
    pub fn probe_now(&self) {
        let url = {
            let guard = match self.inner.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            guard.sm.config().ping_url.clone()
        };
        self.spawn_probe(ProbeTier::L2HttpHead, PingReason::Warmup, url);
    }

    /// Clears stored telemetry logs and notifies observers.
    pub fn clear_logs(&self) {
        let snapshot = {
            let mut guard = match self.inner.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            guard.logs.clear();
            guard.snapshot()
        };
        self.observer.on_telemetry_updated(snapshot);
    }

    /// Returns the latest telemetry snapshot synchronously.
    pub fn get_telemetry(&self) -> TelemetrySnapshot {
        match self.inner.lock() {
            Ok(guard) => guard.snapshot(),
            Err(poisoned) => poisoned.into_inner().snapshot(),
        }
    }

    /// Idempotently stops the engine and shuts down the Tokio reactor thread.
    pub fn shutdown(&self) {
        let snapshot = {
            let mut guard = match self.inner.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if guard.shut_down {
                return;
            }
            let actions = guard.sm.handle_event(Event::Stop);
            for action in actions {
                if let Action::Log(level, msg) = action {
                    guard.push_log(level, msg);
                }
            }
            guard.shut_down = true;
            guard.snapshot()
        };
        let _ = self.shutdown_tx.send(true);
        self.observer.on_telemetry_updated(snapshot);
    }
}

impl Drop for StabilizerEngine {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
    }
}

impl StabilizerEngine {
    fn dispatch_event(&self, event: Event, notify_always: bool) {
        let (actions, ping_url, snapshot, should_notify) = {
            let mut guard = match self.inner.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if guard.shut_down {
                return;
            }
            let prev_state = guard.sm.state();
            let prev_reason = guard.sm.state_reason().to_owned();
            let actions = guard.sm.handle_event(event);
            let mut changed =
                guard.sm.state() != prev_state || guard.sm.state_reason() != prev_reason;
            for action in &actions {
                if let Action::Log(level, msg) = action {
                    guard.push_log(*level, msg.clone());
                    changed = true;
                }
            }
            let ping_url = guard.sm.config().ping_url.clone();
            let snapshot = guard.snapshot();
            (actions, ping_url, snapshot, notify_always || changed)
        };

        if should_notify {
            self.observer.on_telemetry_updated(snapshot);
        }

        for action in actions {
            match action {
                Action::SendProbe { tier, reason } => {
                    self.spawn_probe(tier, reason, ping_url.clone())
                }
                Action::RequestAndroidRevalidation => {
                    self.observer.on_request_android_revalidation()
                }
                Action::TriggerRootRecovery => self.observer.on_request_root_recovery(),
                Action::Log(_, _) => {}
            }
        }
    }

    fn spawn_probe(&self, tier: ProbeTier, reason: PingReason, url: String) {
        let is_background = matches!(
            reason,
            PingReason::LiveKeepalive | PingReason::EcoTail | PingReason::OutageCheck
        );
        if is_background
            && self
                .probe_in_flight
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
        if !is_background {
            self.probe_in_flight.store(true, Ordering::Release);
        }

        let pinger = self.pinger.clone();
        let inner = Arc::clone(&self.inner);
        let observer = Arc::clone(&self.observer);
        let probe_in_flight = Arc::clone(&self.probe_in_flight);

        self.runtime.spawn(async move {
            let result = pinger.probe(tier, &url).await;
            probe_in_flight.store(false, Ordering::Release);

            let (succeeded, follow_actions, ping_url, snapshot) = {
                let mut guard = match inner.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                if guard.shut_down {
                    return;
                }
                guard.total_pings = guard.total_pings.saturating_add(1);
                let label = match reason {
                    PingReason::Warmup => "Warmup",
                    PingReason::EcoTail => "EcoTail",
                    PingReason::LiveKeepalive => "Live",
                    PingReason::LiveReverify => "LiveVerify",
                    PingReason::StallCheck => "StallCheck",
                    PingReason::RadioEvent => "RadioEvent",
                    PingReason::Recovery => "Recovery",
                    PingReason::OutageCheck => "OutageCheck",
                };
                let ok = match &result {
                    Ok(outcome) => {
                        guard.successful_pings = guard.successful_pings.saturating_add(1);
                        guard.last_ping_latency_ms = Some(outcome.latency_ms);
                        guard.total_probe_bytes = guard
                            .total_probe_bytes
                            .saturating_add(outcome.bytes_estimated);
                        let status_desc = if outcome.status_code > 0 {
                            format!("HTTP {}", outcome.status_code)
                        } else {
                            outcome.target.clone()
                        };
                        guard.push_log(
                            LogLevel::Info,
                            format!(
                                "{label} [{:?}] {status_desc} -> {}ms (~{}B)",
                                outcome.tier, outcome.latency_ms, outcome.bytes_estimated
                            ),
                        );
                        true
                    }
                    Err(err_msg) => {
                        guard.failed_pings = guard.failed_pings.saturating_add(1);
                        guard.push_log(LogLevel::Warn, format!("{label} probe error: {err_msg}"));
                        false
                    }
                };

                let follow = guard.sm.handle_event(Event::ProbeCompleted {
                    tier,
                    reason,
                    succeeded: ok,
                });
                for action in &follow {
                    if let Action::Log(lvl, msg) = action {
                        guard.push_log(*lvl, msg.clone());
                    }
                }
                let url = guard.sm.config().ping_url.clone();
                (ok, follow, url, guard.snapshot())
            };

            let _ = succeeded;
            observer.on_telemetry_updated(snapshot);

            for action in follow_actions {
                match action {
                    Action::RequestAndroidRevalidation => {
                        observer.on_request_android_revalidation();
                    }
                    Action::TriggerRootRecovery => {
                        observer.on_request_root_recovery();
                    }
                    Action::SendProbe { tier, reason } => {
                        if let Ok(outcome) = pinger.probe(tier, &ping_url).await {
                            let mut guard = match inner.lock() {
                                Ok(g) => g,
                                Err(p) => p.into_inner(),
                            };
                            if !guard.shut_down {
                                guard.total_pings = guard.total_pings.saturating_add(1);
                                guard.successful_pings = guard.successful_pings.saturating_add(1);
                                guard.last_ping_latency_ms = Some(outcome.latency_ms);
                                guard.total_probe_bytes = guard
                                    .total_probe_bytes
                                    .saturating_add(outcome.bytes_estimated);
                                guard.push_log(
                                    LogLevel::Info,
                                    format!(
                                        "Follow-up {:?} ({:?}) -> {}ms",
                                        reason, tier, outcome.latency_ms
                                    ),
                                );
                                let snap = guard.snapshot();
                                drop(guard);
                                observer.on_telemetry_updated(snap);
                            }
                        }
                    }
                    Action::Log(_, _) => {}
                }
            }
        });
    }
}

/// Estimates `radio_tail_secs` from `(gap_secs, Option<rtt_ms>)` calibration samples.
pub fn estimate_tail_from_samples(samples: &[(u64, Option<u64>)]) -> u64 {
    let baseline = samples
        .iter()
        .filter_map(|&(_, rtt)| rtt)
        .take(2)
        .min()
        .unwrap_or(80);
    let jump_threshold = (baseline.saturating_mul(3) / 2).max(baseline.saturating_add(45));

    let mut last_warm_gap = 6u64;
    for &(gap, rtt_opt) in samples {
        if let Some(rtt) = rtt_opt {
            if rtt >= jump_threshold && gap > 3 {
                return last_warm_gap.clamp(4, 20);
            }
            last_warm_gap = gap;
        }
    }
    8
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct TestObserver {
        updates: AtomicUsize,
        revalidations: AtomicUsize,
        root_requests: AtomicUsize,
    }

    impl TestObserver {
        fn new() -> Self {
            Self {
                updates: AtomicUsize::new(0),
                revalidations: AtomicUsize::new(0),
                root_requests: AtomicUsize::new(0),
            }
        }
    }

    impl StabilizerObserver for TestObserver {
        fn on_telemetry_updated(&self, _snapshot: TelemetrySnapshot) {
            self.updates.fetch_add(1, Ordering::Relaxed);
        }

        fn bind_socket_to_cellular(&self, _fd: i32) -> bool {
            true
        }

        fn on_request_android_revalidation(&self) {
            self.revalidations.fetch_add(1, Ordering::Relaxed);
        }

        fn on_request_root_recovery(&self) {
            self.root_requests.fetch_add(1, Ordering::Relaxed);
        }

        fn on_calibration_finished(&self, _recommended_tail_secs: u64, _summary: String) {}
    }

    #[test]
    fn engine_validates_config_and_tracks_rx_tx_and_sampling() {
        let observer = Arc::new(TestObserver::new());
        let bad_cfg = StabilizerConfig {
            radio_tail_secs: 2,
            ..StabilizerConfig::default()
        };
        assert!(StabilizerEngine::new(bad_cfg, observer.clone()).is_err());

        let engine = StabilizerEngine::new(StabilizerConfig::default(), observer.clone())
            .expect("valid default config");
        engine.on_network_capabilities_changed(true, true);
        engine.on_screen_state_changed(true, true);
        engine.start();

        engine.on_sampling_tick(1, 5_000, 400);
        let active_snap = engine.get_telemetry();
        assert_eq!(active_snap.state, StabilizerState::Active);
        assert_eq!(active_snap.last_rx_bytes_per_sec, 5_000);
        assert_eq!(active_snap.last_tx_bytes_per_sec, 400);
        assert_eq!(active_snap.recommended_sample_ms, 1_000);

        engine.shutdown();
        let stopped_snap = engine.get_telemetry();
        assert!(!stopped_snap.is_running);
        assert_eq!(stopped_snap.state, StabilizerState::Stopped);
    }

    #[test]
    fn calibration_estimator_detects_rtt_jump() {
        let samples = vec![
            (3, Some(60)),
            (6, Some(68)),
            (10, Some(72)),
            (15, Some(220)), // Jump indicates tail ended between 10s and 15s
            (30, Some(260)),
        ];
        assert_eq!(estimate_tail_from_samples(&samples), 10);
    }
}
