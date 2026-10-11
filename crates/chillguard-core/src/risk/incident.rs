//! Incidents: one continuous temperature excursion each.

use alloc::format;
use alloc::string::String;
use core::fmt;
use core::time::Duration;

use serde::{Deserialize, Serialize};

use super::model::{RefrigeratorId, TemperatureReading, Timestamp};

/// Identifies one incident.
///
/// Derived from the refrigerator and the incident's start time, so replaying
/// the same readings yields the same identifier and a monitor that lost its
/// state cannot reuse an old one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IncidentId(String);

impl IncidentId {
    /// Wraps an identifier that came from outside: an operator naming the
    /// incident to acknowledge. The state machine never calls this; it
    /// derives the identifiers of the incidents it opens.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    fn derive(refrigerator_id: &RefrigeratorId, started_at: Timestamp) -> Self {
        Self(format!("{refrigerator_id}-{}", started_at.as_millis()))
    }

    /// The identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IncidentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How serious an incident became. Ordered: warning is below critical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The temperature left the safe range.
    Warning,
    /// The excursion lasted at least the configured critical duration.
    Critical,
}

impl Severity {
    /// The severity's name as it appears on the wire and in logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The record of one excursion, from the first out-of-range reading to the
/// reading that ended it.
///
/// An incident records what was observed. A recovered incident does not mean
/// the refrigerator's contents are safe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Incident {
    /// Identifier, unique per refrigerator and start time.
    pub incident_id: IncidentId,
    /// The refrigerator the incident belongs to.
    pub refrigerator_id: RefrigeratorId,
    /// Timestamp of the first out-of-range reading.
    pub started_at: Timestamp,
    /// Temperature of the first out-of-range reading, in degrees Celsius.
    pub initial_temperature_c: f64,
    /// Highest temperature observed during the excursion.
    pub highest_temperature_c: f64,
    /// Lowest temperature observed during the excursion.
    pub lowest_temperature_c: f64,
    /// The highest severity the incident reached. It never goes back down.
    pub severity: Severity,
    /// Time from the first to the latest reading that kept the incident open.
    /// Silence between two such readings counts in full; the time between the
    /// last of them and the recovering reading does not. Serialised as whole
    /// milliseconds.
    #[serde(with = "crate::duration_millis")]
    pub exposure: Duration,
    /// Whether two consecutive readings of the incident, the recovering one
    /// included, were more than `stale_after` apart. When set, `exposure`
    /// spans time for which there is no data.
    pub has_data_gap: bool,
    /// Timestamp of the reading that ended the excursion; `None` while open.
    pub recovered_at: Option<Timestamp>,
    /// Temperature of the reading that ended the excursion.
    pub recovery_temperature_c: Option<f64>,
}

impl Incident {
    /// Whether the excursion is still going on.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.recovered_at.is_none()
    }

    /// Starts an incident at its first out-of-range reading.
    pub(super) fn open(refrigerator_id: &RefrigeratorId, reading: &TemperatureReading) -> Self {
        Self {
            incident_id: IncidentId::derive(refrigerator_id, reading.observed_at),
            refrigerator_id: refrigerator_id.clone(),
            started_at: reading.observed_at,
            initial_temperature_c: reading.temperature_c,
            highest_temperature_c: reading.temperature_c,
            lowest_temperature_c: reading.temperature_c,
            severity: Severity::Warning,
            exposure: Duration::ZERO,
            has_data_gap: false,
            recovered_at: None,
            recovery_temperature_c: None,
        }
    }

    /// Records a further reading that keeps the incident open.
    pub(super) fn extend(&mut self, reading: &TemperatureReading, after_gap: bool) {
        self.highest_temperature_c = self.highest_temperature_c.max(reading.temperature_c);
        self.lowest_temperature_c = self.lowest_temperature_c.min(reading.temperature_c);
        self.exposure = reading
            .observed_at
            .saturating_duration_since(self.started_at);
        self.has_data_gap |= after_gap;
    }

    /// Records the reading that ended the excursion.
    pub(super) fn close(&mut self, reading: &TemperatureReading, after_gap: bool) {
        self.has_data_gap |= after_gap;
        self.recovered_at = Some(reading.observed_at);
        self.recovery_temperature_c = Some(reading.temperature_c);
    }
}
