//! Unit tests for the risk state machine: a transition table, one named test
//! per rule of the specification, and one per edge-case policy recorded in
//! `docs/decisions/0003-risk-policies.md`.

// Temperatures here are literals that pass through the state machine
// unchanged, so exact comparison is the correct check.
#![allow(clippy::float_cmp)]

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use super::*;

const T0_MS: u64 = 1_791_555_000_000;
const MINUTE_MS: u64 = 60_000;
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

fn at_ms(offset_ms: u64) -> Timestamp {
    Timestamp::from_millis(T0_MS + offset_ms)
}

fn at(minute: u64) -> Timestamp {
    at_ms(minute * MINUTE_MS)
}

fn reading_ms(offset_ms: u64, temperature_c: f64) -> Input {
    Input::Reading(TemperatureReading {
        refrigerator_id: RefrigeratorId::new(FRIDGE),
        temperature_c,
        observed_at: at_ms(offset_ms),
        source: ReadingSource::Simulated,
    })
}

fn reading(minute: u64, temperature_c: f64) -> Input {
    reading_ms(minute * MINUTE_MS, temperature_c)
}

fn tick(minute: u64) -> Input {
    Input::Tick { now: at(minute) }
}

/// Feeds `inputs` to a fresh monitor and returns the final state with the
/// events of every input, in order.
fn run(config: &RefrigeratorConfig, inputs: Vec<Input>) -> (MonitorState, Vec<DomainEvent>) {
    let mut state = MonitorState::new();
    let mut all = Vec::new();
    for input in inputs {
        let (next, events) = evaluate(&state, input, config).expect("config is valid");
        state = next;
        all.extend(events);
    }
    (state, all)
}

/// Applies one more input to `state`.
fn step(
    state: &MonitorState,
    input: Input,
    config: &RefrigeratorConfig,
) -> (MonitorState, Vec<DomainEvent>) {
    evaluate(state, input, config).expect("config is valid")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    StatusChanged,
    WarningRaised,
    CriticalRaised,
    Recovered,
    SensorStale,
    SensorResumed,
    ReadingRejected,
}

fn kinds(events: &[DomainEvent]) -> Vec<Kind> {
    events
        .iter()
        .map(|event| match event {
            DomainEvent::StatusChanged { .. } => Kind::StatusChanged,
            DomainEvent::WarningRaised { .. } => Kind::WarningRaised,
            DomainEvent::CriticalRaised { .. } => Kind::CriticalRaised,
            DomainEvent::Recovered { .. } => Kind::Recovered,
            DomainEvent::SensorStale { .. } => Kind::SensorStale,
            DomainEvent::SensorResumed { .. } => Kind::SensorResumed,
            DomainEvent::ReadingRejected { .. } => Kind::ReadingRejected,
        })
        .collect()
}

fn count(events: &[DomainEvent], kind: Kind) -> usize {
    kinds(events).into_iter().filter(|k| *k == kind).count()
}

fn open_incident(state: &MonitorState) -> &Incident {
    state.open_incident().expect("an incident is open")
}

fn rejection(events: &[DomainEvent]) -> &RejectReason {
    match events {
        [DomainEvent::ReadingRejected { reason }] => reason,
        other => panic!("expected exactly one ReadingRejected, got {other:?}"),
    }
}

fn recovered(events: &[DomainEvent]) -> &Incident {
    events
        .iter()
        .find_map(|event| match event {
            DomainEvent::Recovered { incident, .. } => Some(incident),
            _ => None,
        })
        .expect("a Recovered event")
}

// ---------------------------------------------------------------------------
// Transition table
// ---------------------------------------------------------------------------

struct Transition {
    name: &'static str,
    /// Inputs that bring a fresh monitor into the starting state.
    setup: Vec<Input>,
    input: Input,
    from: Status,
    to: Status,
    events: Vec<Kind>,
}

