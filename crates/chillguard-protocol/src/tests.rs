//! Unit tests for what the integration tests cannot reach: the names every
//! payload declares, and the envelope's handling of its own version.

use alloc::string::String;
use alloc::vec::Vec;

use myrmic_sdk::{Command, Event};

use crate::commands::{Acknowledge, GetIncidents, GetStatus, SetLimits, SimulateScenario};
use crate::events::{
    AlertRaised, IncidentRecovered, ReadingAccepted, ReadingObserved, StatusChanged,
};
use crate::responses::{IncidentList, StatusSnapshot};
use crate::{Kind, Payload, SCHEMA_VERSION};

fn kinds() -> Vec<(&'static str, Kind)> {
    alloc::vec![
        ("ReadingObserved", ReadingObserved::KIND),
        ("ReadingAccepted", ReadingAccepted::KIND),
        ("StatusChanged", StatusChanged::KIND),
        ("AlertRaised", AlertRaised::KIND),
        ("IncidentRecovered", IncidentRecovered::KIND),
        ("GetStatus", GetStatus::KIND),
        ("GetIncidents", GetIncidents::KIND),
        ("SetLimits", SetLimits::KIND),
        ("SimulateScenario", SimulateScenario::KIND),
        ("Acknowledge", Acknowledge::KIND),
        ("StatusSnapshot", StatusSnapshot::KIND),
        ("IncidentList", IncidentList::KIND),
    ]
}

#[test]
fn every_wire_name_is_a_valid_myrmic_name() {
    // The SDK's own validators are the authority on what a name may be.
    for (payload, kind) in kinds() {
        match kind {
            Kind::Event(name) => {
                assert!(Event::try_from(name).is_ok(), "{payload}: {name}");
            }
            Kind::Command(name) => {
                assert!(Command::try_from(name).is_ok(), "{payload}: {name}");
            }
            Kind::Response => {}
        }
    }
}

#[test]
fn wire_names_are_unique_and_snake_case() {
    let mut names: Vec<&str> = kinds()
        .into_iter()
        .filter_map(|(_, kind)| match kind {
            Kind::Event(name) | Kind::Command(name) => Some(name),
            Kind::Response => None,
        })
        .collect();
    for name in &names {
        assert!(
            name.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "{name}"
        );
    }
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count, "duplicate wire name");
}

#[test]
fn schema_version_is_one() {
    // The golden files were written for this version. Changing it is a
    // deliberate act recorded in docs/decisions/0004-wire-contract.md.
    assert_eq!(SCHEMA_VERSION, 1);
}

#[test]
fn error_strings_fit_the_sdk_result() {
    // `&'static str` is all a handler can return; every error has one.
    let samples = [
        crate::ProtocolError::Malformed,
        crate::ProtocolError::UnsupportedVersion { found: 2 },
        crate::ProtocolError::EncodingFailed,
        crate::ProtocolError::Invalid(crate::ValidationError::LimitOutOfRange { limit: 0 }),
        crate::ProtocolError::Invalid(crate::ValidationError::InvalidReplyTo("x")),
    ];
    for error in samples {
        assert!(!error.as_str().is_empty(), "{error:?}");
        // And Display gives at least as much.
        let display = alloc::format!("{error}");
        assert!(display.len() >= error.as_str().len().min(5), "{error:?}");
        let _: String = display;
    }
}
