//! Property tests: invariants of the risk state machine that must hold for
//! every sequence of inputs, including out-of-order, duplicate, foreign and
//! non-finite readings, clocks that step backwards, and long silences.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use chillguard_core::risk::{
    DomainEvent, IncidentId, Input, MonitorState, ReadingSource, RefrigeratorConfig,
    RefrigeratorId, Severity, Status, TemperatureReading, Timestamp, evaluate,
};
use proptest::prelude::*;

const T0_MS: u64 = 1_791_555_000_000;
const FRIDGE: &str = "fridge-1";
const LOWER_C: f64 = 2.0;
const UPPER_C: f64 = 8.0;

/// One generated step: how far the clock moves, and what happens then.
#[derive(Debug, Clone)]
enum Step {
    Reading {
        delta_ms: i64,
        temperature_c: f64,
        foreign: bool,
    },
    Tick {
        delta_ms: i64,
    },
}

/// Mostly forward by up to 20 minutes; sometimes zero or backwards (duplicate
/// and out-of-order input); sometimes a silence of up to two hours.
fn delta_ms() -> impl Strategy<Value = i64> {
    prop_oneof![
        8 => 1_000_i64..1_200_000,
        1 => -600_000_i64..=0,
        1 => 1_200_000_i64..7_200_000,
    ]
}

/// Mostly around the 2..=8 °C range; sometimes exactly on or next to a
/// limit; sometimes not a number at all.
fn temperature_c() -> impl Strategy<Value = f64> {
    prop_oneof![
        12 => -2.0_f64..12.0,
        3 => prop::sample::select(vec![LOWER_C, UPPER_C, 1.99, 2.01, 7.99, 8.01, 7.5, 2.5]),
        1 => prop::sample::select(vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY]),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => (delta_ms(), temperature_c(), prop::bool::weighted(0.05)).prop_map(
            |(delta_ms, temperature_c, foreign)| Step::Reading {
                delta_ms,
                temperature_c,
                foreign,
            }
        ),
        1 => delta_ms().prop_map(|delta_ms| Step::Tick { delta_ms }),
    ]
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec(step(), 0..200)
}

/// Critical after 0 to 60 minutes, stale after 1 second to 30 minutes,
/// hysteresis off or half a degree, with or without a lower limit.
fn config() -> impl Strategy<Value = RefrigeratorConfig> {
    (
        0_u64..=3_600_000,
        1_000_u64..=1_800_000,
        prop::sample::select(vec![0.0, 0.5]),
        any::<bool>(),
    )
        .prop_map(
            |(critical_ms, stale_ms, hysteresis_c, has_lower)| RefrigeratorConfig {
                refrigerator_id: RefrigeratorId::new(FRIDGE),
                lower_limit_c: has_lower.then_some(LOWER_C),
                upper_limit_c: UPPER_C,
                critical_after: Duration::from_millis(critical_ms),
                stale_after: Duration::from_millis(stale_ms),
                hysteresis_c,
            },
        )
}

/// One call of `evaluate` and everything around it.
struct Transition {
    input: Input,
    before: MonitorState,
    after: MonitorState,
    events: Vec<DomainEvent>,
}

impl Transition {
    fn rejected(&self) -> bool {
        self.events
            .iter()
            .any(|event| matches!(event, DomainEvent::ReadingRejected { .. }))
    }
}

/// Turns the steps into inputs on one shared clock and runs them through a
/// fresh monitor.
fn run(config: &RefrigeratorConfig, steps: &[Step]) -> Result<Vec<Transition>, TestCaseError> {
    let mut clock_ms = T0_MS;
    let mut state = MonitorState::new();
    let mut transitions = Vec::with_capacity(steps.len());
    for step in steps {
        let input = match *step {
            Step::Reading {
                delta_ms,
                temperature_c,
                foreign,
            } => {
                clock_ms = clock_ms.saturating_add_signed(delta_ms);
                Input::Reading(TemperatureReading {
                    refrigerator_id: RefrigeratorId::new(if foreign { "fridge-2" } else { FRIDGE }),
                    temperature_c,
                    observed_at: Timestamp::from_millis(clock_ms),
                    source: ReadingSource::Simulated,
                })
            }
            Step::Tick { delta_ms } => {
                clock_ms = clock_ms.saturating_add_signed(delta_ms);
                Input::Tick {
                    now: Timestamp::from_millis(clock_ms),
                }
            }
        };
        let (after, events) = evaluate(&state, input.clone(), config)
            .map_err(|error| TestCaseError::fail(format!("valid config rejected: {error}")))?;
        transitions.push(Transition {
            input,
            before: state,
            after: after.clone(),
            events,
        });
        state = after;
    }
    Ok(transitions)
}