#[allow(clippy::too_many_lines)] // one row per transition; splitting the table would hide gaps
fn transitions() -> Vec<Transition> {
    use Kind::{
        CriticalRaised, Recovered, SensorResumed, SensorStale, StatusChanged, WarningRaised,
    };
    vec![
        Transition {
            name: "normal stays normal on an in-range reading",
            setup: vec![reading(0, 5.0)],
            input: reading(1, 6.0),
            from: Status::Normal,
            to: Status::Normal,
            events: vec![],
        },
        Transition {
            name: "normal to warning on a reading above the upper limit",
            setup: vec![reading(0, 5.0)],
            input: reading(1, 8.5),
            from: Status::Normal,
            to: Status::Warning,
            events: vec![StatusChanged, WarningRaised],
        },
        Transition {
            name: "normal to warning on a reading below the lower limit",
            setup: vec![reading(0, 5.0)],
            input: reading(1, 1.5),
            from: Status::Normal,
            to: Status::Warning,
            events: vec![StatusChanged, WarningRaised],
        },
        Transition {
            name: "fresh monitor to warning on a first reading out of range",
            setup: vec![],
            input: reading(0, 9.0),
            from: Status::Normal,
            to: Status::Warning,
            events: vec![StatusChanged, WarningRaised],
        },
        Transition {
            name: "warning stays warning before the critical duration",
            setup: vec![reading(0, 9.0)],
            input: reading(29, 9.0),
            from: Status::Warning,
            to: Status::Warning,
            events: vec![],
        },
        Transition {
            name: "warning to critical at the critical duration",
            setup: vec![reading(0, 9.0)],
            input: reading(30, 9.0),
            from: Status::Warning,
            to: Status::Critical,
            events: vec![StatusChanged, CriticalRaised],
        },
        Transition {
            name: "warning to normal on an in-range reading",
            setup: vec![reading(0, 9.0)],
            input: reading(1, 7.0),
            from: Status::Warning,
            to: Status::Normal,
            events: vec![StatusChanged, Recovered],
        },
        Transition {
            name: "critical stays critical on a further out-of-range reading",
            setup: vec![reading(0, 9.0), reading(30, 9.0)],
            input: reading(31, 9.5),
            from: Status::Critical,
            to: Status::Critical,
            events: vec![],
        },
        Transition {
            name: "critical to normal on an in-range reading",
            setup: vec![reading(0, 9.0), reading(30, 9.0)],
            input: reading(31, 7.0),
            from: Status::Critical,
            to: Status::Normal,
            events: vec![StatusChanged, Recovered],
        },
        Transition {
            name: "normal stays normal on a tick within stale_after",
            setup: vec![reading(0, 5.0)],
            input: tick(5),
            from: Status::Normal,
            to: Status::Normal,
            events: vec![],
        },
        Transition {
            name: "normal to stale on a late tick",
            setup: vec![reading(0, 5.0)],
            input: tick(6),
            from: Status::Normal,
            to: Status::SensorStale,
            events: vec![StatusChanged, SensorStale],
        },
        Transition {
            name: "warning to stale on a late tick",
            setup: vec![reading(0, 9.0)],
            input: tick(6),
            from: Status::Warning,
            to: Status::SensorStale,
            events: vec![StatusChanged, SensorStale],
        },
        Transition {
            name: "critical to stale on a late tick",
            setup: vec![reading(0, 9.0), reading(30, 9.0)],
            input: tick(36),
            from: Status::Critical,
            to: Status::SensorStale,
            events: vec![StatusChanged, SensorStale],
        },
        Transition {
            name: "stale stays stale on a further tick",
            setup: vec![reading(0, 5.0), tick(6)],
            input: tick(7),
            from: Status::SensorStale,
            to: Status::SensorStale,
            events: vec![],
        },
        Transition {
            name: "stale to normal on an in-range reading, no incident",
            setup: vec![reading(0, 5.0), tick(6)],
            input: reading(7, 5.0),
            from: Status::SensorStale,
            to: Status::Normal,
            events: vec![StatusChanged, SensorResumed],
        },
        Transition {
            name: "stale to warning on an out-of-range reading, no incident",
            setup: vec![reading(0, 5.0), tick(6)],
            input: reading(7, 9.0),
            from: Status::SensorStale,
            to: Status::Warning,
            events: vec![StatusChanged, SensorResumed, WarningRaised],
        },
        Transition {
            name: "stale to warning, open incident still below the critical duration",
            setup: vec![reading(0, 9.0), tick(6)],
            input: reading(7, 9.0),
            from: Status::SensorStale,
            to: Status::Warning,
            events: vec![StatusChanged, SensorResumed],
        },
        Transition {
            name: "stale to critical, open incident passed the critical duration while silent",
            setup: vec![reading(0, 9.0), tick(6)],
            input: reading(40, 9.0),
            from: Status::SensorStale,
            to: Status::Critical,
            events: vec![StatusChanged, SensorResumed, CriticalRaised],
        },
        Transition {
            name: "stale to critical, open incident was already critical",
            setup: vec![reading(0, 9.0), reading(30, 9.0), tick(36)],
            input: reading(37, 9.0),
            from: Status::SensorStale,
            to: Status::Critical,
            events: vec![StatusChanged, SensorResumed],
        },
        Transition {
            name: "stale to normal on an in-range reading, closing the open incident",
            setup: vec![reading(0, 9.0), tick(6)],
            input: reading(7, 5.0),
            from: Status::SensorStale,
            to: Status::Normal,
            events: vec![StatusChanged, SensorResumed, Recovered],
        },
    ]
}

