//! Output formatting, pinned by snapshots (`tests/snapshots/`).
//!
//! A failure here means the text or JSON an operator or a script sees has
//! changed. If that is intended, review the difference and accept it:
//!
//! ```sh
//! INSTA_UPDATE=always cargo test -p chillguard-cli --test render
//! git diff crates/chillguard-cli/tests/snapshots
//! ```

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use std::time::Duration;

use chillguard_cli::render::{self, AckOutcome, OpenIncident, StatusReport, WatchEvent};
use chillguard_core::risk::{RefrigeratorConfig, Severity, Status};
use chillguard_protocol::commands::ScenarioPreset;
use chillguard_protocol::events::{AlertRaised, IncidentRecovered, ReadingAccepted, StatusChanged};
use chillguard_protocol::responses::IncidentList;
use common::{
    at, closed_incident, config, fridge, incident_list, open_incident, reading, snapshot,
};
use insta::assert_snapshot;

fn pretty(value: &serde_json::Value) -> String {
    format!("{value:#}")
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

#[test]
fn status_normal() {
    let report = StatusReport {
        snapshot: snapshot(Status::Normal, Some((44, 4.12)), config()),
        incident: OpenIncident::None,
    };
    assert_snapshot!("status_normal", render::status(&report));
    assert_snapshot!("status_normal_json", pretty(&render::status_json(&report)));
}

#[test]
fn status_critical_with_an_acknowledged_incident() {
    let report = StatusReport {
        snapshot: snapshot(Status::Critical, Some((44, 10.1)), config()),
        incident: OpenIncident::Open {
            incident: open_incident(),
            acknowledged_at: Some(at(42)),
        },
    };
    assert_snapshot!("status_critical", render::status(&report));
    assert_snapshot!(
        "status_critical_json",
        pretty(&render::status_json(&report))
    );
}

#[test]
fn status_before_any_reading_with_the_monitor_unreachable() {
    let no_lower = RefrigeratorConfig {
        lower_limit_c: None,
        hysteresis_c: 0.0,
        critical_after: Duration::from_secs(90),
        ..config()
    };
    let report = StatusReport {
        snapshot: snapshot(Status::Normal, None, no_lower),
        incident: OpenIncident::Unknown(
            "`risk-monitor-cell` is not deployed on a runtime this client can reach".to_string(),
        ),
    };
    assert_snapshot!("status_no_reading_monitor_unknown", render::status(&report));
    assert_snapshot!(
        "status_no_reading_monitor_unknown_json",
        pretty(&render::status_json(&report))
    );
}

#[test]
fn status_says_when_view_and_monitor_disagree() {
    // The asset's view missed the change to warning.
    let behind = StatusReport {
        snapshot: snapshot(Status::Normal, Some((44, 10.1)), config()),
        incident: OpenIncident::Open {
            incident: open_incident(),
            acknowledged_at: None,
        },
    };
    assert_snapshot!("status_view_behind", render::status(&behind));

    // The asset's view missed the recovery.
    let stuck = StatusReport {
        snapshot: snapshot(Status::Critical, Some((120, 5.0)), config()),
        incident: OpenIncident::None,
    };
    assert_snapshot!("status_view_stuck", render::status(&stuck));

    // A stale sensor with no incident is not a disagreement.
    let stale = StatusReport {
        snapshot: snapshot(Status::SensorStale, Some((44, 5.0)), config()),
        incident: OpenIncident::None,
    };
    assert!(!render::status(&stale).contains("note"));
}

// ---------------------------------------------------------------------------
// incidents
// ---------------------------------------------------------------------------

#[test]
fn incidents_table() {
    let list = incident_list();
    assert_snapshot!("incidents", render::incidents(&list));
    assert_snapshot!("incidents_json", pretty(&render::incidents_json(&list)));
}

#[test]
fn incidents_single_and_empty() {
    let one = IncidentList {
        refrigerator_id: fridge(),
        incidents: vec![closed_incident()],
        acknowledgements: vec![],
        truncated: false,
        as_of: at(120),
    };
    assert_snapshot!("incidents_one", render::incidents(&one));

    let none = IncidentList {
        incidents: vec![],
        ..one
    };
    assert_snapshot!("incidents_none", render::incidents(&none));
    assert_snapshot!(
        "incidents_none_json",
        pretty(&render::incidents_json(&none))
    );
}

// ---------------------------------------------------------------------------
// ack, set-limits, simulate
// ---------------------------------------------------------------------------

#[test]
fn ack_outcomes() {
    let id = "fridge-1-1791598200000";
    let done = AckOutcome::Acknowledged(at(42));
    let repeat = AckOutcome::AlreadyAcknowledged(at(42));
    assert_snapshot!("ack", render::ack(id, done));
    assert_snapshot!("ack_json", pretty(&render::ack_json(id, done)));
    assert_snapshot!("ack_repeat", render::ack(id, repeat));
    assert_snapshot!("ack_repeat_json", pretty(&render::ack_json(id, repeat)));
}

#[test]
fn limits_change() {
    let previous = config();
    let now = RefrigeratorConfig {
        lower_limit_c: None,
        upper_limit_c: 10.0,
        critical_after: Duration::from_mins(90),
        ..config()
    };
    assert_snapshot!("limits_changed", render::limits_change(&previous, &now));
    assert_snapshot!(
        "limits_changed_json",
        pretty(&render::limits_change_json(&previous, &now))
    );
    assert_snapshot!(
        "limits_unchanged",
        render::limits_change(&previous, &previous)
    );
    assert_snapshot!(
        "limits_unchanged_json",
        pretty(&render::limits_change_json(&previous, &previous))
    );
}

#[test]
fn simulate() {
    let preset = ScenarioPreset::PowerFailure2h;
    assert_snapshot!(
        "simulate",
        render::simulate("site-a-sensor-cell", preset, 7)
    );
    assert_snapshot!(
        "simulate_json",
        pretty(&render::simulate_json("site-a-sensor-cell", preset, 7))
    );
}

// ---------------------------------------------------------------------------
// watch
// ---------------------------------------------------------------------------

fn watch_events() -> Vec<WatchEvent> {
    let (_, events) = common::run(&[(0, 5.0), (10, 9.2), (14, 12.4), (30, 11.0), (40, 10.5)]);
    let raised = |severity| {
        events
            .iter()
            .find_map(|event| match (event, severity) {
                (
                    chillguard_core::risk::DomainEvent::WarningRaised { incident },
                    Severity::Warning,
                )
                | (
                    chillguard_core::risk::DomainEvent::CriticalRaised { incident },
                    Severity::Critical,
                ) => Some(incident.clone()),
                _ => None,
            })
            .unwrap()
    };
    let closed = closed_incident();
    vec![
        WatchEvent::Reading(ReadingAccepted {
            reading: reading(9, 4.12),
            config: config(),
        }),
        WatchEvent::Status(StatusChanged {
            refrigerator_id: fridge(),
            from: Status::Normal,
            to: Status::Warning,
            at: at(10),
        }),
        WatchEvent::Alert(AlertRaised {
            severity: Severity::Warning,
            incident: raised(Severity::Warning),
        }),
        WatchEvent::Alert(AlertRaised {
            severity: Severity::Critical,
            incident: raised(Severity::Critical),
        }),
        WatchEvent::Status(StatusChanged {
            refrigerator_id: fridge(),
            from: Status::Critical,
            to: Status::SensorStale,
            at: at(50),
        }),
        WatchEvent::Recovered(IncidentRecovered {
            at: closed.recovered_at.unwrap(),
            temperature_c: closed.recovery_temperature_c.unwrap(),
            incident: closed,
        }),
    ]
}

#[test]
fn watch_lines() {
    let header = render::watch_header(&snapshot(Status::Normal, Some((9, 4.12)), config()));
    let lines: Vec<String> = watch_events().iter().map(render::watch_event).collect();
    assert_snapshot!("watch", format!("{header}\n{}", lines.join("\n")));

    let fresh = render::watch_header(&snapshot(Status::Normal, None, config()));
    assert_snapshot!("watch_header_no_reading", fresh);
}

#[test]
fn watch_json_is_one_object_per_line() {
    let lines: Vec<String> = watch_events()
        .iter()
        .map(|event| render::watch_event_json(event).to_string())
        .collect();
    for line in &lines {
        assert!(!line.contains('\n'));
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(value["event"].is_string() && value["body"].is_object());
    }
    assert_snapshot!("watch_json", lines.join("\n"));
}

#[test]
fn every_watch_event_names_its_refrigerator() {
    for event in watch_events() {
        assert_eq!(event.refrigerator_id(), &fridge());
    }
}

// ---------------------------------------------------------------------------
// building blocks
// ---------------------------------------------------------------------------

#[test]
fn times_and_spans() {
    struct Case {
        duration: Duration,
        text: &'static str,
    }
    assert_eq!(render::time(at(0)), "2026-10-10T02:00:00Z");
    assert_eq!(render::time(at(37)), "2026-10-10T02:37:00Z");
    let cases = [
        Case {
            duration: Duration::ZERO,
            text: "0s",
        },
        Case {
            duration: Duration::from_millis(486),
            text: "486ms",
        },
        Case {
            duration: Duration::from_millis(1_250),
            text: "1s",
        },
        Case {
            duration: Duration::from_secs(59),
            text: "59s",
        },
        Case {
            duration: Duration::from_millis(714_215),
            text: "11m 54s",
        },
        Case {
            duration: Duration::from_mins(30),
            text: "30m",
        },
        Case {
            duration: Duration::from_mins(90),
            text: "1h 30m",
        },
        Case {
            duration: Duration::from_hours(26),
            text: "1day 2h",
        },
    ];
    for case in cases {
        assert_eq!(render::span(case.duration), case.text);
    }
}
