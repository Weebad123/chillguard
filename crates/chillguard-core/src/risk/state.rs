//! The monitor's state between two inputs.

use serde::{Deserialize, Serialize};

use super::incident::{Incident, Severity};
use super::model::Timestamp;

/// The status shown for a refrigerator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The latest reading is in range and no incident is open.
    Normal,
    /// An incident is open and has not become critical.
    Warning,
    /// An incident is open and has become critical.
    Critical,
    /// No reading arrived within `stale_after`. An incident may still be open.
    SensorStale,
}

impl From<Severity> for Status {
    fn from(severity: Severity) -> Self {
        match severity {
            Severity::Warning => Self::Warning,
            Severity::Critical => Self::Critical,
        }
    }
}

/// The most recent reading the monitor accepted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LastReading {
    /// Temperature in degrees Celsius.
    pub temperature_c: f64,
    /// When it was observed.
    pub observed_at: Timestamp,
}

/// Everything the risk rules remember about one refrigerator.
///
/// Only [`evaluate`](super::evaluate) produces new states. The state is
/// small and bounded (closed incidents leave it in a `Recovered` event), and
/// it serialises with any serde format, so a caller can persist it and carry
/// on after a restart.
///
/// The status is derived, not stored: a state cannot claim `Warning` without
/// an open incident.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MonitorState {
    pub(super) last_reading: Option<LastReading>,
    pub(super) open_incident: Option<Incident>,
    pub(super) stale: bool,
    /// When the first tick arrived, while no reading has been accepted yet.
    /// Stale detection counts from here until the first reading.
    pub(super) waiting_since: Option<Timestamp>,
}

impl MonitorState {
    /// The state of a monitor that has seen nothing yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_reading: None,
            open_incident: None,
            stale: false,
            waiting_since: None,
        }
    }

    /// The current status.
    #[must_use]
    pub fn status(&self) -> Status {
        if self.stale {
            return Status::SensorStale;
        }
        self.open_incident
            .as_ref()
            .map_or(Status::Normal, |incident| incident.severity.into())
    }

    /// The most recent accepted reading, if any.
    #[must_use]
    pub const fn last_reading(&self) -> Option<LastReading> {
        self.last_reading
    }

    /// The incident in progress, if any. There is never more than one.
    #[must_use]
    pub const fn open_incident(&self) -> Option<&Incident> {
        self.open_incident.as_ref()
    }

    /// The time stale detection counts from: the last accepted reading, or
    /// the first tick while no reading has arrived.
    pub(super) fn silent_since(&self) -> Option<Timestamp> {
        self.last_reading
            .map(|last| last.observed_at)
            .or(self.waiting_since)
    }
}
