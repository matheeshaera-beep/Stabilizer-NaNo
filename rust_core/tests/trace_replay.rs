use rust_core::pinger::ProbeTier;
use rust_core::state_machine::{
    Action, Event, PingReason, RuntimeContext, StabilizerConfig, StabilizerProfile,
    StabilizerState, StateMachine,
};

struct TraceReport {
    name: &'static str,
    probes: usize,
    active_modem_secs_est: u64,
    avoided_4s_pings: u64,
    stalls_healed: u64,
}

fn replay_trace<F>(
    name: &'static str,
    profile: StabilizerProfile,
    total_secs: u64,
    mut step: F,
) -> TraceReport
where
    F: FnMut(u64, &mut StateMachine) -> usize,
{
    let cfg = StabilizerConfig {
        profile,
        root_recovery_enabled: true,
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
    let start_actions = sm.handle_event(Event::Start);
    let mut probes = start_actions
        .iter()
        .filter(|a| matches!(a, Action::SendProbe { .. }))
        .count();
    let mut modem_tail_remaining = if probes > 0 { 8u64 } else { 0 };
    let mut active_modem_secs_est = 0u64;

    for t in 1..=total_secs {
        let step_probes = step(t, &mut sm);
        probes += step_probes;
        if step_probes > 0
            || sm.last_rx_bytes_per_sec() >= 1_024
            || sm.last_tx_bytes_per_sec() >= 256
        {
            modem_tail_remaining = 8;
        }
        if modem_tail_remaining > 0 {
            active_modem_secs_est += 1;
            modem_tail_remaining = modem_tail_remaining.saturating_sub(1);
        }
    }

    TraceReport {
        name,
        probes,
        active_modem_secs_est,
        avoided_4s_pings: sm.avoided_fixed_4s_pings(),
        stalls_healed: sm.stalls_healed(),
    }
}

#[test]
fn trace_replay_benchmarks_meet_v2_efficiency_targets() {
    // 1. 1-hour intermittent messaging (screen on/off bursts) in Auto (Eco default)
    let messaging = replay_trace(
        "Messaging 1h (Auto)",
        StabilizerProfile::Auto,
        3_600,
        |t, sm| {
            if t % 300 == 0 {
                sm.handle_event(Event::ScreenChanged {
                    is_screen_on: false,
                    is_unlocked: false,
                });
            } else if t % 300 == 120 {
                sm.handle_event(Event::ScreenChanged {
                    is_screen_on: true,
                    is_unlocked: true,
                });
            }
            let rx = if t % 120 < 4 { 3_200 } else { 40 };
            let tx = if t % 120 < 2 { 512 } else { 0 };
            sm.handle_event(Event::TrafficObserved {
                rx_bytes_delta: rx,
                tx_bytes_delta: tx,
            });
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            a.iter()
                .filter(|x| matches!(x, Action::SendProbe { .. }))
                .count()
        },
    );
    assert!(
        messaging.probes <= 80,
        "{} sent {} probes (too high)",
        messaging.name,
        messaging.probes
    );

    // 2. 1-hour continuous video streaming -> 0 keepalive probes after Warmup!
    let streaming = replay_trace(
        "Streaming 1h (Auto)",
        StabilizerProfile::Auto,
        3_600,
        |_, sm| {
            sm.handle_event(Event::TrafficObserved {
                rx_bytes_delta: 250_000,
                tx_bytes_delta: 4_000,
            });
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            a.iter()
                .filter(|x| matches!(x, Action::SendProbe { .. }))
                .count()
        },
    );
    assert_eq!(
        streaming.probes, 1,
        "Streaming should only emit initial Warmup"
    );
    assert!(streaming.avoided_4s_pings >= 890);

    // 3. 1-hour hotspot ON with 0 clients & screen off -> Quiet (0 probes after screen off)
    let idle_hotspot = replay_trace(
        "Idle Hotspot 0 Clients 1h",
        StabilizerProfile::Auto,
        3_600,
        |t, sm| {
            if t == 1 {
                sm.handle_event(Event::ContextChanged(RuntimeContext {
                    hotspot_clients: 0,
                    ..RuntimeContext::default()
                }));
                sm.handle_event(Event::ScreenChanged {
                    is_screen_on: false,
                    is_unlocked: false,
                });
            }
            sm.handle_event(Event::TrafficObserved {
                rx_bytes_delta: 50,
                tx_bytes_delta: 20,
            });
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            a.iter()
                .filter(|x| matches!(x, Action::SendProbe { .. }))
                .count()
        },
    );
    assert_eq!(idle_hotspot.probes, 1);

    // 4. 1-hour hotspot w/ 1 connected laptop (screen off) -> stays Live inside tail!
    let active_hotspot = replay_trace(
        "Active Hotspot 1 Client 1h",
        StabilizerProfile::Auto,
        3_600,
        |t, sm| {
            if t == 1 {
                sm.handle_event(Event::ContextChanged(RuntimeContext {
                    hotspot_clients: 1,
                    ..RuntimeContext::default()
                }));
                sm.handle_event(Event::ScreenChanged {
                    is_screen_on: false,
                    is_unlocked: false,
                });
            }
            let a = sm.handle_event(Event::TickElapsed { secs: 1 });
            a.iter()
                .filter(|x| matches!(x, Action::SendProbe { .. }))
                .count()
        },
    );
    assert!(
        (450..=900).contains(&active_hotspot.probes),
        "Active tethered client kept warm with {} probes/hr (modem ~{}s)",
        active_hotspot.probes,
        active_hotspot.active_modem_secs_est
    );

    // 5. Silent TX freeze healed in <= 4s
    let stall_heal = replay_trace(
        "Silent TX Stall Heal",
        StabilizerProfile::Auto,
        20,
        |t, sm| {
            if t <= 3 {
                sm.handle_event(Event::TrafficObserved {
                    rx_bytes_delta: 4_096,
                    tx_bytes_delta: 512,
                });
            } else {
                sm.handle_event(Event::TrafficObserved {
                    rx_bytes_delta: 0,
                    tx_bytes_delta: 700,
                });
            }
            let actions = sm.handle_event(Event::TickElapsed { secs: 1 });
            let mut count = 0;
            for act in actions {
                if let Action::SendProbe { tier, reason } = act {
                    count += 1;
                    if reason == PingReason::StallCheck {
                        sm.handle_event(Event::ProbeCompleted {
                            tier: ProbeTier::L2HttpHead,
                            reason,
                            succeeded: true,
                        });
                    } else {
                        let _ = tier;
                    }
                }
            }
            count
        },
    );
    assert!(stall_heal.stalls_healed >= 1);

    // 6. LTE <-> 5G handover triggers immediate RadioEvent probe from Quiet/Idle
    let cfg = StabilizerConfig {
        profile: StabilizerProfile::Eco,
        ..StabilizerConfig::default()
    };
    let mut sm = StateMachine::new(cfg);
    sm.handle_event(Event::NetworkCapabilitiesChanged {
        has_internet: true,
        is_validated: true,
    });
    sm.handle_event(Event::Start);
    for _ in 0..20 {
        sm.handle_event(Event::TickElapsed { secs: 1 });
    }
    assert_eq!(sm.state(), StabilizerState::Quiet);
    let handover_actions = sm.handle_event(Event::RadioTopologyChanged {
        reason: "RAT LTE -> NR_NSA".to_owned(),
    });
    assert!(handover_actions.iter().any(|a| matches!(
        a,
        Action::SendProbe {
            reason: PingReason::RadioEvent,
            ..
        }
    )));
}
