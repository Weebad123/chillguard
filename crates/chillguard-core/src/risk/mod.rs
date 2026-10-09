//! Risk evaluation: a pure, deterministic state machine.
//!
//! [`evaluate`] takes the monitor's current state, one [`Input`], and the
//! refrigerator's configuration, and returns the next state together with the
//! [`DomainEvent`]s to emit. It reads no clock and performs no I/O: time
//! arrives in the reading's timestamp or in a [`Input::Tick`].
//!
//! ```text
//!                 out-of-range reading          excursion lasted >= critical_after
//!   Normal ───────────────────────────► Warning ───────────────────────────► Critical
//!     ▲                                    │                                    │
//!     └────────────────────────────────────┴────── in-range reading ───────────┘
//!
//!   any status ── Tick more than stale_after after the last reading ──► SensorStale
//!   SensorStale ── next accepted reading ──► Normal | Warning | Critical
//! ```
//!
//! Events of one call are ordered: [`DomainEvent::StatusChanged`] first when
//! the status changed, then [`DomainEvent::SensorStale`] or
//! [`DomainEvent::SensorResumed`], then the alert events in the order
//! warning, critical, recovered.
//!
//! The policies behind the edge cases (ordering, clock, gaps, limits,
//! hysteresis) are recorded in `docs/decisions/0003-risk-policies.md`.

mod config;
mod error;
mod evaluate;
mod event;
mod incident;
mod model;
mod state;

#[cfg(test)]
mod tests;

pub use config::RefrigeratorConfig;
pub use error::RiskError;
pub use evaluate::evaluate;
pub use event::{DomainEvent, RejectReason};
pub use incident::{Incident, IncidentId, Severity};
pub use model::{Input, ReadingSource, RefrigeratorId, TemperatureReading, Timestamp};
pub use state::{LastReading, MonitorState, Status};