#[test]
fn transition_table() {
    let config = config();
    for case in transitions() {
        let (before, _) = run(&config, case.setup);
        assert_eq!(before.status(), case.from, "{}: starting status", case.name);

        let (after, events) = step(&before, case.input, &config);
        assert_eq!(after.status(), case.to, "{}: resulting status", case.name);
        assert_eq!(kinds(&events), case.events, "{}: events", case.name);

        // A status change is announced exactly when the status changed.
        if case.from == case.to {
            assert_eq!(count(&events, Kind::StatusChanged), 0, "{}", case.name);
        } else {
            let DomainEvent::StatusChanged { from, to, .. } = &events[0] else {
                panic!("{}: StatusChanged must come first", case.name);
            };
            assert_eq!((*from, *to), (case.from, case.to), "{}", case.name);
        }
    }
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

#[test]
fn rule_1_normal_to_warning_opens_incident_and_raises_warning_once() {
    let (state, events) = run(
        &config(),
        vec![
            reading(0, 5.0),
            reading(1, 9.0),
            reading(2, 9.5),
            reading(3, 9.2),
        ],
    );

    assert_eq!(state.status(), Status::Warning);
    assert_eq!(kinds(&events), [Kind::StatusChanged, Kind::WarningRaised]);
    assert_eq!(
        events[0],
        DomainEvent::StatusChanged {
            from: Status::Normal,
            to: Status::Warning,
            at: at(1),
        }
    );

    let DomainEvent::WarningRaised { incident } = &events[1] else {
        panic!("expected WarningRaised, got {:?}", events[1]);
    };
    assert_eq!(incident.refrigerator_id, RefrigeratorId::new(FRIDGE));
    assert_eq!(incident.started_at, at(1));
    assert_eq!(incident.initial_temperature_c, 9.0);
    assert_eq!(incident.severity, Severity::Warning);
    assert!(incident.is_open());
    assert_eq!(
        open_incident(&state).incident_id,
        incident.incident_id,
        "the open incident is the one that was announced"
    );
}

#[test]
fn rule_2_warning_to_critical_uses_reading_timestamps_not_counts() {
    let config = config();

    // 200 out-of-range readings inside 20 minutes: many readings, not critical.
    let burst = (0..200).map(|i| reading_ms(i * 6_000, 9.0)).collect();
    let (state, events) = run(&config, burst);
    assert_eq!(state.status(), Status::Warning);
    assert_eq!(count(&events, Kind::CriticalRaised), 0);

    // Two readings 30 minutes apart: few readings, critical.
    let (state, events) = run(&config, vec![reading(0, 9.0), reading(30, 9.0)]);
    assert_eq!(state.status(), Status::Critical);
    assert_eq!(count(&events, Kind::CriticalRaised), 1);

    // The threshold is inclusive: one millisecond short is still a warning.
    let just_short = 30 * MINUTE_MS - 1;
    let (state, _) = run(&config, vec![reading(0, 9.0), reading_ms(just_short, 9.0)]);
    assert_eq!(state.status(), Status::Warning);
    let (state, _) = step(&state, reading(30, 9.0), &config);
    assert_eq!(state.status(), Status::Critical);
}

#[test]
fn rule_2_critical_is_raised_once_per_incident() {
    let (state, events) = run(
        &config(),
        vec![
            reading(0, 9.0),
            reading(30, 9.0),
            reading(31, 9.0),
            reading(60, 9.0),
            reading(90, 12.0),
        ],
    );

    assert_eq!(state.status(), Status::Critical);
    assert_eq!(count(&events, Kind::WarningRaised), 1);
    assert_eq!(count(&events, Kind::CriticalRaised), 1);
    let critical = events
        .iter()
        .find_map(|event| match event {
            DomainEvent::CriticalRaised { incident } => Some(incident),
            _ => None,
        })
        .expect("a CriticalRaised event");
    assert_eq!(critical.severity, Severity::Critical);
    assert_eq!(critical.exposure, Duration::from_mins(30));
    assert_eq!(critical.incident_id, open_incident(&state).incident_id);
}

#[test]
fn rule_3_duplicate_suppression_updates_only_peak_and_exposure() {
    let config = config();
    let (opened, _) = run(&config, vec![reading(0, 9.0)]);
    let first = open_incident(&opened).clone();

    // While a warning.
    let (state, events) = step(&opened, reading(1, 10.5), &config);
    assert!(events.is_empty(), "no new alert: {events:?}");
    let (state, events) = step(&state, reading(2, 9.5), &config);
    assert!(events.is_empty(), "no new alert: {events:?}");

    let incident = open_incident(&state);
    assert_eq!(incident.highest_temperature_c, 10.5);
    assert_eq!(incident.exposure, Duration::from_mins(2));
    assert_eq!(
        *incident,
        Incident {
            highest_temperature_c: 10.5,
            exposure: Duration::from_mins(2),
            ..first.clone()
        },
        "nothing but the peak and the exposure changed"
    );

    // While critical. Readings keep coming every five minutes, so the
    // incident reaches the critical duration without a data gap.
    let mut state = state;
    for minute in [5, 10, 15, 20, 25, 30] {
        (state, _) = step(&state, reading(minute, 9.0), &config);
    }
    assert_eq!(state.status(), Status::Critical);
    let (state, events) = step(&state, reading(31, 11.0), &config);
    assert!(events.is_empty(), "no new alert: {events:?}");
    assert_eq!(
        *open_incident(&state),
        Incident {
            highest_temperature_c: 11.0,
            exposure: Duration::from_mins(31),
            severity: Severity::Critical,
            ..first
        }
    );
}

#[test]
fn rule_3_cold_excursion_tracks_the_lowest_temperature() {
    let (state, _) = run(
        &config(),
        vec![reading(0, 1.5), reading(1, -3.0), reading(2, 0.5)],
    );
    let incident = open_incident(&state);
    assert_eq!(incident.lowest_temperature_c, -3.0);
    assert_eq!(incident.highest_temperature_c, 1.5);
}

#[test]
fn rule_4_recovery_closes_incident_and_keeps_peak_severity() {
    struct Case {
        name: &'static str,
        excursion: Vec<Input>,
        recovery_minute: u64,
        severity: Severity,
        exposure: Duration,
    }
    let cases = [
        Case {
            name: "from warning",
            excursion: vec![reading(0, 9.0), reading(10, 9.5)],
            recovery_minute: 11,
            severity: Severity::Warning,
            exposure: Duration::from_mins(10),
        },
        Case {
            name: "from critical",
            excursion: vec![reading(0, 9.0), reading(30, 9.5), reading(40, 9.0)],
            recovery_minute: 41,
            severity: Severity::Critical,
            exposure: Duration::from_mins(40),
        },
    ];

    let config = config();
    for case in cases {
        let (during, _) = run(&config, case.excursion);
        let open = open_incident(&during).clone();

        let (after, events) = step(&during, reading(case.recovery_minute, 6.5), &config);

        assert_eq!(after.status(), Status::Normal, "{}", case.name);
        assert!(after.open_incident().is_none(), "{}", case.name);
        assert_eq!(
            kinds(&events),
            [Kind::StatusChanged, Kind::Recovered],
            "{}",
            case.name
        );
        let DomainEvent::Recovered {
            incident,
            at: recovered_at,
            temperature,
        } = &events[1]
        else {
            panic!("{}: expected Recovered", case.name);
        };
        assert_eq!(*recovered_at, at(case.recovery_minute), "{}", case.name);
        assert_eq!(*temperature, 6.5, "{}", case.name);
        assert_eq!(
            *incident,
            Incident {
                recovered_at: Some(at(case.recovery_minute)),
                recovery_temperature_c: Some(6.5),
                ..open
            },
            "{}: the incident closes without losing what it recorded",
            case.name
        );
        assert_eq!(incident.severity, case.severity, "{}", case.name);
        assert_eq!(incident.exposure, case.exposure, "{}", case.name);
        assert!(!incident.is_open(), "{}", case.name);
    }
}

#[test]
fn rule_4_next_excursion_after_recovery_is_a_new_incident() {
    let config = config();
    let (state, events) = run(
        &config,
        vec![reading(0, 9.0), reading(1, 5.0), reading(2, 9.0)],
    );
    let first = recovered(&events).incident_id.clone();
    let second = open_incident(&state);
    assert_ne!(second.incident_id, first);
    assert_eq!(second.started_at, at(2));
    assert_eq!(second.exposure, Duration::ZERO);
    assert_eq!(count(&events, Kind::WarningRaised), 2);
}

#[test]
fn rule_5_any_state_goes_stale_on_late_tick_and_keeps_incident_open() {
    struct Case {
        name: &'static str,
        setup: Vec<Input>,
        last_reading_minute: u64,
        from: Status,
    }
    let cases = [
        Case {
            name: "normal",
            setup: vec![reading(0, 5.0)],
            last_reading_minute: 0,
            from: Status::Normal,
        },
        Case {
            name: "warning",
            setup: vec![reading(0, 9.0), reading(1, 9.0)],
            last_reading_minute: 1,
            from: Status::Warning,
        },
        Case {
            name: "critical",
            setup: vec![reading(0, 9.0), reading(30, 9.0)],
            last_reading_minute: 30,
            from: Status::Critical,
        },
    ];

    let config = config();
    for case in cases {
        let (before, _) = run(&config, case.setup);
        assert_eq!(before.status(), case.from, "{}", case.name);
        let last = case.last_reading_minute;

        // Exactly stale_after is not "more than" stale_after.
        let (on_time, events) = step(&before, tick(last + 5), &config);
        assert_eq!(on_time, before, "{}: tick at the limit", case.name);
        assert!(events.is_empty(), "{}: tick at the limit", case.name);

        let (stale, events) = step(&before, tick(last + 6), &config);
        assert_eq!(stale.status(), Status::SensorStale, "{}", case.name);
        assert_eq!(
            events,
            [
                DomainEvent::StatusChanged {
                    from: case.from,
                    to: Status::SensorStale,
                    at: at(last + 6),
                },
                DomainEvent::SensorStale { since: at(last) },
            ],
            "{}",
            case.name
        );
        assert_eq!(
            stale.open_incident(),
            before.open_incident(),
            "{}: the incident is untouched while the sensor is stale",
            case.name
        );
        assert_eq!(stale.last_reading(), before.last_reading(), "{}", case.name);

        // Staleness is announced once.
        let (still_stale, events) = step(&stale, tick(last + 60), &config);
        assert_eq!(still_stale, stale, "{}", case.name);
        assert!(events.is_empty(), "{}", case.name);
    }
}

#[test]
fn rule_5_sensor_that_never_reported_goes_stale() {
    let config = config();
    let (state, events) = run(&config, vec![tick(0), tick(5)]);
    assert_eq!(state.status(), Status::Normal);
    assert!(events.is_empty());

    let (state, events) = step(&state, tick(6), &config);
    assert_eq!(state.status(), Status::SensorStale);
    assert_eq!(
        events,
        [
            DomainEvent::StatusChanged {
                from: Status::Normal,
                to: Status::SensorStale,
                at: at(6),
            },
            DomainEvent::SensorStale { since: at(0) },
        ]
    );
}

#[test]
fn rule_6_stale_resumes_to_normal_warning_or_critical() {
    struct Case {
        name: &'static str,
        setup: Vec<Input>,
        resume: Input,
        to: Status,
        same_incident: bool,
    }
    let cases = [
        Case {
            name: "no incident, in range",
            setup: vec![reading(0, 5.0), tick(6)],
            resume: reading(20, 5.0),
            to: Status::Normal,
            same_incident: false,
        },
        Case {
            name: "no incident, out of range",
            setup: vec![reading(0, 5.0), tick(6)],
            resume: reading(20, 9.0),
            to: Status::Warning,
            same_incident: false,
        },
        Case {
            name: "open warning, still below the critical duration",
            setup: vec![reading(0, 9.0), tick(6)],
            resume: reading(20, 9.0),
            to: Status::Warning,
            same_incident: true,
        },
        Case {
            name: "open warning, critical duration passed during the silence",
            setup: vec![reading(0, 9.0), tick(6)],
            resume: reading(45, 9.0),
            to: Status::Critical,
            same_incident: true,
        },
        Case {
            name: "open critical",
            setup: vec![reading(0, 9.0), reading(30, 9.0), tick(36)],
            resume: reading(50, 9.0),
            to: Status::Critical,
            same_incident: true,
        },
        Case {
            name: "open incident, in range",
            setup: vec![reading(0, 9.0), tick(6)],
            resume: reading(20, 5.0),
            to: Status::Normal,
            same_incident: false,
        },
    ];

    let config = config();
    for case in cases {
        let (stale, _) = run(&config, case.setup);
        assert_eq!(stale.status(), Status::SensorStale, "{}", case.name);
        let Input::Reading(resumed_with) = &case.resume else {
            panic!("{}: resume must be a reading", case.name);
        };
        let resumed_at = resumed_with.observed_at;

        let (after, events) = step(&stale, case.resume, &config);

        assert_eq!(after.status(), case.to, "{}", case.name);
        assert_eq!(
            events[..2],
            [
                DomainEvent::StatusChanged {
                    from: Status::SensorStale,
                    to: case.to,
                    at: resumed_at,
                },
                DomainEvent::SensorResumed { at: resumed_at },
            ],
            "{}",
            case.name
        );
        if case.same_incident {
            assert_eq!(
                open_incident(&after).incident_id,
                open_incident(&stale).incident_id,
                "{}: the incident that was open is the one that continues",
                case.name
            );
            assert_eq!(count(&events, Kind::WarningRaised), 0, "{}", case.name);
        }
    }
}

// ---------------------------------------------------------------------------
// Edge-case policies (docs/decisions/0003-risk-policies.md)
// ---------------------------------------------------------------------------

#[test]
fn edge_out_of_order_reading_is_rejected_and_state_unchanged() {
    let config = config();
    let (before, _) = run(&config, vec![reading(0, 5.0), reading(10, 9.0)]);

    // An older reading that would have ended the incident had it counted.
    let (after, events) = step(&before, reading(5, 5.0), &config);

    assert_eq!(after, before);
    assert_eq!(
        *rejection(&events),
        RejectReason::OutOfOrder {
            observed_at: at(5),
            last_reading_at: at(10),
        }
    );
}

#[test]
fn edge_duplicate_reading_is_rejected_and_state_unchanged() {
    let config = config();
    let (before, _) = run(&config, vec![reading(0, 9.0), reading(30, 9.0)]);

    // Redelivery of the reading that raised the critical alert.
    let (after, events) = step(&before, reading(30, 9.0), &config);
    assert_eq!(after, before);
    assert_eq!(
        *rejection(&events),
        RejectReason::Duplicate {
            observed_at: at(30)
        }
    );

    // Same timestamp, different temperature: the first one stands.
    let (after, events) = step(&before, reading(30, 5.0), &config);
    assert_eq!(after, before);
    assert_eq!(
        *rejection(&events),
        RejectReason::Duplicate {
            observed_at: at(30)
        }
    );
}

#[test]
fn edge_clock_backwards_tick_is_ignored() {
    let config = config();
    let (before, _) = run(&config, vec![reading(60, 5.0)]);

    // A tick from before the last reading: no elapsed time, nothing happens.
    let (after, events) = step(&before, tick(0), &config);
    assert_eq!(after, before);
    assert!(events.is_empty());

    // The same holds for a tick older than the tick that started the wait.
    let (waiting, _) = run(&config, vec![tick(60)]);
    let (after, events) = step(&waiting, tick(0), &config);
    assert_eq!(after, waiting);
    assert!(events.is_empty());

    // A stale sensor is not un-staled by an old tick either.
    let (stale, _) = run(&config, vec![reading(0, 5.0), tick(60)]);
    let (after, events) = step(&stale, tick(1), &config);
    assert_eq!(after, stale);
    assert!(events.is_empty());
}

#[test]
fn edge_clock_backwards_readings_are_rejected_until_time_catches_up() {
    let config = config();
    let (before, _) = run(&config, vec![reading(60, 9.0)]);

    // The sensor clock steps back an hour: its readings are older than the
    // last accepted one and are rejected, one event each.
    let mut state = before.clone();
    for minute in [0, 20, 40, 59] {
        let (next, events) = step(&state, reading(minute, 5.0), &config);
        assert!(matches!(
            rejection(&events),
            RejectReason::OutOfOrder { .. }
        ));
        state = next;
    }
    assert_eq!(state, before, "the last accepted time never moves back");

    // Once the stepped clock passes the last accepted time, readings count again.
    let (after, events) = step(&state, reading(61, 5.0), &config);
    assert_eq!(after.status(), Status::Normal);
    assert_eq!(kinds(&events), [Kind::StatusChanged, Kind::Recovered]);
}

#[test]
fn edge_wrong_refrigerator_reading_is_rejected_and_state_unchanged() {
    let config = config();
    let (before, _) = run(&config, vec![reading(0, 5.0)]);

    let foreign = Input::Reading(TemperatureReading {
        refrigerator_id: RefrigeratorId::new("fridge-2"),
        temperature_c: 20.0,
        observed_at: at(1),
        source: ReadingSource::Sensor,
    });
    let (after, events) = step(&before, foreign, &config);

    assert_eq!(after, before);
    assert_eq!(
        *rejection(&events),
        RejectReason::WrongRefrigerator {
            expected: RefrigeratorId::new(FRIDGE),
            actual: RefrigeratorId::new("fridge-2"),
        }
    );

    // A rejected reading does not count as a sign of life.
    let (after, _) = step(&after, tick(6), &config);
    assert_eq!(after.status(), Status::SensorStale);
}

#[test]
fn edge_non_finite_temperature_is_rejected_and_state_unchanged() {
    let config = config();
    let (before, _) = run(&config, vec![reading(0, 9.0)]);

    for temperature in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        // NaN would otherwise compare as "not out of range" and end the incident.
        let (after, events) = step(&before, reading(1, temperature), &config);
        assert_eq!(after, before, "{temperature}");
        assert_eq!(
            *rejection(&events),
            RejectReason::NonFiniteTemperature,
            "{temperature}"
        );
    }
}

