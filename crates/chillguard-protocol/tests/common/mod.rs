//! Fixtures shared by the integration tests: one valid instance of every
//! payload, built from values a running system would produce.

#![allow(dead_code)]
// each test file uses a different subset

// The project standard exempts tests from the unwrap/expect ban (clippy.toml),
// but clippy only recognises `#[test]` functions, not the helpers of an
// integration test file.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use core::time::Duration;

use chillguard_core::asset::AssetState;
use chillguard_core::risk::{
    Input, MonitorState, RefrigeratorConfig, RefrigeratorId, Status, TemperatureReading, Timestamp,
    evaluate,
};
use chillguard_protocol::commands::{
    Acknowledge, GetIncidents, GetStatus, ReplyTo, ScenarioPreset, SetLimits, SimulateScenario,
};
use chillguard_protocol::events::{
    AlertRaised, IncidentRecovered, ReadingAccepted, ReadingObserved, StatusChanged,
};
use chillguard_protocol::responses::{Acknowledgement, IncidentList, StatusSnapshot};
use chillguard_protocol::{Envelope, Incident, Payload, ReadingSource, Severity};

pub const T0_MS: u64 = 1_791_555_000_000;
pub const FRIDGE: &str = "fridge-1";

pub fn at(minute: u64) -> Timestamp {
    Timestamp::from_millis(T0_MS + minute * 60_000)
}

pub fn fridge() -> RefrigeratorId {
    RefrigeratorId::new(FRIDGE)
}

/// Safe range 2..=8 °C, critical after 30 minutes, stale after 5 minutes.
pub fn config() -> RefrigeratorConfig {
    RefrigeratorConfig {
        refrigerator_id: fridge(),
        lower_limit_c: Some(2.0),
        upper_limit_c: 8.0,
        critical_after: Duration::from_mins(30),
        stale_after: Duration::from_mins(5),
        hysteresis_c: 0.0,
    }
}

pub fn reading(minute: u64, temperature_c: f64) -> TemperatureReading {
    TemperatureReading {
        refrigerator_id: fridge(),
        temperature_c,
        observed_at: at(minute),
        source: ReadingSource::Simulated,
    }
}

/// Runs core over an excursion and returns the states after each reading.
fn run(temperatures: &[(u64, f64)]) -> Vec<MonitorState> {
    let config = config();
    let mut state = MonitorState::new();
    let mut states = Vec::new();
    for &(minute, temperature_c) in temperatures {
        let (next, _) = evaluate(
            &state,
            Input::Reading(reading(minute, temperature_c)),
            &config,
        )
        .expect("valid config");
        state = next;
        states.push(state.clone());
    }
    states
}

/// An incident that opened at minute 10 at 9 °C, peaked at 12.5 °C, and is
/// still open as a warning.
pub fn open_incident() -> Incident {
    let states = run(&[(0, 5.0), (10, 9.0), (20, 12.5)]);
    states[2].open_incident().expect("open").clone()
}

/// The same incident after it became critical at minute 40.
pub fn critical_incident() -> Incident {
    let states = run(&[(0, 5.0), (10, 9.0), (20, 12.5), (40, 11.0)]);
    states[3].open_incident().expect("open").clone()
}

/// The same incident, closed by a 6 °C reading at minute 55.
pub fn closed_incident() -> Incident {
    let config = config();
    let states = run(&[(0, 5.0), (10, 9.0), (20, 12.5), (40, 11.0)]);
    let (_, events) =
        evaluate(&states[3], Input::Reading(reading(55, 6.0)), &config).expect("valid config");
    events
        .into_iter()
        .find_map(|event| match event {
            chillguard_core::risk::DomainEvent::Recovered { incident, .. } => Some(incident),
            _ => None,
        })
        .expect("recovered")
}

pub fn reading_observed() -> ReadingObserved {
    ReadingObserved {
        reading: reading(10, 9.0),
    }
}

pub fn reading_accepted() -> ReadingAccepted {
    ReadingAccepted {
        reading: reading(10, 9.0),
        config: RefrigeratorConfig {
            hysteresis_c: 0.5,
            ..config()
        },
    }
}

/// The asset's snapshot: a status view, and never an incident.
pub fn asset_status_snapshot() -> StatusSnapshot {
    let mut asset = AssetState::new(config()).expect("valid config");
    asset.accept_reading(&reading(20, 12.5)).expect("accepted");
    asset.apply_status_change(&fridge(), Status::Normal, Status::Warning, at(10));
    StatusSnapshot::from_asset(&asset, at(21))
}

pub fn status_changed() -> StatusChanged {
    StatusChanged {
        refrigerator_id: fridge(),
        from: Status::Normal,
        to: Status::Warning,
        at: at(10),
    }
}

pub fn alert_raised() -> AlertRaised {
    AlertRaised {
        severity: Severity::Critical,
        incident: critical_incident(),
    }
}

pub fn incident_recovered() -> IncidentRecovered {
    IncidentRecovered {
        incident: closed_incident(),
        at: at(55),
        temperature_c: 6.0,
    }
}

pub fn get_status() -> GetStatus {
    GetStatus {
        reply_to: ReplyTo::new("on_status").expect("valid name"),
    }
}

pub fn get_incidents() -> GetIncidents {
    GetIncidents {
        limit: 20,
        reply_to: ReplyTo::new("on_incidents").expect("valid name"),
    }
}

pub fn set_limits() -> SetLimits {
    SetLimits {
        config: RefrigeratorConfig {
            hysteresis_c: 0.5,
            ..config()
        },
    }
}

pub fn simulate_scenario() -> SimulateScenario {
    SimulateScenario {
        preset: ScenarioPreset::PowerFailure2h,
        seed: 42,
    }
}

pub fn acknowledge() -> Acknowledge {
    Acknowledge {
        incident_id: open_incident().incident_id,
    }
}

pub fn status_snapshot() -> StatusSnapshot {
    let states = run(&[(0, 5.0), (10, 9.0), (20, 12.5), (40, 11.0)]);
    StatusSnapshot::from_state(&states[3], &config(), at(41))
}

pub fn incident_list() -> IncidentList {
    IncidentList {
        refrigerator_id: fridge(),
        incidents: vec![open_incident(), closed_incident()],
        acknowledgements: vec![Acknowledgement {
            incident_id: closed_incident().incident_id,
            acknowledged_at: at(58),
        }],
        truncated: true,
        as_of: at(60),
    }
}

/// Every payload, named, as encoded bytes. The golden and round-trip tests
/// iterate over this so that adding a payload type without a fixture here is
/// caught by the test that checks the list is complete.
pub fn all_encoded() -> Vec<(&'static str, Vec<u8>)> {
    fn enc<T: Payload>(name: &'static str, body: T) -> (&'static str, Vec<u8>) {
        (name, Envelope::new(body).encode().expect("fixture encodes"))
    }
    vec![
        enc("reading_observed", reading_observed()),
        enc("reading_accepted", reading_accepted()),
        enc("status_changed", status_changed()),
        enc("alert_raised", alert_raised()),
        enc("incident_recovered", incident_recovered()),
        enc("get_status", get_status()),
        enc("get_incidents", get_incidents()),
        enc("set_limits", set_limits()),
        enc("simulate_scenario", simulate_scenario()),
        enc("acknowledge", acknowledge()),
        enc("status_snapshot", status_snapshot()),
        enc("status_snapshot_from_asset", asset_status_snapshot()),
        enc("incident_list", incident_list()),
    ]
}
