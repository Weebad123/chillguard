//! What the state machine tells the outside world.

use serde::{Deserialize, Serialize};

use super::incident::Incident;
use super::model::{RefrigeratorId, Timestamp};
use super::state::Status;

/// Something the caller should publish, log, or store.
///
/// Events that carry an [`Incident`] carry a snapshot taken at that moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainEvent {
    /// The status changed.
    StatusChanged {
        /// The status before the input.
        from: Status,
        /// The status after the input.
        to: Status,
        /// The reading's timestamp, or the tick's time for a stale sensor.
        at: Timestamp,
    },
    /// An incident opened. Emitted once per incident.
    WarningRaised {
        /// The incident as it opened.
        incident: Incident,
    },
    /// An incident became critical. Emitted at most once per incident.
    CriticalRaised {
        /// The incident as it became critical.
        incident: Incident,
    },
    /// A reading ended an incident. The incident keeps the highest severity
    /// it reached. This records an observation, not that contents are safe.
    Recovered {
        /// The closed incident, for the caller to keep as history.
        incident: Incident,
        /// Timestamp of the recovering reading.
        at: Timestamp,
        /// Temperature of the recovering reading, in degrees Celsius.
        temperature: f64,
    },
    /// The sensor has been silent for more than `stale_after`.
    SensorStale {
        /// When the silence began: the last accepted reading, or the first
        /// tick if no reading ever arrived.
        since: Timestamp,
    },
    /// A reading arrived from a sensor that was stale.
    SensorResumed {
        /// Timestamp of that reading.
        at: Timestamp,
    },
    /// A reading was ignored. The state did not change.
    ReadingRejected {
        /// Why it was ignored.
        reason: RejectReason,
    },
}

/// Why a reading was ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    /// The reading belongs to another refrigerator.
    WrongRefrigerator {
        /// The refrigerator this monitor is configured for.
        expected: RefrigeratorId,
        /// The refrigerator named in the reading.
        actual: RefrigeratorId,
    },
    /// The temperature is NaN or infinite.
    NonFiniteTemperature,
    /// The reading is older than the last accepted one: late delivery, or a
    /// sensor clock that went backwards.
    OutOfOrder {
        /// The rejected reading's timestamp.
        observed_at: Timestamp,
        /// Timestamp of the last accepted reading.
        last_reading_at: Timestamp,
    },
    /// The reading carries the same timestamp as the last accepted one.
    Duplicate {
        /// The shared timestamp.
        observed_at: Timestamp,
    },
}

impl RejectReason {
    /// A short name for the reason, as it appears on the wire and in logs.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::WrongRefrigerator { .. } => "wrong_refrigerator",
            Self::NonFiniteTemperature => "non_finite_temperature",
            Self::OutOfOrder { .. } => "out_of_order",
            Self::Duplicate { .. } => "duplicate",
        }
    }
}
