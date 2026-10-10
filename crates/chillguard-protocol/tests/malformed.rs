//! Input that must be refused: wrong version, wrong shape, wrong content.

// The project standard exempts tests from the unwrap/expect ban (clippy.toml),
// but clippy only recognises `#[test]` functions, not the helpers of an
// integration test file.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use chillguard_core::risk::{RiskError, Status};
use chillguard_protocol::commands::{
    Acknowledge, GetIncidents, GetStatus, MAX_INCIDENT_LIMIT, ReplyTo, SetLimits, SimulateScenario,
};
use chillguard_protocol::events::{AlertRaised, IncidentRecovered, ReadingObserved, StatusChanged};
use chillguard_protocol::responses::{IncidentList, StatusSnapshot};
use chillguard_protocol::{Envelope, Payload, ProtocolError, SCHEMA_VERSION, ValidationError};
use myrmic_sdk::Decoder;
use serde_json::{Value, json};

/// Decodes a JSON value as an envelope of `T`.
fn decode<T: Payload>(value: &Value) -> Result<Envelope<T>, ProtocolError> {
    Envelope::<T>::decode(&serde_json::to_vec(value).expect("serialises"))
}

/// The fixture for `T` as a JSON value, for tests to break one piece of.
fn json_of<T: Payload>(body: T) -> Value {
    let bytes = Envelope::new(body).encode().expect("fixture encodes");
    serde_json::from_slice(&bytes).expect("valid JSON")
}

fn with<T: Payload>(body: T, edit: impl FnOnce(&mut Value)) -> Result<Envelope<T>, ProtocolError> {
    let mut value = json_of(body);
    edit(&mut value);
    decode::<T>(&value)
}

// ---------------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------------

#[test]
fn not_json_is_malformed() {
    for bytes in [
        &b""[..],
        b"\xff\xfe",
        b"not json",
        b"{",
        b"[]",
        b"42",
        b"\"text\"",
        b"null",
    ] {
        assert_eq!(
            Envelope::<GetStatus>::decode(bytes),
            Err(ProtocolError::Malformed),
            "{bytes:?}"
        );
    }
}

#[test]
fn envelope_without_version_is_malformed() {
    assert_eq!(
        decode::<GetStatus>(&json!({"body": {"reply_to": "x"}})),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        decode::<GetStatus>(&json!({"version": 1, "body": {"reply_to": "x"}})),
        Err(ProtocolError::Malformed)
    );
}

#[test]
fn other_schema_versions_are_refused_before_the_body_is_read() {
    for found in [0, 2, 7, u16::MAX] {
        // The body is deliberately unreadable: the version check comes first.
        let value = json!({"v": found, "body": "not a body"});
        assert_eq!(
            decode::<GetStatus>(&value),
            Err(ProtocolError::UnsupportedVersion { found })
        );
    }
    // A version that is not a u16 is a malformed envelope.
    for bad in [
        json!(-1),
        json!(70_000),
        json!("1"),
        json!(1.5),
        json!(null),
    ] {
        assert_eq!(
            decode::<GetStatus>(&json!({"v": bad, "body": {"reply_to": "x"}})),
            Err(ProtocolError::Malformed),
            "{bad}"
        );
    }
}

#[test]
fn envelope_without_body_is_malformed() {
    assert_eq!(
        decode::<GetStatus>(&json!({"v": SCHEMA_VERSION})),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        decode::<GetStatus>(&json!({"v": SCHEMA_VERSION, "body": null})),
        Err(ProtocolError::Malformed)
    );
}

#[test]
fn unknown_fields_are_ignored_for_forward_compatibility() {
    let result = with(common::get_status(), |v| {
        v["extra"] = json!("ignored");
        v["body"]["added_later"] = json!({"nested": true});
    });
    assert_eq!(result, Ok(Envelope::new(common::get_status())));
}

#[test]
fn the_sdk_decoder_reports_the_same_refusals_as_static_strings() {
    let err = Envelope::<GetStatus>::from_bytes(b"{".to_vec()).expect_err("malformed");
    assert_eq!(err, ProtocolError::Malformed.as_str());

    let bytes = serde_json::to_vec(&json!({"v": 9, "body": {}})).expect("serialises");
    let err = Envelope::<GetStatus>::from_bytes(bytes).expect_err("version");
    assert_eq!(err, ProtocolError::UnsupportedVersion { found: 9 }.as_str());

    let bytes = serde_json::to_vec(&json!({"v": 1, "body": {"reply_to": "has space"}}))
        .expect("serialises");
    let err = Envelope::<GetStatus>::from_bytes(bytes).expect_err("invalid");
    assert_eq!(err, "name cannot contain whitespace");
}

