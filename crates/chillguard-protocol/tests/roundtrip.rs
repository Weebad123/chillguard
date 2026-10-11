//! Every message encodes, decodes back to an equal value, and does so
//! through the SDK's `Encoder`/`Decoder` as well as through `Envelope`.

// The project standard exempts tests from the unwrap/expect ban (clippy.toml),
// but clippy only recognises `#[test]` functions, not the helpers of an
// integration test file.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

mod common;

use chillguard_protocol::{Envelope, Payload, SCHEMA_VERSION};
use myrmic_sdk::{Decoder, Encoder};

fn round_trip<T: Payload + PartialEq + core::fmt::Debug + Clone>(body: T) {
    let envelope = Envelope::new(body);
    assert_eq!(envelope.v, SCHEMA_VERSION);

    let bytes = envelope.encode().expect("encodes");
    let decoded = Envelope::<T>::decode(&bytes).expect("decodes");
    assert_eq!(decoded, envelope);

    // The SDK traits are what the handler macros and `publish`/`send` use.
    let sdk_bytes = envelope.to_bytes().expect("SDK encodes");
    assert_eq!(sdk_bytes, bytes, "both paths produce the same bytes");
    let sdk_decoded = Envelope::<T>::from_bytes(sdk_bytes).expect("SDK decodes");
    assert_eq!(sdk_decoded, envelope);

    // Encoding is deterministic.
    assert_eq!(decoded.encode().expect("re-encodes"), bytes);
}

#[test]
fn reading_observed() {
    round_trip(common::reading_observed());
}

#[test]
fn reading_accepted() {
    round_trip(common::reading_accepted());
}

#[test]
fn status_snapshot_from_asset() {
    round_trip(common::asset_status_snapshot());
}

#[test]
fn status_changed() {
    round_trip(common::status_changed());
}

#[test]
fn alert_raised() {
    round_trip(common::alert_raised());
}

#[test]
fn incident_recovered() {
    round_trip(common::incident_recovered());
}

#[test]
fn get_status() {
    round_trip(common::get_status());
}

#[test]
fn get_incidents() {
    round_trip(common::get_incidents());
}

#[test]
fn set_limits() {
    round_trip(common::set_limits());
}

#[test]
fn simulate_scenario() {
    round_trip(common::simulate_scenario());
}

#[test]
fn acknowledge() {
    round_trip(common::acknowledge());
}

#[test]
fn status_snapshot() {
    round_trip(common::status_snapshot());
}

#[test]
fn incident_list() {
    round_trip(common::incident_list());
}

#[test]
fn every_payload_is_valid_json_with_the_envelope_shape() {
    for (name, bytes) in common::all_encoded() {
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect(name);
        let object = value.as_object().expect(name);
        assert_eq!(object.len(), 2, "{name}: exactly v and body");
        assert_eq!(object["v"], serde_json::json!(SCHEMA_VERSION), "{name}");
        assert!(object["body"].is_object(), "{name}: body is an object");
        assert!(!bytes.contains(&b'\n'), "{name}: compact, single line");
    }
}
