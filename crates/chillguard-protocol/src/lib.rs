//! Message, event, and command types shared by the ChillGuard cells.
//!
//! Every payload that crosses a cell boundary is defined here, together with
//! its wire encoding, so that the sender and the receiver cannot disagree on
//! a shape. The types carry data only; business rules live in
//! `chillguard-core`, and the domain types defined there ([`Incident`],
//! [`TemperatureReading`], [`Status`], ...) are used as they are.
//!
//! # Shape of the contract
//!
//! Every message travels inside an [`Envelope`]: `{"v": 1, "body": {...}}`.
//! The envelope carries the [`SCHEMA_VERSION`]; the body is one of the
//! payload types below, each of which implements [`Payload`].
//!
//! | Kind | Type | Wire name |
//! |---|---|---|
//! | event | [`events::ReadingObserved`] | `reading_observed` |
//! | event | [`events::StatusChanged`] | `status_changed` |
//! | event | [`events::AlertRaised`] | `alert_raised` |
//! | event | [`events::IncidentRecovered`] | `incident_recovered` |
//! | command | [`commands::GetStatus`] | `get_status` |
//! | command | [`commands::GetIncidents`] | `get_incidents` |
//! | command | [`commands::SetLimits`] | `set_limits` |
//! | command | [`commands::SimulateScenario`] | `simulate_scenario` |
//! | command | [`commands::Acknowledge`] | `acknowledge` |
//! | response | [`responses::StatusSnapshot`] | named by the caller |
//! | response | [`responses::IncidentList`] | named by the caller |
//!
//! Events are published for anyone to observe (`myrmic_sdk::publish`).
//! Commands are sent to one cell (`myrmic_sdk::send`) and are redelivered
//! until the handler succeeds. A query command carries a [`commands::ReplyTo`]
//! naming the command on the caller that receives the response.
//!
//! Encoding is JSON through the SDK's codec, the same one that
//! `#[derive(myrmic_sdk::Message)]` binds. Decoding checks the schema
//! version, then the shape, then the content ([`Payload::validate`]); see
//! `docs/decisions/0004-wire-contract.md` for the compatibility policy.
//!
//! This crate depends on `myrmic-sdk` because the SDK's `Encoder`/`Decoder`
//! traits must be implemented in the crate that defines a payload type; see
//! `docs/decisions/0001-core-vs-cells.md`.

#![no_std]

extern crate alloc;

pub mod commands;
mod envelope;
pub mod events;
pub mod responses;
mod validate;

#[cfg(test)]
mod tests;

pub use envelope::{Envelope, Kind, Payload, ProtocolError, SCHEMA_VERSION};
pub use validate::ValidationError;

// The core types that appear in payloads, re-exported so that a cell reading
// this contract finds every type it names in one place.
pub use chillguard_core::risk::{
    Incident, IncidentId, LastReading, ReadingSource, RefrigeratorConfig, RefrigeratorId, Severity,
    Status, TemperatureReading, Timestamp,
};