// ---------------------------------------------------------------------------
// Shape: a field of the wrong type, or a required field missing
// ---------------------------------------------------------------------------

#[test]
fn wrong_field_types_are_malformed() {
    assert_eq!(
        with(
            common::reading_observed(),
            |v| v["body"]["reading"]["temperature_c"] = json!("9")
        ),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(
            common::reading_observed(),
            |v| v["body"]["reading"]["observed_at"] = json!(-1)
        ),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(
            common::reading_observed(),
            |v| v["body"]["reading"]["source"] = json!("guess")
        ),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(common::status_changed(), |v| v["body"]["to"] =
            json!("Warning")),
        Err(ProtocolError::Malformed),
        "status names are snake_case"
    );
    assert_eq!(
        with(common::get_incidents(), |v| v["body"]["limit"] =
            json!(10.5)),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(common::get_incidents(), |v| v["body"]["limit"] =
            json!(70_000)),
        Err(ProtocolError::Malformed),
        "limit is a u16"
    );
    assert_eq!(
        with(common::simulate_scenario(), |v| v["body"]["preset"] =
            json!("power_failure")),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(common::simulate_scenario(), |v| v["body"]["seed"] =
            json!(-1)),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(common::incident_list(), |v| v["body"]["incidents"] =
            json!({})),
        Err(ProtocolError::Malformed)
    );
    // Durations are unsigned integers of milliseconds, nothing else.
    for bad in [
        json!({"secs": 1800, "nanos": 0}),
        json!(1_800_000.5),
        json!(-1),
        json!("1800000"),
        json!(null),
    ] {
        assert_eq!(
            with(common::alert_raised(), |v| {
                v["body"]["incident"]["exposure"] = bad.clone();
            }),
            Err(ProtocolError::Malformed),
            "exposure = {bad}"
        );
        assert_eq!(
            with(common::set_limits(), |v| {
                v["body"]["config"]["critical_after"] = bad.clone();
            }),
            Err(ProtocolError::Malformed),
            "critical_after = {bad}"
        );
    }
}

