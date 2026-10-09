//! Integration test: every simulator preset, piped through the risk state
//! machine the way the sensor and risk-monitor cells will be wired.

use core::time::Duration;

use chillguard_core::risk::{
    DomainEvent, Input, MonitorState, RefrigeratorConfig, RefrigeratorId, Status, Timestamp,
    evaluate,
};
use chillguard_core::simulator::{Scenario, Simulator, SimulatorConfig};

const T0_MS: u64 = 1_791_555_000_000;
const FRIDGE: &str = "fridge-1";

/// Safe range 2..=8 °C, critical after 30 minutes, stale after 5 minutes.
///
/// The hysteresis is what makes the incident count independent of the seed.
/// The simulator's noise is ±0.2 °C, so while the cabinet drifts across the
/// 8 °C limit, readings land on both sides of it; without hysteresis each of
/// those crossings is its own incident (docs/decisions/0003-risk-policies.md,
/// policy 9). Half a degree is more than the 0.4 °C the noise can span.
fn risk_config() -> RefrigeratorConfig {
    RefrigeratorConfig {
        refrigerator_id: RefrigeratorId::new(FRIDGE),
        lower_limit_c: Some(2.0),
        upper_limit_c: 8.0,
        critical_after: Duration::from_mins(30),
        stale_after: Duration::from_mins(5),
        hysteresis_c: 0.5,
    }
}

/// What one run of a preset produced.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Outcome {
    final_status: Status,
    /// The most severe status seen on the way.
    peak_status: Status,
    incidents: usize,
    criticals: usize,
    recoveries: usize,
}

fn rank(status: Status) -> u8 {
    match status {
        Status::Normal => 0,
        Status::SensorStale => 1,
        Status::Warning => 2,
        Status::Critical => 3,
    }
}

/// Plays `scenario` to its end: a reading on every minute and a tick thirty
/// seconds later, each fed to `evaluate`.
fn play(scenario: Scenario, seed: u64) -> Result<Outcome, String> {
    let risk_config = risk_config();
    let minutes = scenario.total_duration().as_secs() / 60;
    let mut simulator = Simulator::new(
        SimulatorConfig::new(RefrigeratorId::new(FRIDGE)),
        scenario,
        seed,
    )
    .map_err(|error| error.to_string())?;

    let mut state = MonitorState::new();
    let mut outcome = Outcome {
        final_status: Status::Normal,
        peak_status: Status::Normal,
        incidents: 0,
        criticals: 0,
        recoveries: 0,
    };
    for minute in 0..=minutes {
        let on_the_minute = T0_MS + minute * 60_000;
        let reading = simulator.next_reading(Timestamp::from_millis(on_the_minute));
        let tick = Input::Tick {
            now: Timestamp::from_millis(on_the_minute + 30_000),
        };
        for input in [Input::Reading(reading), tick] {
            let (next, events) =
                evaluate(&state, input, &risk_config).map_err(|error| error.to_string())?;
            state = next;
            if rank(state.status()) > rank(outcome.peak_status) {
                outcome.peak_status = state.status();
            }
            for event in events {
                match event {
                    DomainEvent::WarningRaised { .. } => outcome.incidents += 1,
                    DomainEvent::CriticalRaised { .. } => outcome.criticals += 1,
                    DomainEvent::Recovered { .. } => outcome.recoveries += 1,
                    DomainEvent::StatusChanged { .. } => {}
                    DomainEvent::SensorStale { .. }
                    | DomainEvent::SensorResumed { .. }
                    | DomainEvent::ReadingRejected { .. } => {
                        return Err(format!("minute {minute}: unexpected {event:?}"));
                    }
                }
            }
        }
    }
    if state.open_incident().is_some() {
        return Err("an incident is still open at the end".to_string());
    }
    outcome.final_status = state.status();
    Ok(outcome)
}

#[test]
fn presets_end_in_the_expected_status_with_the_expected_incidents() {
    struct Case {
        name: &'static str,
        scenario: fn() -> Scenario,
        expected: Outcome,
    }
    let cases = [
        Case {
            name: "normal_day",
            scenario: Scenario::normal_day,
            expected: Outcome {
                final_status: Status::Normal,
                peak_status: Status::Normal,
                incidents: 0,
                criticals: 0,
                recoveries: 0,
            },
        },
        // Two hours without power: one incident that becomes critical, and
        // is recorded as recovered once the cabinet is back in range.
        Case {
            name: "power_failure_2h",
            scenario: Scenario::power_failure_2h,
            expected: Outcome {
                final_status: Status::Normal,
                peak_status: Status::Critical,
                incidents: 1,
                criticals: 1,
                recoveries: 1,
            },
        },
        // Five minutes of open door: one incident that recovers before the
        // 30 minutes that would make it critical.
        Case {
            name: "short_door_open",
            scenario: Scenario::short_door_open,
            expected: Outcome {
                final_status: Status::Normal,
                peak_status: Status::Warning,
                incidents: 1,
                criticals: 0,
                recoveries: 1,
            },
        },
    ];

    for case in cases {
        // The outcome must not depend on the noise, so it is checked for a
        // spread of seeds, not for one that happens to work.
        for seed in 0..64 {
            assert_eq!(
                play((case.scenario)(), seed),
                Ok(case.expected),
                "{} with seed {seed}",
                case.name
            );
        }
    }
}
