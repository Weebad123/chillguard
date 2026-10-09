//! Scenario tests: whole timelines fed through the public API of the risk
//! state machine, the way a cell would drive it.

// Temperatures are literals or sums of exactly representable halves that
// pass through the state machine unchanged, so exact comparison is correct.
#![allow(clippy::float_cmp)]
// The project standard exempts tests from the unwrap ban (clippy.toml), but
// clippy only recognises `#[test]` functions and `#[cfg(test)]` modules, not
// the helper functions of an integration test file.
#![allow(clippy::unwrap_used)]

use core::time::Duration;

use chillguard_core::risk::{
    DomainEvent, Incident, Input, MonitorState, ReadingSource, RefrigeratorConfig, RefrigeratorId,
    Severity, Status, TemperatureReading, Timestamp, evaluate,
};

const T0_MS: u64 = 1_791_555_000_000;
const FRIDGE: &str = "fridge-1";

/// Safe range 2..=8 °C, critical after 30 minutes, stale after 5 minutes.
fn config() -> RefrigeratorConfig {
    RefrigeratorConfig {
        refrigerator_id: RefrigeratorId::new(FRIDGE),
        lower_limit_c: Some(2.0),
        upper_limit_c: 8.0,
        critical_after: Duration::from_mins(30),
        stale_after: Duration::from_mins(5),
        hysteresis_c: 0.0,
    }
}

fn at(minute: u32) -> Timestamp {
    Timestamp::from_millis(T0_MS + u64::from(minute) * 60_000)
}

fn minutes(minutes: u64) -> Duration {
    Duration::from_mins(minutes)
}

/// The sensor reports on the minute.
fn reading(minute: u32, temperature_c: f64) -> Input {
    Input::Reading(TemperatureReading {
        refrigerator_id: RefrigeratorId::new(FRIDGE),
        temperature_c,
        observed_at: at(minute),
        source: ReadingSource::Simulated,
    })
}

/// The monitor's timer fires thirty seconds after the minute.
fn tick_at(minute: u32) -> Timestamp {
    Timestamp::from_millis(at(minute).as_millis() + 30_000)
}

fn tick(minute: u32) -> Input {
    Input::Tick {
        now: tick_at(minute),
    }
}

/// A timeline with a tick every minute and a reading on the minutes for
/// which `temperature` returns one.
fn timeline(minutes: u32, temperature: impl Fn(u32) -> Option<f64>) -> Vec<Input> {
    let mut inputs = Vec::new();
    for minute in 0..minutes {
        if let Some(temperature_c) = temperature(minute) {
            inputs.push(reading(minute, temperature_c));
        }
        inputs.push(tick(minute));
    }
    inputs
}

/// Runs a timeline on a fresh monitor; returns the state after every input
/// and the events of every input.
fn run(config: &RefrigeratorConfig, inputs: &[Input]) -> (Vec<MonitorState>, Vec<DomainEvent>) {
    let mut state = MonitorState::new();
    let mut states = Vec::new();
    let mut all = Vec::new();
    for input in inputs {
        let (next, events) = evaluate(&state, input.clone(), config).unwrap();
        state = next;
        states.push(state.clone());
        all.extend(events);
    }
    (states, all)
}

fn status_changed(from: Status, to: Status, at: Timestamp) -> DomainEvent {
    DomainEvent::StatusChanged { from, to, at }
}

/// Power fails at minute 10 and returns at minute 50. The cabinet warms by
/// half a degree a minute up to 20 °C, then cools by one degree a minute.
fn power_failure_temperature(minute: u32) -> f64 {
    match minute {
        10..=40 => 5.0 + 0.5 * f64::from(minute - 10),
        41..=50 => 20.0,
        51..=65 => 20.0 - f64::from(minute - 50),
        // Before the failure, and once the cabinet is back at its set point.
        _ => 5.0,
    }
}

/// The cabinet sits at 9 °C from minute 5. The sensor is silent from minute
/// 11 to minute 49, reports 11 °C at minute 50, and 7 °C from minute 51.
fn dropout_temperature(minute: u32) -> Option<f64> {
    match minute {
        0..5 => Some(5.0),
        5..=10 => Some(9.0),
        11..50 => None,
        50 => Some(11.0),
        _ => Some(7.0),
    }
}

#[test]
fn normal_day() {
    // 24 hours of readings between 3 and 6 °C, one a minute.
    let inputs = timeline(24 * 60, |minute| Some(3.0 + 0.5 * f64::from(minute % 7)));

    let (states, events) = run(&config(), &inputs);

    assert_eq!(events, [], "a normal day raises nothing");
    assert!(states.iter().all(|s| s.status() == Status::Normal));
    let last = states.last().unwrap();
    assert!(last.open_incident().is_none());
    assert_eq!(last.last_reading().unwrap().observed_at, at(24 * 60 - 1));
}

