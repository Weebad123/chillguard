//! Wire-format stability: the exact bytes of every message are pinned in
//! `tests/golden/<name>.json`.
//!
//! A failure here means the wire format changed. If that is intended, bump
//! `SCHEMA_VERSION` when the change breaks an older receiver (see
//! `docs/decisions/0004-wire-contract.md`), then regenerate the files with
//!
//! ```sh
//! UPDATE_GOLDEN=1 cargo test -p chillguard-protocol --test golden
//! ```
//!
//! and review the diff.

// The project standard exempts tests from the unwrap/expect ban (clippy.toml),
// but clippy only recognises `#[test]` functions, not the helpers of an
// integration test file.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use std::fs;
use std::path::PathBuf;

use chillguard_protocol::commands::{
    Acknowledge, GetIncidents, GetStatus, SetLimits, SimulateScenario,
};
use chillguard_protocol::events::{
    AlertRaised, IncidentRecovered, ReadingAccepted, ReadingObserved, StatusChanged,
};
use chillguard_protocol::responses::{IncidentList, StatusSnapshot};
use chillguard_protocol::{Envelope, Payload};

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn golden_path(name: &str) -> PathBuf {
    golden_dir().join(format!("{name}.json"))
}

/// The pretty-printed form of the compact wire bytes. The files are stored
/// pretty so that a diff is readable; the test compares the compact bytes
/// the files parse back to, which is what travels.
fn pretty(bytes: &[u8]) -> String {
    let value: serde_json::Value = serde_json::from_slice(bytes).expect("valid JSON");
    let mut text = serde_json::to_string_pretty(&value).expect("serialises");
    text.push('\n');
    text
}

fn compact(text: &str) -> Vec<u8> {
    let value: serde_json::Value = serde_json::from_str(text).expect("golden file is valid JSON");
    serde_json::to_vec(&value).expect("serialises")
}

#[test]
fn encoded_bytes_match_the_golden_files() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut mismatches = Vec::new();

    for (name, bytes) in common::all_encoded() {
        let path = golden_path(name);
        let expected_pretty = pretty(&bytes);
        if update {
            fs::write(&path, &expected_pretty).expect("writes golden file");
            continue;
        }
        let Ok(stored) = fs::read_to_string(&path) else {
            mismatches.push(format!("{name}: missing {}", path.display()));
            continue;
        };
        // Exact bytes: field order, number formatting, and compactness all
        // count. Pretty-printing preserves order, so the compact form of the
        // stored file is the wire form.
        if compact(&stored) != bytes {
            mismatches.push(format!(
                "{name}: wire bytes differ from {}\n--- stored\n{stored}--- produced\n{expected_pretty}",
                path.display()
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "wire format changed:\n{}",
        mismatches.join("\n")
    );
}

#[test]
fn golden_files_decode_to_the_fixtures() {
    fn check<T: Payload + PartialEq + core::fmt::Debug>(name: &str, expected: T) {
        let stored = fs::read_to_string(golden_path(name)).expect("golden file exists");
        let decoded = Envelope::<T>::decode(&compact(&stored)).expect("golden file decodes");
        assert_eq!(decoded, Envelope::new(expected), "{name}");
    }
    check("reading_observed", common::reading_observed());
    check("reading_accepted", common::reading_accepted());
    check("status_changed", common::status_changed());
    check("alert_raised", common::alert_raised());
    check("incident_recovered", common::incident_recovered());
    check("get_status", common::get_status());
    check("get_incidents", common::get_incidents());
    check("set_limits", common::set_limits());
    check("simulate_scenario", common::simulate_scenario());
    check("acknowledge", common::acknowledge());
    check("status_snapshot", common::status_snapshot());
    check(
        "status_snapshot_from_asset",
        common::asset_status_snapshot(),
    );
    check("incident_list", common::incident_list());
}

#[test]
fn every_golden_file_has_a_fixture_and_every_fixture_a_file() {
    let fixtures: std::collections::BTreeSet<String> = common::all_encoded()
        .into_iter()
        .map(|(name, _)| name.to_string())
        .collect();
    let files: std::collections::BTreeSet<String> = fs::read_dir(golden_dir())
        .expect("golden dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            path.file_stem()
                .expect("stem")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(files, fixtures);
}

/// The preset names are typed by hand into `myrmic send`, and they are the
/// names of core's scenario constructors.
#[test]
fn preset_names_are_pinned() {
    use chillguard_protocol::commands::ScenarioPreset;
    assert_eq!(ScenarioPreset::ALL.len(), 3);
    for (preset, name) in [
        (ScenarioPreset::NormalDay, "normal_day"),
        (ScenarioPreset::PowerFailure2h, "power_failure_2h"),
        (ScenarioPreset::ShortDoorOpen, "short_door_open"),
    ] {
        assert!(ScenarioPreset::ALL.contains(&preset));
        assert_eq!(preset.as_str(), name);
        assert_eq!(preset.to_string(), name);
        assert_eq!(
            serde_json::to_value(preset).unwrap(),
            serde_json::json!(name)
        );
        let back: ScenarioPreset = serde_json::from_value(serde_json::json!(name)).unwrap();
        assert_eq!(back, preset);
    }
    // What serde's `snake_case` alone would have produced.
    assert!(
        serde_json::from_value::<ScenarioPreset>(serde_json::json!("power_failure2h")).is_err()
    );
}

/// The wire names, pinned: a cell subscribes and sends by these strings.
#[test]
fn wire_names_are_pinned() {
    use chillguard_protocol::Kind;
    assert_eq!(ReadingObserved::KIND, Kind::Event("reading_observed"));
    assert_eq!(ReadingAccepted::KIND, Kind::Event("reading_accepted"));
    assert_eq!(StatusChanged::KIND, Kind::Event("status_changed"));
    assert_eq!(AlertRaised::KIND, Kind::Event("alert_raised"));
    assert_eq!(IncidentRecovered::KIND, Kind::Event("incident_recovered"));
    assert_eq!(GetStatus::KIND, Kind::Command("get_status"));
    assert_eq!(GetIncidents::KIND, Kind::Command("get_incidents"));
    assert_eq!(SetLimits::KIND, Kind::Command("set_limits"));
    assert_eq!(SimulateScenario::KIND, Kind::Command("simulate_scenario"));
    assert_eq!(Acknowledge::KIND, Kind::Command("acknowledge"));
    assert_eq!(StatusSnapshot::KIND, Kind::Response);
    assert_eq!(IncidentList::KIND, Kind::Response);
    const { assert!(SimulateScenario::DEV_ONLY) };
    const { assert!(!SetLimits::DEV_ONLY && !Acknowledge::DEV_ONLY && !GetStatus::DEV_ONLY) };
}