proptest! {
    /// At most one incident is open at any time: an incident is only ever
    /// opened when none is open, every other incident event refers to the
    /// open one, and the state agrees with the event stream.
    #[test]
    fn at_most_one_open_incident(config in config(), steps in steps()) {
        let mut open: Option<IncidentId> = None;
        for transition in run(&config, &steps)? {
            for event in &transition.events {
                match event {
                    DomainEvent::WarningRaised { incident } => {
                        prop_assert_eq!(&open, &None, "opened while another incident was open");
                        prop_assert!(incident.is_open());
                        open = Some(incident.incident_id.clone());
                    }
                    DomainEvent::CriticalRaised { incident } => {
                        prop_assert_eq!(open.as_ref(), Some(&incident.incident_id));
                        prop_assert!(incident.is_open());
                    }
                    DomainEvent::Recovered { incident, .. } => {
                        prop_assert_eq!(open.as_ref(), Some(&incident.incident_id));
                        prop_assert!(!incident.is_open());
                        open = None;
                    }
                    _ => {}
                }
            }
            let in_state = transition.after.open_incident().map(|i| &i.incident_id);
            prop_assert_eq!(in_state, open.as_ref());
        }
    }

    /// Exposure never decreases within an incident, from the moment it opens
    /// to the record carried by its `Recovered` event.
    #[test]
    fn exposure_never_decreases_within_an_incident(config in config(), steps in steps()) {
        for transition in run(&config, &steps)? {
            let Some(before) = transition.before.open_incident() else {
                continue;
            };
            let closed = transition.events.iter().find_map(|event| match event {
                DomainEvent::Recovered { incident, .. } => Some(incident),
                _ => None,
            });
            let Some(after) = closed.or(transition.after.open_incident()) else {
                return Err(TestCaseError::fail("an open incident vanished without Recovered"));
            };
            prop_assert_eq!(&after.incident_id, &before.incident_id);
            prop_assert!(
                after.exposure >= before.exposure,
                "exposure went from {:?} to {:?}",
                before.exposure,
                after.exposure
            );
        }
    }

    /// Critical is never reached without a prior out-of-range reading: every
    /// critical incident started at the timestamp of an accepted reading that
    /// was outside the configured limits.
    #[test]
    fn critical_requires_a_prior_out_of_range_reading(config in config(), steps in steps()) {
        let mut out_of_range_at: BTreeSet<Timestamp> = BTreeSet::new();
        for transition in run(&config, &steps)? {
            if let Input::Reading(reading) = &transition.input {
                let t = reading.temperature_c;
                let outside = t > UPPER_C || config.lower_limit_c.is_some_and(|lower| t < lower);
                if outside && !transition.rejected() {
                    out_of_range_at.insert(reading.observed_at);
                }
            }

            for event in &transition.events {
                if let DomainEvent::CriticalRaised { incident } = event {
                    prop_assert!(out_of_range_at.contains(&incident.started_at));
                }
            }
            if transition.after.status() == Status::Critical {
                let Some(incident) = transition.after.open_incident() else {
                    return Err(TestCaseError::fail("critical status without an open incident"));
                };
                prop_assert_eq!(incident.severity, Severity::Critical);
                prop_assert!(out_of_range_at.contains(&incident.started_at));
            }
        }
    }

    /// For any reading sequence, each incident emits at most one warning and
    /// at most one critical, and never a critical before its warning.
    #[test]
    fn each_incident_emits_at_most_one_warning_and_one_critical(
        config in config(),
        steps in steps(),
    ) {
        let mut warnings: BTreeMap<IncidentId, u32> = BTreeMap::new();
        let mut criticals: BTreeMap<IncidentId, u32> = BTreeMap::new();
        for transition in run(&config, &steps)? {
            for event in &transition.events {
                match event {
                    DomainEvent::WarningRaised { incident } => {
                        *warnings.entry(incident.incident_id.clone()).or_default() += 1;
                    }
                    DomainEvent::CriticalRaised { incident } => {
                        prop_assert_eq!(warnings.get(&incident.incident_id), Some(&1));
                        *criticals.entry(incident.incident_id.clone()).or_default() += 1;
                    }
                    _ => {}
                }
            }
        }
        prop_assert!(warnings.values().all(|count| *count == 1), "{:?}", warnings);
        prop_assert!(criticals.values().all(|count| *count == 1), "{:?}", criticals);
    }

    /// Whenever an open incident's record differs from before, the state
    /// says so, and what it says to persist is the current record; when
    /// nothing about the incident changed, it says nothing.
    #[test]
    fn changed_incident_reports_exactly_the_changes(config in config(), steps in steps()) {
        for transition in run(&config, &steps)? {
            let changed = transition.after.changed_incident(&transition.before);
            let now = transition.after.open_incident();
            let before = transition.before.open_incident();
            match (now, before) {
                (Some(now), Some(before)) if now == before => prop_assert_eq!(changed, None),
                (Some(now), _) => prop_assert_eq!(changed, Some(now)),
                (None, _) => prop_assert_eq!(changed, None),
            }
        }
    }

    /// A rejected reading changes nothing and is reported by exactly one
    /// event; an accepted reading always becomes the last reading.
    #[test]
    fn rejected_readings_leave_the_state_unchanged(config in config(), steps in steps()) {
        for transition in run(&config, &steps)? {
            let Input::Reading(reading) = &transition.input else {
                continue;
            };
            if transition.rejected() {
                prop_assert_eq!(transition.events.len(), 1);
                prop_assert_eq!(&transition.after, &transition.before);
            } else {
                let last = transition.after.last_reading().map(|last| last.observed_at);
                prop_assert_eq!(last, Some(reading.observed_at));
            }
        }
    }
}