#[test]
fn power_failure_escalates_to_critical_then_recovers() {
    let inputs = timeline(90, |minute| Some(power_failure_temperature(minute)));

    let (states, events) = run(&config(), &inputs);

    // 8.0 °C at minute 16 is on the limit; 8.5 °C at minute 17 opens the
    // incident; 30 minutes later it is critical; 8.0 °C at minute 62 ends it.
    let opened = Incident {
        started_at: at(17),
        initial_temperature_c: 8.5,
        highest_temperature_c: 8.5,
        lowest_temperature_c: 8.5,
        severity: Severity::Warning,
        exposure: Duration::ZERO,
        has_data_gap: false,
        recovered_at: None,
        recovery_temperature_c: None,
        ..incident_of(&events)
    };
    let critical = Incident {
        highest_temperature_c: 20.0,
        severity: Severity::Critical,
        exposure: minutes(30),
        ..opened.clone()
    };
    let closed = Incident {
        exposure: minutes(44),
        recovered_at: Some(at(62)),
        recovery_temperature_c: Some(8.0),
        ..critical.clone()
    };
    assert_eq!(
        events,
        [
            status_changed(Status::Normal, Status::Warning, at(17)),
            DomainEvent::WarningRaised { incident: opened },
            status_changed(Status::Warning, Status::Critical, at(47)),
            DomainEvent::CriticalRaised { incident: critical },
            status_changed(Status::Critical, Status::Normal, at(62)),
            DomainEvent::Recovered {
                incident: closed,
                at: at(62),
                temperature: 8.0,
            },
        ]
    );

    let last = states.last().unwrap();
    assert_eq!(last.status(), Status::Normal);
    assert!(last.open_incident().is_none());
}

#[test]
fn brief_excursion_recovers_before_critical() {
    // The door stands open from minute 10 to minute 14.
    let inputs = timeline(30, |minute| {
        Some(match minute {
            10 => 9.0,
            11 => 11.0,
            12 => 12.0,
            13 => 10.0,
            14 => 8.5,
            _ => 5.0,
        })
    });

    let (states, events) = run(&config(), &inputs);

    let opened = Incident {
        started_at: at(10),
        initial_temperature_c: 9.0,
        highest_temperature_c: 9.0,
        lowest_temperature_c: 9.0,
        severity: Severity::Warning,
        exposure: Duration::ZERO,
        has_data_gap: false,
        recovered_at: None,
        recovery_temperature_c: None,
        ..incident_of(&events)
    };
    let closed = Incident {
        highest_temperature_c: 12.0,
        lowest_temperature_c: 8.5,
        exposure: minutes(4),
        recovered_at: Some(at(15)),
        recovery_temperature_c: Some(5.0),
        ..opened.clone()
    };
    assert_eq!(
        events,
        [
            status_changed(Status::Normal, Status::Warning, at(10)),
            DomainEvent::WarningRaised { incident: opened },
            status_changed(Status::Warning, Status::Normal, at(15)),
            DomainEvent::Recovered {
                incident: closed,
                at: at(15),
                temperature: 5.0,
            },
        ],
        "the excursion never becomes critical"
    );
    assert_eq!(states.last().unwrap().status(), Status::Normal);
}

#[test]
fn sensor_dropout_mid_excursion() {
    let inputs = timeline(60, dropout_temperature);

    let (states, events) = run(&config(), &inputs);

    let opened = Incident {
        started_at: at(5),
        initial_temperature_c: 9.0,
        highest_temperature_c: 9.0,
        lowest_temperature_c: 9.0,
        severity: Severity::Warning,
        exposure: Duration::ZERO,
        has_data_gap: false,
        recovered_at: None,
        recovery_temperature_c: None,
        ..incident_of(&events)
    };
    // The 40 silent minutes count as exposure, and the incident says so.
    let critical = Incident {
        highest_temperature_c: 11.0,
        severity: Severity::Critical,
        exposure: minutes(45),
        has_data_gap: true,
        ..opened.clone()
    };
    let closed = Incident {
        recovered_at: Some(at(51)),
        recovery_temperature_c: Some(7.0),
        ..critical.clone()
    };
    assert_eq!(
        events,
        [
            status_changed(Status::Normal, Status::Warning, at(5)),
            DomainEvent::WarningRaised {
                incident: opened.clone()
            },
            // Last reading at minute 10; the tick at 15:30 is the first one
            // more than five minutes later.
            status_changed(Status::Warning, Status::SensorStale, tick_at(15)),
            DomainEvent::SensorStale { since: at(10) },
            status_changed(Status::SensorStale, Status::Critical, at(50)),
            DomainEvent::SensorResumed { at: at(50) },
            DomainEvent::CriticalRaised { incident: critical },
            status_changed(Status::Critical, Status::Normal, at(51)),
            DomainEvent::Recovered {
                incident: closed,
                at: at(51),
                temperature: 7.0,
            },
        ]
    );

    // While the sensor is silent the incident stays open and frozen as the
    // last reading left it.
    let frozen = Incident {
        exposure: minutes(5),
        ..opened
    };
    let silent: Vec<&MonitorState> = states
        .iter()
        .filter(|s| s.status() == Status::SensorStale)
        .collect();
    assert_eq!(silent.len(), 35, "ticks of minutes 15 through 49");
    assert!(silent.iter().all(|s| s.open_incident() == Some(&frozen)));
}

