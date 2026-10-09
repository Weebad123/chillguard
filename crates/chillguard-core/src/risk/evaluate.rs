//! The transition function.

use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

use super::config::RefrigeratorConfig;
use super::error::RiskError;
use super::event::{DomainEvent, RejectReason};
use super::incident::{Incident, Severity};
use super::model::{Input, TemperatureReading, Timestamp};
use super::state::{LastReading, MonitorState, Status};

/// Applies one input to the monitor and returns the next state and the
/// events to emit.
///
/// The function is pure: the same arguments always give the same result. A
/// reading that cannot be used is not an error; it leaves the state unchanged
/// and yields a single [`DomainEvent::ReadingRejected`].
///
/// # Errors
///
/// Returns a [`RiskError`] when `config` is invalid; see
/// [`RefrigeratorConfig::validate`]. Nothing else fails.
pub fn evaluate(
    state: &MonitorState,
    input: Input,
    config: &RefrigeratorConfig,
) -> Result<(MonitorState, Vec<DomainEvent>), RiskError> {
    config.validate()?;
    Ok(match input {
        Input::Reading(reading) => on_reading(state, &reading, config),
        Input::Tick { now } => on_tick(state, now, config),
    })
}

/// A tick only ever starts the stale clock or declares the sensor stale.
fn on_tick(
    state: &MonitorState,
    now: Timestamp,
    config: &RefrigeratorConfig,
) -> (MonitorState, Vec<DomainEvent>) {
    let mut next = state.clone();
    let Some(since) = state.silent_since() else {
        next.waiting_since = Some(now);
        return (next, Vec::new());
    };
    // A tick earlier than `since` (clock went backwards) saturates to zero.
    if state.stale || now.saturating_duration_since(since) <= config.stale_after {
        return (next, Vec::new());
    }
    next.stale = true;
    let events = vec![
        DomainEvent::StatusChanged {
            from: state.status(),
            to: Status::SensorStale,
            at: now,
        },
        DomainEvent::SensorStale { since },
    ];
    (next, events)
}

fn on_reading(
    state: &MonitorState,
    reading: &TemperatureReading,
    config: &RefrigeratorConfig,
) -> (MonitorState, Vec<DomainEvent>) {
    if let Some(reason) = rejection(state, reading, config) {
        return (state.clone(), vec![DomainEvent::ReadingRejected { reason }]);
    }

    let at = reading.observed_at;
    let temperature = reading.temperature_c;
    // Judged on reading timestamps, whether or not a tick noticed the silence.
    let after_gap = state
        .last_reading
        .is_some_and(|last| at.saturating_duration_since(last.observed_at) > config.stale_after);

    let mut alerts = Vec::new();
    let mut open_incident = match state.open_incident.clone() {
        Some(mut incident) if config.is_recovered(temperature) => {
            incident.close(reading, after_gap);
            alerts.push(DomainEvent::Recovered {
                incident,
                at,
                temperature,
            });
            None
        }
        Some(mut incident) => {
            incident.extend(reading, after_gap);
            Some(incident)
        }
        None if config.is_out_of_range(temperature) => {
            let incident = Incident::open(&config.refrigerator_id, reading);
            alerts.push(DomainEvent::WarningRaised {
                incident: incident.clone(),
            });
            Some(incident)
        }
        None => None,
    };
    if let Some(incident) = open_incident.as_mut()
        && incident.severity == Severity::Warning
        && incident.exposure >= config.critical_after
    {
        incident.severity = Severity::Critical;
        alerts.push(DomainEvent::CriticalRaised {
            incident: incident.clone(),
        });
    }

    let next = MonitorState {
        last_reading: Some(LastReading {
            temperature_c: temperature,
            observed_at: at,
        }),
        open_incident,
        stale: false,
        waiting_since: None,
    };

    let mut events = Vec::new();
    let (from, to) = (state.status(), next.status());
    if from != to {
        events.push(DomainEvent::StatusChanged { from, to, at });
    }
    if state.stale {
        events.push(DomainEvent::SensorResumed { at });
    }
    events.append(&mut alerts);
    (next, events)
}

/// Why a reading must be ignored, if it must.
fn rejection(
    state: &MonitorState,
    reading: &TemperatureReading,
    config: &RefrigeratorConfig,
) -> Option<RejectReason> {
    if reading.refrigerator_id != config.refrigerator_id {
        return Some(RejectReason::WrongRefrigerator {
            expected: config.refrigerator_id.clone(),
            actual: reading.refrigerator_id.clone(),
        });
    }
    // NaN compares false against every limit and would read as "in range".
    if !reading.temperature_c.is_finite() {
        return Some(RejectReason::NonFiniteTemperature);
    }
    let last_reading_at = state.last_reading?.observed_at;
    match reading.observed_at.cmp(&last_reading_at) {
        Ordering::Greater => None,
        Ordering::Equal => Some(RejectReason::Duplicate {
            observed_at: reading.observed_at,
        }),
        Ordering::Less => Some(RejectReason::OutOfOrder {
            observed_at: reading.observed_at,
            last_reading_at,
        }),
    }
}