#[test]
fn edge_gap_during_excursion_accrues_exposure_and_flags_data_gap() {
    let config = config();

    // No tick ever arrives: the gap is found from the reading timestamps alone.
    let (state, events) = run(
        &config,
        vec![reading(0, 9.0), reading(4, 9.0), reading(44, 9.0)],
    );
    let incident = open_incident(&state);
    assert_eq!(
        incident.exposure,
        Duration::from_mins(44),
        "the 40 silent minutes count as exposure"
    );
    assert!(incident.has_data_gap);
    assert_eq!(state.status(), Status::Critical);
    assert_eq!(count(&events, Kind::CriticalRaised), 1);

    // Readings exactly stale_after apart are not a gap.
    let (state, _) = run(
        &config,
        vec![reading(0, 9.0), reading(5, 9.0), reading(10, 9.0)],
    );
    assert!(!open_incident(&state).has_data_gap);

    // The flag sticks once set.
    let (state, _) = run(
        &config,
        vec![reading(0, 9.0), reading(6, 9.0), reading(7, 9.0)],
    );
    assert!(open_incident(&state).has_data_gap);
}

#[test]
fn edge_gap_before_recovery_flags_data_gap_without_adding_exposure() {
    let config = config();
    let (_, events) = run(
        &config,
        vec![reading(0, 9.0), reading(2, 9.0), reading(120, 5.0)],
    );

    let incident = recovered(&events);
    assert!(incident.has_data_gap);
    assert_eq!(
        incident.exposure,
        Duration::from_mins(2),
        "exposure ends at the last reading that was out of range"
    );
    assert_eq!(incident.severity, Severity::Warning);
    assert_eq!(incident.recovered_at, Some(at(120)));
    assert_eq!(count(&events, Kind::CriticalRaised), 0);
}