#[test]
fn restart_resumes_from_serialized_state() {
    let config = config();
    for (name, inputs) in [
        (
            "power failure",
            timeline(90, |minute| Some(power_failure_temperature(minute))),
        ),
        ("sensor dropout", timeline(60, dropout_temperature)),
    ] {
        let (expected_states, expected_events) = run(&config, &inputs);

        // Restart after every single input: persist the state, drop it, load
        // it back, carry on. Postcard is what a cell's `State` stores by
        // default; JSON is the format on the wire.
        let mut via_postcard = MonitorState::new();
        let mut via_json = MonitorState::new();
        let mut events_postcard = Vec::new();
        let mut events_json = Vec::new();
        for (index, input) in inputs.iter().enumerate() {
            let (next, events) = evaluate(&via_postcard, input.clone(), &config).unwrap();
            let bytes = postcard::to_allocvec(&next).unwrap();
            via_postcard = postcard::from_bytes(&bytes).unwrap();
            events_postcard.extend(events);

            let (next, events) = evaluate(&via_json, input.clone(), &config).unwrap();
            let text = serde_json::to_string(&next).unwrap();
            via_json = serde_json::from_str(&text).unwrap();
            events_json.extend(events);

            assert_eq!(
                via_postcard, expected_states[index],
                "{name}: input {index}"
            );
            assert_eq!(via_json, expected_states[index], "{name}: input {index}");
        }
        assert_eq!(events_postcard, expected_events, "{name}");
        assert_eq!(events_json, expected_events, "{name}");
    }
}

#[test]
fn restart_mid_incident_keeps_the_incident_and_its_alert_history() {
    let config = config();
    let inputs = timeline(90, |minute| Some(power_failure_temperature(minute)));
    // Stop after the reading of minute 50: critical, power not yet back.
    let stop = inputs
        .iter()
        .position(|input| *input == reading(50, 20.0))
        .unwrap()
        + 1;

    let (before, events_before) = run(&config, &inputs[..stop]);
    let saved = postcard::to_allocvec(before.last().unwrap()).unwrap();
    drop(before);

    let mut state: MonitorState = postcard::from_bytes(&saved).unwrap();
    assert_eq!(state.status(), Status::Critical);
    let mut events_after = Vec::new();
    for input in &inputs[stop..] {
        let (next, events) = evaluate(&state, input.clone(), &config).unwrap();
        state = next;
        events_after.extend(events);
    }

    // The alerts raised before the restart are not raised again; the only
    // thing left to report is the recovery of the same incident.
    let raised = incident_of(&events_before);
    assert!(matches!(
        events_after.as_slice(),
        [
            DomainEvent::StatusChanged {
                from: Status::Critical,
                to: Status::Normal,
                ..
            },
            DomainEvent::Recovered { incident, .. },
        ] if incident.incident_id == raised.incident_id && incident.severity == Severity::Critical
    ));
}

/// The incident announced by the first `WarningRaised`. Expected incidents
/// are built from it with struct update syntax, so every other field is
/// spelled out by the test and only the identifiers are taken from here.
fn incident_of(events: &[DomainEvent]) -> Incident {
    let incident = events
        .iter()
        .find_map(|event| match event {
            DomainEvent::WarningRaised { incident } => Some(incident.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(incident.refrigerator_id, RefrigeratorId::new(FRIDGE));
    assert_eq!(
        incident.incident_id.as_str(),
        format!("{FRIDGE}-{}", incident.started_at.as_millis())
    );
    incident
}
