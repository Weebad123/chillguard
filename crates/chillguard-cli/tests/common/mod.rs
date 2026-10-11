//! Fixtures shared by the integration tests: protocol values built the way
//! the cells build them, from core.

#![allow(dead_code)]
// each test file uses a different subset
// The project standard exempts tests from the unwrap/expect ban (clippy.toml),
// but clippy only recognises `#[test]` functions, not the helpers of an
// integration test file.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

use std::time::Duration;

use chillguard_core::asset::AssetState;
use chillguard_core::risk::{
    DomainEvent, Incident, Input, MonitorState, ReadingSource, RefrigeratorConfig, RefrigeratorId,
    Status, TemperatureReading, Timestamp, evaluate,
};
use chillguard_protocol::responses::{Acknowledgement, IncidentList, StatusSnapshot};

/// 2026-10-10T02:00:00Z.
pub const T0_MS: u64 = 1_791_597_600_000;
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
        hysteresis_c: 0.5,
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

/// Runs core over readings; returns the final state and every event.
pub fn run(temperatures: &[(u64, f64)]) -> (MonitorState, Vec<DomainEvent>) {
    let config = config();
    let mut state = MonitorState::new();
    let mut all = Vec::new();
    for &(minute, temperature_c) in temperatures {
        let input = Input::Reading(reading(minute, temperature_c));
        let (next, events) = evaluate(&state, input, &config).expect("valid config");
        state = next;
        all.extend(events);
    }
    (state, all)
}

/// Opened at minute 10 at 9.2 °C, critical since minute 40, peak 12.4 °C,
/// with a silence of 16 minutes in it, still open at minute 44.
pub fn open_incident() -> Incident {
    let (state, _) = run(&[
        (0, 5.0),
        (10, 9.2),
        (14, 12.4),
        (30, 11.0),
        (40, 10.5),
        (44, 10.1),
    ]);
    state.open_incident().expect("open").clone()
}

/// A short excursion of yesterday's kind: minutes 100 to 104, recovered at 105.
pub fn closed_incident() -> Incident {
    let (_, events) = run(&[(99, 5.0), (100, 8.6), (102, 9.1), (104, 8.3), (105, 6.9)]);
    events
        .into_iter()
        .find_map(|event| match event {
            DomainEvent::Recovered { incident, .. } => Some(incident),
            _ => None,
        })
        .expect("recovered")
}

/// The asset's snapshot with a reading at `minute` and the given status view.
pub fn snapshot(
    status: Status,
    last: Option<(u64, f64)>,
    config: RefrigeratorConfig,
) -> StatusSnapshot {
    let mut asset = AssetState::new(config).expect("valid config");
    let mut as_of = at(0);
    if let Some((minute, temperature_c)) = last {
        asset
            .accept_reading(&reading(minute, temperature_c))
            .expect("accepted");
        as_of = Timestamp::from_millis(at(minute).as_millis() + 1_250);
    }
    if status != Status::Normal {
        asset.apply_status_change(&fridge(), Status::Normal, status, at(10));
    }
    StatusSnapshot::from_asset(&asset, as_of)
}

/// The open incident first, then the closed one, which is acknowledged.
pub fn incident_list() -> IncidentList {
    let closed = closed_incident();
    IncidentList {
        refrigerator_id: fridge(),
        acknowledgements: vec![Acknowledgement {
            incident_id: closed.incident_id.clone(),
            acknowledged_at: at(110),
        }],
        incidents: vec![open_incident(), closed],
        truncated: true,
        as_of: at(120),
    }
}
