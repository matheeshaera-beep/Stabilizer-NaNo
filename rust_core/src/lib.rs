pub mod pinger;
pub mod state_machine;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pinger::HttpPinger;
use state_machine::{Action, Event, PingReason, StateMachine};
pub use state_machine::{LogLevel, StabilizerConfig, StabilizerState};
use thiserror::Error;
use tokio::runtime::Runtime;
use tokio::sync::watch;

uniffi::setup_scaffolding!();

const MAX_LOG_ENTRIES: usize = 100;

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
    pub is_running: bool,
    pub last_ping_latency_ms: Option<u64>,
    pub total_pings: u64,
    pub successful_pings: u64,
    pub failed_pings: u64,
    pub last_traffic_bytes_per_sec: u64,
    pub has_internet_capability: bool,
    pub is_validated: bool,
    pub is_screen_on: bool,
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

/// Foreign callback implemented in Kotlin to receive telemetry and root recovery requests.
#[uniffi::export(with_foreign)]
pub trait StabilizerObserver: Send + Sync {
    fn on_telemetry_updated(&self, snapshot: TelemetrySnapshot);
    fn on_request_root_recovery(&self);
}

/// Returns the default configuration for [`StabilizerEngine`].
#[uniffi::export]
pub fn default_stabilizer_config() -> StabilizerConfig {
    StabilizerConfig::default()
}

struct EngineInner {
    sm: StateMachine,
    last_ping_latency_ms: Option<u64>,
    total_pings: u64,
    successful_pings: u64,
    failed_pings: u64,
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

    fn snapshot(&self) -> TelemetrySnapshot {
        TelemetrySnapshot {
            state: self.sm.state(),
            is_running: self.sm.is_running(),
            last_ping_latency_ms: self.last_ping_latency_ms,
            total_pings: self.total_pings,
            successful_pings: self.successful_pings,
            failed_pings: self.failed_pings,
            last_traffic_bytes_per_sec: self.sm.last_traffic_bytes_per_sec(),
            has_internet_capability: self.sm.has_internet(),
            is_validated: self.sm.is_validated(),
            is_screen_on: self.sm.is_screen_on(),
            logs: self.logs.iter().cloned().collect(),
        }
    }
}

/// Thread-safe Rust core engine bridged to Kotlin via UniFFI.
#[derive(uniffi::Object)]
pub struct StabilizerEngine {
    inner: Arc<Mutex<EngineInner>>,
    observer: Arc<dyn StabilizerObserver>,
    pinger: HttpPinger,
    runtime: Runtime,
    shutdown_tx: watch::Sender<bool>,
    ping_in_flight: Arc<AtomicBool>,
}

