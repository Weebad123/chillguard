//! Everything core produces must be expressible on the wire: a whole
//! power-failure timeline is run through `risk::evaluate`, every domain event
//! is translated, encoded, and decoded back.

// The project standard exempts tests from the unwrap/expect ban (clippy.toml),
// but clippy only recognises `#[test]` functions, not the helpers of an
// integration test file.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use chillguard_core::risk::{DomainEvent, Input, MonitorState, Status, evaluate};
use chillguard_core::simulator::{Scenario, Simulator, SimulatorConfig};
use chillguard_protocol::commands::ScenarioPreset;
use chillguard_protocol::events::{
    AlertRaised, IncidentRecovered, MonitorEvent, ReadingObserved, StatusChanged,
};
use chillguard_protocol::responses::StatusSnapshot;
use chillguard_protocol::{Envelope, Severity, Timestamp};

#[test]
fn every_domain_event_of_a_power_failure_crosses_the_wire() {
    let risk_config = chillguard_core::risk::RefrigeratorConfig {
        hysteresis_c: 0.5,
        ..common::config()
    };
    let mut simulator = Simulator::new(
        SimulatorConfig::new(common::fridge()),
        ScenarioPreset::PowerFailure2h.scenario(),
        7,
    )
    .expect("valid");

    let mut state = MonitorState::new();
    let mut published = Vec::new();
    let mut dropped = Vec::new();
    for minute in 0..=300 {
        let now = Timestamp::from_millis(common::T0_MS + minute * 60_000);
        let reading = simulator.next_reading(now);

        // The sensor cell publishes the reading...
        let bytes = Envelope::new(ReadingObserved {
            reading: reading.clone(),
        })
        .encode()
        .expect("reading encodes");
        let received = Envelope::<ReadingObserved>::decode(&bytes).expect("reading decodes");

        // ...the monitor cell evaluates what it received and publishes the
        // result.
        let (next, events) = evaluate(&state, Input::Reading(received.body.reading), &risk_config)
            .expect("valid config");
        state = next;
        for event in events {
            match MonitorEvent::from_domain(&common::fridge(), &event) {
                Some(outbound) => {
                    let bytes = outbound.encode().expect("domain event encodes");
                    published.push((outbound.name(), bytes));
                }
                None => dropped.push(event),
            }
        }

        // And a snapshot can be taken at any moment.
        let snapshot = StatusSnapshot::from_state(&state, &risk_config, now);
        let bytes = Envelope::new(snapshot.clone())
            .encode()
            .expect("snapshot encodes");
        assert_eq!(
            Envelope::<StatusSnapshot>::decode(&bytes)
                .expect("snapshot decodes")
                .body,
            snapshot
        );
    }

    assert!(
        dropped.is_empty(),
        "a reading-only timeline has no stale, resumed, or rejected events: {dropped:?}"
    );

    // Decode everything back as the dashboard would.
    let mut statuses = Vec::new();
    let mut alerts = Vec::new();
    let mut recoveries = Vec::new();
    for (name, bytes) in &published {
        match *name {
            "status_changed" => {
                statuses.push(Envelope::<StatusChanged>::decode(bytes).expect(name).body);
            }
            "alert_raised" => {
                alerts.push(Envelope::<AlertRaised>::decode(bytes).expect(name).body);
            }
            "incident_recovered" => {
                recoveries.push(
                    Envelope::<IncidentRecovered>::decode(bytes)
                        .expect(name)
                        .body,
                );
            }
            other => panic!("unexpected event name {other}"),
        }
    }
    assert_eq!(
        statuses.iter().map(|s| (s.from, s.to)).collect::<Vec<_>>(),
        [
            (Status::Normal, Status::Warning),
            (Status::Warning, Status::Critical),
            (Status::Critical, Status::Normal),
        ]
    );
    assert_eq!(
        alerts.iter().map(|a| a.severity).collect::<Vec<_>>(),
        [Severity::Warning, Severity::Critical]
    );
    assert_eq!(recoveries.len(), 1);
    assert_eq!(recoveries[0].incident.severity, Severity::Critical);
    assert_eq!(
        recoveries[0].incident.incident_id, alerts[0].incident.incident_id,
        "one incident from warning to recovery"
    );
}

#[test]
fn domain_events_without_a_wire_counterpart_map_to_none() {
    let fridge = common::fridge();
    for event in [
        DomainEvent::SensorStale {
            since: common::at(0),
        },
        DomainEvent::SensorResumed { at: common::at(9) },
        DomainEvent::ReadingRejected {
            reason: chillguard_core::risk::RejectReason::NonFiniteTemperature,
        },
    ] {
        assert_eq!(
            MonitorEvent::from_domain(&fridge, &event),
            None,
            "{event:?}"
        );
    }
}

#[test]
fn presets_name_the_core_scenarios() {
    assert_eq!(ScenarioPreset::NormalDay.scenario(), Scenario::normal_day());
    assert_eq!(
        ScenarioPreset::PowerFailure2h.scenario(),
        Scenario::power_failure_2h()
    );
    assert_eq!(
        ScenarioPreset::ShortDoorOpen.scenario(),
        Scenario::short_door_open()
    );
}