#[test]
fn missing_required_fields_are_malformed() {
    fn remove<'a>(path: &'a [&'a str]) -> impl Fn(&mut Value) + 'a {
        move |v| {
            let mut node = &mut *v;
            for key in &path[..path.len() - 1] {
                node = &mut node[*key];
            }
            node.as_object_mut()
                .expect("object")
                .remove(path[path.len() - 1]);
        }
    }
    assert_eq!(
        with(common::reading_observed(), remove(&["body", "reading"])),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(
            common::reading_observed(),
            remove(&["body", "reading", "refrigerator_id"])
        ),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(common::status_changed(), remove(&["body", "at"])),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(common::alert_raised(), remove(&["body", "severity"])),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(
            common::alert_raised(),
            remove(&["body", "incident", "started_at"])
        ),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(common::get_status(), remove(&["body", "reply_to"])),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(common::get_incidents(), remove(&["body", "limit"])),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(
            common::set_limits(),
            remove(&["body", "config", "upper_limit_c"])
        ),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(common::acknowledge(), remove(&["body", "incident_id"])),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        with(common::status_snapshot(), remove(&["body", "config"])),
        Err(ProtocolError::Malformed)
    );
}

#[test]
fn optional_fields_may_be_absent() {
    // `seed` defaults to zero.
    let decoded = with(common::simulate_scenario(), |v| {
        v["body"].as_object_mut().expect("object").remove("seed");
    })
    .expect("decodes");
    assert_eq!(decoded.body.seed, 0);

    // `lower_limit_c` and `hysteresis_c` are optional in core's config.
    let decoded = with(common::set_limits(), |v| {
        let config = v["body"]["config"].as_object_mut().expect("object");
        config.remove("lower_limit_c");
        config.remove("hysteresis_c");
    })
    .expect("decodes");
    assert_eq!(decoded.body.config.lower_limit_c, None);
    assert!(decoded.body.config.hysteresis_c.abs() < f64::EPSILON);

    // A snapshot of a monitor that has seen nothing.
    let decoded = with(common::status_snapshot(), |v| {
        v["body"]["last_reading"] = json!(null);
        v["body"]["open_incident"] = json!(null);
        v["body"]["status"] = json!("normal");
    })
    .expect("decodes");
    assert_eq!(decoded.body.last_reading, None);
    assert_eq!(decoded.body.open_incident, None);
}

// ---------------------------------------------------------------------------
// Content: the shape is right, the values are not
// ---------------------------------------------------------------------------

fn invalid(error: ValidationError) -> Result<Envelope<GetStatus>, ProtocolError> {
    Err(ProtocolError::Invalid(error))
}

#[test]
fn empty_identifiers_are_invalid() {
    assert_eq!(
        with(
            common::reading_observed(),
            |v| v["body"]["reading"]["refrigerator_id"] = json!("")
        ),
        Err(ProtocolError::Invalid(ValidationError::EmptyRefrigeratorId))
    );
    assert_eq!(
        with(common::status_changed(), |v| v["body"]["refrigerator_id"] =
            json!("")),
        Err(ProtocolError::Invalid(ValidationError::EmptyRefrigeratorId))
    );
    assert_eq!(
        with(common::acknowledge(), |v| v["body"]["incident_id"] =
            json!("")),
        Err(ProtocolError::Invalid(ValidationError::EmptyIncidentId))
    );
    assert_eq!(
        with(
            common::alert_raised(),
            |v| v["body"]["incident"]["incident_id"] = json!("")
        ),
        Err(ProtocolError::Invalid(ValidationError::EmptyIncidentId))
    );
    assert_eq!(
        with(
            common::set_limits(),
            |v| v["body"]["config"]["refrigerator_id"] = json!("")
        ),
        Err(ProtocolError::Invalid(ValidationError::EmptyRefrigeratorId))
    );
    assert_eq!(
        with(common::incident_list(), |v| v["body"]["refrigerator_id"] =
            json!("")),
        Err(ProtocolError::Invalid(ValidationError::EmptyRefrigeratorId))
    );
}

#[test]
fn non_finite_temperatures_cannot_be_sent_and_arrive_as_malformed() {
    // JSON has no NaN or infinity. Encoding refuses them before the codec
    // would write `null`...
    let mut body = common::reading_observed();
    body.reading.temperature_c = f64::NAN;
    assert_eq!(
        Envelope::new(body).encode(),
        Err(ProtocolError::Invalid(
            ValidationError::NonFiniteTemperature
        ))
    );
    let mut body = common::incident_recovered();
    body.temperature_c = f64::INFINITY;
    assert_eq!(
        Envelope::new(body).encode(),
        Err(ProtocolError::Invalid(
            ValidationError::NonFiniteTemperature
        ))
    );
    // ...and a `null` where a number belongs fails the shape check.
    assert_eq!(
        with(
            common::reading_observed(),
            |v| v["body"]["reading"]["temperature_c"] = json!(null)
        ),
        Err(ProtocolError::Malformed)
    );
}

#[test]
fn accepted_reading_must_agree_with_its_config() {
    assert_eq!(
        with(common::reading_accepted(), |v| {
            v["body"]["config"]["refrigerator_id"] = json!("fridge-2");
        }),
        Err(ProtocolError::Invalid(
            ValidationError::ReadingConfigMismatch
        ))
    );
    assert_eq!(
        with(common::reading_accepted(), |v| {
            v["body"]["config"]["lower_limit_c"] = json!(9.0);
        }),
        Err(ProtocolError::Invalid(ValidationError::InvalidLimits(
            RiskError::LimitsInverted {
                lower: 9.0,
                upper: 8.0
            }
        )))
    );
    assert_eq!(
        with(common::reading_accepted(), |v| {
            v["body"].as_object_mut().unwrap().remove("config");
        }),
        Err(ProtocolError::Malformed)
    );
    // A bare reading_observed is not a reading_accepted.
    let bytes = Envelope::new(common::reading_observed())
        .encode()
        .expect("encodes");
    assert_eq!(
        Envelope::<chillguard_protocol::events::ReadingAccepted>::decode(&bytes),
        Err(ProtocolError::Malformed)
    );
}

#[test]
fn status_change_must_change_the_status() {
    for status in ["normal", "warning", "critical", "sensor_stale"] {
        assert_eq!(
            with(common::status_changed(), |v| {
                v["body"]["from"] = json!(status);
                v["body"]["to"] = json!(status);
            }),
            Err(ProtocolError::Invalid(ValidationError::StatusUnchanged)),
            "{status}"
        );
    }
    let mut body = common::status_changed();
    body.from = Status::Warning;
    body.to = Status::Warning;
    assert_eq!(
        Envelope::new(body).encode(),
        Err(ProtocolError::Invalid(ValidationError::StatusUnchanged))
    );
}

#[test]
fn alert_must_match_its_incident() {
    assert_eq!(
        with(common::alert_raised(), |v| v["body"]["severity"] =
            json!("warning")),
        Err(ProtocolError::Invalid(
            ValidationError::AlertSeverityMismatch
        ))
    );
    assert_eq!(
        with(common::alert_raised(), |v| {
            v["body"]["incident"]["recovered_at"] = json!(common::at(50).as_millis());
            v["body"]["incident"]["recovery_temperature_c"] = json!(5.0);
        }),
        Err(ProtocolError::Invalid(
            ValidationError::AlertForClosedIncident
        ))
    );
}

#[test]
fn recovery_must_match_its_incident() {
    assert_eq!(
        with(common::incident_recovered(), |v| v["body"]["at"] =
            json!(common::at(56).as_millis())),
        Err(ProtocolError::Invalid(ValidationError::RecoveryMismatch))
    );
    assert_eq!(
        with(
            common::incident_recovered(),
            |v| v["body"]["temperature_c"] = json!(5.5)
        ),
        Err(ProtocolError::Invalid(ValidationError::RecoveryMismatch))
    );
    assert_eq!(
        with(common::incident_recovered(), |v| {
            v["body"]["incident"]["recovered_at"] = json!(null);
            v["body"]["incident"]["recovery_temperature_c"] = json!(null);
        }),
        Err(ProtocolError::Invalid(ValidationError::RecoveryMismatch))
    );
}

#[test]
fn incident_records_must_be_consistent() {
    assert_eq!(
        with(
            common::alert_raised(),
            |v| v["body"]["incident"]["lowest_temperature_c"] = json!(20.0)
        ),
        Err(ProtocolError::Invalid(
            ValidationError::InconsistentIncidentTemperatures
        ))
    );
    assert_eq!(
        with(
            common::incident_recovered(),
            |v| v["body"]["incident"]["recovery_temperature_c"] = json!(null)
        ),
        Err(ProtocolError::Invalid(
            ValidationError::PartialIncidentRecovery
        ))
    );
    assert_eq!(
        with(common::incident_recovered(), |v| {
            let before_start = common::at(5).as_millis();
            v["body"]["incident"]["recovered_at"] = json!(before_start);
            v["body"]["at"] = json!(before_start);
        }),
        Err(ProtocolError::Invalid(
            ValidationError::IncidentRecoveredBeforeStart
        ))
    );
    // The same checks guard the incidents inside a list and a snapshot.
    assert_eq!(
        with(
            common::incident_list(),
            |v| v["body"]["incidents"][1]["highest_temperature_c"] = json!(-50.0)
        ),
        Err(ProtocolError::Invalid(
            ValidationError::InconsistentIncidentTemperatures
        ))
    );
    assert_eq!(
        with(common::status_snapshot(), |v| v["body"]["open_incident"]
            ["incident_id"] =
            json!("")),
        Err(ProtocolError::Invalid(ValidationError::EmptyIncidentId))
    );
}

#[test]
fn reply_to_must_be_a_valid_command_name() {
    for (name, reason) in [
        ("", "name cannot be empty"),
        ("has space", "name cannot contain whitespace"),
        (
            "on-status",
            "name can only contain ASCII alphanumeric characters and underscores",
        ),
        (
            "on.status",
            "name can only contain ASCII alphanumeric characters and underscores",
        ),
        (
            "état",
            "name can only contain ASCII alphanumeric characters and underscores",
        ),
    ] {
        assert_eq!(
            ReplyTo::new(name),
            Err(ValidationError::InvalidReplyTo(reason)),
            "{name:?}"
        );
        assert_eq!(
            with(common::get_status(), |v| v["body"]["reply_to"] =
                json!(name)),
            invalid(ValidationError::InvalidReplyTo(reason)),
            "{name:?}"
        );
        assert_eq!(
            with(common::get_incidents(), |v| v["body"]["reply_to"] =
                json!(name)),
            Err(ProtocolError::Invalid(ValidationError::InvalidReplyTo(
                reason
            ))),
            "{name:?}"
        );
    }
    for name in ["on_status", "OnStatus", "reply2", "_", "7up"] {
        assert!(ReplyTo::new(name).is_ok(), "{name:?}");
    }
}

#[test]
fn incident_page_size_must_be_in_range() {
    for limit in [0, MAX_INCIDENT_LIMIT + 1, u16::MAX] {
        assert_eq!(
            with(common::get_incidents(), |v| v["body"]["limit"] =
                json!(limit)),
            Err(ProtocolError::Invalid(ValidationError::LimitOutOfRange {
                limit
            })),
            "{limit}"
        );
    }
    for limit in [1, MAX_INCIDENT_LIMIT] {
        assert!(
            with(common::get_incidents(), |v| v["body"]["limit"] =
                json!(limit))
            .is_ok(),
            "{limit}"
        );
    }
}

#[test]
fn limits_are_validated_by_core() {
    assert_eq!(
        with(
            common::set_limits(),
            |v| v["body"]["config"]["lower_limit_c"] = json!(9.0)
        ),
        Err(ProtocolError::Invalid(ValidationError::InvalidLimits(
            RiskError::LimitsInverted {
                lower: 9.0,
                upper: 8.0
            }
        )))
    );
    assert_eq!(
        with(
            common::set_limits(),
            |v| v["body"]["config"]["hysteresis_c"] = json!(-1.0)
        ),
        Err(ProtocolError::Invalid(ValidationError::InvalidLimits(
            RiskError::InvalidHysteresis
        )))
    );
    assert_eq!(
        with(
            common::set_limits(),
            |v| v["body"]["config"]["stale_after"] = json!(0)
        ),
        Err(ProtocolError::Invalid(ValidationError::InvalidLimits(
            RiskError::ZeroStaleAfter
        )))
    );
    // The snapshot's config is held to the same standard.
    assert_eq!(
        with(
            common::status_snapshot(),
            |v| v["body"]["config"]["upper_limit_c"] = json!(-100.0)
        ),
        Err(ProtocolError::Invalid(ValidationError::InvalidLimits(
            RiskError::LimitsInverted {
                lower: 2.0,
                upper: -100.0
            }
        )))
    );
}

#[test]
fn a_payload_decoded_as_the_wrong_type_is_malformed() {
    // Sending a command's body to an event handler, and the reverse.
    let bytes = Envelope::new(common::get_status())
        .encode()
        .expect("encodes");
    assert_eq!(
        Envelope::<ReadingObserved>::decode(&bytes),
        Err(ProtocolError::Malformed)
    );
    let bytes = Envelope::new(common::alert_raised())
        .encode()
        .expect("encodes");
    assert_eq!(
        Envelope::<IncidentRecovered>::decode(&bytes),
        Err(ProtocolError::Malformed)
    );
    let bytes = Envelope::new(common::status_snapshot())
        .encode()
        .expect("encodes");
    assert_eq!(
        Envelope::<IncidentList>::decode(&bytes),
        Err(ProtocolError::Malformed)
    );
    let bytes = Envelope::new(common::acknowledge())
        .encode()
        .expect("encodes");
    assert_eq!(
        Envelope::<SetLimits>::decode(&bytes),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        Envelope::<GetIncidents>::decode(&bytes),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        Envelope::<SimulateScenario>::decode(&bytes),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        Envelope::<StatusChanged>::decode(&bytes),
        Err(ProtocolError::Malformed)
    );
    assert!(Envelope::<Acknowledge>::decode(&bytes).is_ok());
    assert_eq!(
        Envelope::<AlertRaised>::decode(&bytes),
        Err(ProtocolError::Malformed)
    );
    assert_eq!(
        Envelope::<StatusSnapshot>::decode(&bytes),
        Err(ProtocolError::Malformed)
    );
}