#[test]
fn edge_reading_exactly_on_a_limit_is_in_range() {
    struct Case {
        temperature_c: f64,
        status: Status,
    }
    let cases = [
        Case {
            temperature_c: 8.0,
            status: Status::Normal,
        },
        Case {
            temperature_c: 8.000_001,
            status: Status::Warning,
        },
        Case {
            temperature_c: 2.0,
            status: Status::Normal,
        },
        Case {
            temperature_c: 1.999_999,
            status: Status::Warning,
        },
    ];

    let config = config();
    for case in cases {
        let (state, _) = run(&config, vec![reading(0, case.temperature_c)]);
        assert_eq!(state.status(), case.status, "{} °C", case.temperature_c);
    }

    // On the limit also ends an incident (hysteresis is off).
    let (state, events) = run(&config, vec![reading(0, 9.0), reading(1, 8.0)]);
    assert_eq!(state.status(), Status::Normal);
    assert_eq!(count(&events, Kind::Recovered), 1);
}

#[test]
fn edge_no_lower_limit_means_cold_readings_are_in_range() {
    let config = RefrigeratorConfig::new(
        RefrigeratorId::new(FRIDGE),
        8.0,
        Duration::from_mins(30),
        Duration::from_mins(5),
    );
    let (state, events) = run(&config, vec![reading(0, -40.0)]);
    assert_eq!(state.status(), Status::Normal);
    assert!(events.is_empty());
}