#[uniffi::export]
impl StabilizerEngine {
    /// Creates a new [`StabilizerEngine`] with an embedded Tokio runtime and `rustls` HTTP pinger.
    #[uniffi::constructor]
    pub fn new(
        config: StabilizerConfig,
        observer: Arc<dyn StabilizerObserver>,
    ) -> Result<Arc<Self>, StabilizerError> {
        config
            .validate()
            .map_err(|reason| StabilizerError::InvalidConfig { reason })?;

        let pinger = HttpPinger::new().map_err(|reason| StabilizerError::RuntimeInit { reason })?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("stabilizer-nano-rt")
            .enable_all()
            .build()
            .map_err(|err| StabilizerError::RuntimeInit {
                reason: format!("failed to create Tokio runtime: {err}"),
            })?;

        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let engine = Arc::new(Self {
            inner: Arc::new(Mutex::new(EngineInner::new(config))),
            observer,
            pinger,
            runtime,
            shutdown_tx,
            ping_in_flight: Arc::new(AtomicBool::new(false)),
        });

        let weak_engine = Arc::downgrade(&engine);
        engine.runtime.spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            break;
                        }
                    }
                    _ = ticker.tick() => {
                        let Some(engine) = weak_engine.upgrade() else {
                            break;
                        };
                        let should_emit = {
                            match engine.inner.lock() {
                                Ok(guard) => guard.sm.is_running() && !guard.shut_down,
                                Err(_) => false,
                            }
                        };
                        if should_emit {
                            engine.dispatch_event(Event::Tick1s, true);
                        }
                    }
                }
            }
        });

        Ok(engine)
    }

    /// Starts the idle-aware stabilization loop.
    pub fn start(&self) {
        self.dispatch_event(Event::Start, true);
    }

    /// Stops the stabilization loop without shutting down the engine instance.
    pub fn stop(&self) {
        self.dispatch_event(Event::Stop, true);
    }

    /// Updates stabilization configuration parameters live.
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
            guard.push_log(LogLevel::Info, "Updated stabilizer configuration");
            guard.snapshot()
        };

        self.observer.on_telemetry_updated(snapshot);
        Ok(())
    }

    /// Updates screen power and keyguard unlock status from Android broadcasts.
    pub fn on_screen_state_changed(&self, is_screen_on: bool, is_unlocked: bool) {
        self.dispatch_event(
            Event::ScreenChanged {
                is_screen_on,
                is_unlocked,
            },
            true,
        );
    }

    /// Updates cellular internet and validation capabilities from `ConnectivityManager`.
    pub fn on_network_capabilities_changed(&self, has_internet: bool, is_validated: bool) {
        self.dispatch_event(
            Event::NetworkCapabilitiesChanged {
                has_internet,
                is_validated,
            },
            true,
        );
    }

    /// Feeds the 1-second external mobile traffic delta (after subtracting own UID bytes).
    pub fn on_traffic_sample(&self, external_mobile_bytes_delta: u64) {
        self.dispatch_event(
            Event::TrafficObserved {
                external_bytes_delta: external_mobile_bytes_delta,
            },
            false,
        );
    }

    /// Returns the latest telemetry snapshot synchronously.
    pub fn get_telemetry(&self) -> TelemetrySnapshot {
        match self.inner.lock() {
            Ok(guard) => guard.snapshot(),
            Err(poisoned) => poisoned.into_inner().snapshot(),
        }
    }

    /// Idempotently stops the engine and terminates background tasks.
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
            let actions = guard.sm.handle_event(event);
            let mut changed = guard.sm.state() != prev_state;
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
                Action::SendPing(reason) => self.spawn_ping(reason, ping_url.clone()),
                Action::TriggerRootRecovery => self.observer.on_request_root_recovery(),
                Action::Log(_, _) => {}
            }
        }
    }

    fn spawn_ping(&self, reason: PingReason, url: String) {
        if reason == PingReason::IdleKeepalive
            && self
                .ping_in_flight
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
        if reason != PingReason::IdleKeepalive {
            self.ping_in_flight.store(true, Ordering::Release);
        }

        let pinger = self.pinger.clone();
        let inner = Arc::clone(&self.inner);
        let observer = Arc::clone(&self.observer);
        let ping_in_flight = Arc::clone(&self.ping_in_flight);

        self.runtime.spawn(async move {
            let result = pinger.send_head_ping(&url).await;
            ping_in_flight.store(false, Ordering::Release);

            let snapshot = {
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
                    PingReason::IdleKeepalive => "Idle",
                    PingReason::Recovery => "Recovery",
                };
                match result {
                    Ok(outcome) => {
                        guard.successful_pings = guard.successful_pings.saturating_add(1);
                        guard.last_ping_latency_ms = Some(outcome.latency_ms);
                        guard.push_log(
                            LogLevel::Info,
                            format!(
                                "{label} HEAD {} -> {}ms",
                                outcome.status_code, outcome.latency_ms
                            ),
                        );
                    }
                    Err(err_msg) => {
                        guard.failed_pings = guard.failed_pings.saturating_add(1);
                        guard.push_log(LogLevel::Warn, format!("{label} ping error: {err_msg}"));
                    }
                }
                guard.snapshot()
            };

            observer.on_telemetry_updated(snapshot);
        });
    }
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
        root_requests: AtomicUsize,
        last_snapshot: Mutex<Option<TelemetrySnapshot>>,
    }

    impl TestObserver {
        fn new() -> Self {
            Self {
                updates: AtomicUsize::new(0),
                root_requests: AtomicUsize::new(0),
                last_snapshot: Mutex::new(None),
            }
        }
    }

    impl StabilizerObserver for TestObserver {
        fn on_telemetry_updated(&self, snapshot: TelemetrySnapshot) {
            self.updates.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut guard) = self.last_snapshot.lock() {
                *guard = Some(snapshot);
            }
        }

        fn on_request_root_recovery(&self) {
            self.root_requests.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn engine_validates_config_and_updates_telemetry() {
        let observer = Arc::new(TestObserver::new());
        let bad_cfg = StabilizerConfig {
            idle_threshold_secs: 0,
            ..StabilizerConfig::default()
        };
        assert!(StabilizerEngine::new(bad_cfg, observer.clone()).is_err());

        let engine = StabilizerEngine::new(StabilizerConfig::default(), observer.clone())
            .expect("valid default config");
        engine.on_network_capabilities_changed(true, true);
        engine.on_screen_state_changed(true, true);
        engine.start();

        let snap = engine.get_telemetry();
        assert!(snap.is_running);
        assert_eq!(snap.state, StabilizerState::Warmup);
        assert!(observer.updates.load(Ordering::Relaxed) >= 1);

        engine.on_traffic_sample(4096);
        let active_snap = engine.get_telemetry();
        assert_eq!(active_snap.state, StabilizerState::Active);
        assert_eq!(active_snap.last_traffic_bytes_per_sec, 4096);

        engine.shutdown();
        let stopped_snap = engine.get_telemetry();
        assert!(!stopped_snap.is_running);
        assert_eq!(stopped_snap.state, StabilizerState::Stopped);
    }
}