#[test]
fn edge_flapping_without_hysteresis_opens_an_incident_per_crossing() {
    let flapping = vec![
        reading(0, 8.1),
        reading(1, 7.9),
        reading(2, 8.1),
        reading(3, 7.9),
        reading(4, 8.1),
        reading(5, 7.9),
    ];
    let (state, events) = run(&config(), flapping);

    assert_eq!(state.status(), Status::Normal);
    assert_eq!(count(&events, Kind::WarningRaised), 3);
    assert_eq!(count(&events, Kind::Recovered), 3);
}

#[test]
fn edge_hysteresis_keeps_one_incident_open_while_flapping() {
    let config = RefrigeratorConfig {
        hysteresis_c: 0.5,
        ..config()
    };
    let flapping = vec![
        reading(0, 8.1),
        reading(1, 7.9),
        reading(2, 8.1),
        reading(3, 7.9),
        reading(4, 8.1),
        reading(5, 7.9),
    ];
    let (state, events) = run(&config, flapping);

    assert_eq!(state.status(), Status::Warning);
    assert_eq!(kinds(&events), [Kind::StatusChanged, Kind::WarningRaised]);
    let incident = open_incident(&state);
    assert_eq!(incident.started_at, at(0));
    assert_eq!(
        incident.exposure,
        Duration::from_mins(5),
        "readings inside the hysteresis band keep the exposure running"
    );

    // Recovery needs a reading at least the hysteresis inside the limit.
    let (state, events) = step(&state, reading(6, 7.6), &config);
    assert_eq!(state.status(), Status::Warning);
    assert!(events.is_empty());
    let (state, events) = step(&state, reading(7, 7.5), &config);
    assert_eq!(state.status(), Status::Normal);
    assert_eq!(kinds(&events), [Kind::StatusChanged, Kind::Recovered]);
}

#[test]
fn edge_hysteresis_does_not_change_when_an_incident_opens() {
    let config = RefrigeratorConfig {
        hysteresis_c: 0.5,
        ..config()
    };
    // Inside the hysteresis band, with no incident open: still normal.
    let (state, events) = run(&config, vec![reading(0, 7.8), reading(1, 8.0)]);
    assert_eq!(state.status(), Status::Normal);
    assert!(events.is_empty());

    // The cold side mirrors the warm side.
    let (state, _) = run(&config, vec![reading(0, 1.9), reading(1, 2.4)]);
    assert_eq!(state.status(), Status::Warning);
    let (state, _) = step(&state, reading(2, 2.5), &config);
    assert_eq!(state.status(), Status::Normal);
}

#[test]
fn edge_zero_critical_duration_raises_warning_and_critical_together() {
    let config = RefrigeratorConfig {
        critical_after: Duration::ZERO,
        ..config()
    };
    let (state, events) = run(&config, vec![reading(0, 9.0)]);

    assert_eq!(state.status(), Status::Critical);
    assert_eq!(
        kinds(&events),
        [
            Kind::StatusChanged,
            Kind::WarningRaised,
            Kind::CriticalRaised
        ]
    );
    assert_eq!(
        events[0],
        DomainEvent::StatusChanged {
            from: Status::Normal,
            to: Status::Critical,
            at: at(0),
        }
    );
}

#[test]
fn edge_tick_never_changes_anything_but_staleness() {
    let config = config();
    let (before, _) = run(&config, vec![reading(0, 9.0), reading(29, 9.0)]);

    // A tick past the critical duration does not escalate: only a reading can.
    let (after, events) = step(&before, tick(31), &config);
    assert_eq!(after, before);
    assert!(events.is_empty());
}

#[test]
fn invalid_config_is_an_error_for_every_input() {
    struct Case {
        name: &'static str,
        config: RefrigeratorConfig,
        error: RiskError,
    }
    let cases = [
        Case {
            name: "NaN upper limit",
            config: RefrigeratorConfig {
                upper_limit_c: f64::NAN,
                ..config()
            },
            error: RiskError::NonFiniteLimit,
        },
        Case {
            name: "infinite lower limit",
            config: RefrigeratorConfig {
                lower_limit_c: Some(f64::NEG_INFINITY),
                ..config()
            },
            error: RiskError::NonFiniteLimit,
        },
        Case {
            name: "lower above upper",
            config: RefrigeratorConfig {
                lower_limit_c: Some(9.0),
                ..config()
            },
            error: RiskError::LimitsInverted {
                lower: 9.0,
                upper: 8.0,
            },
        },
        Case {
            name: "negative hysteresis",
            config: RefrigeratorConfig {
                hysteresis_c: -0.1,
                ..config()
            },
            error: RiskError::InvalidHysteresis,
        },
        Case {
            name: "NaN hysteresis",
            config: RefrigeratorConfig {
                hysteresis_c: f64::NAN,
                ..config()
            },
            error: RiskError::InvalidHysteresis,
        },
        Case {
            name: "hysteresis wider than half the range",
            config: RefrigeratorConfig {
                hysteresis_c: 3.5,
                ..config()
            },
            error: RiskError::HysteresisTooWide { hysteresis: 3.5 },
        },
        Case {
            name: "zero stale_after",
            config: RefrigeratorConfig {
                stale_after: Duration::ZERO,
                ..config()
            },
            error: RiskError::ZeroStaleAfter,
        },
    ];

    for case in cases {
        assert_eq!(case.config.validate(), Err(case.error), "{}", case.name);
        for input in [reading(0, 5.0), tick(0)] {
            assert_eq!(
                evaluate(&MonitorState::new(), input, &case.config),
                Err(case.error),
                "{}",
                case.name
            );
        }
    }

    // The widest hysteresis that still leaves a recovery point is accepted.
    let widest = RefrigeratorConfig {
        hysteresis_c: 3.0,
        ..config()
    };
    assert_eq!(widest.validate(), Ok(()));
}

#[test]
fn fresh_state_is_normal_and_empty() {
    let state = MonitorState::new();
    assert_eq!(state, MonitorState::default());
    assert_eq!(state.status(), Status::Normal);
    assert!(state.last_reading().is_none());
    assert!(state.open_incident().is_none());
}
