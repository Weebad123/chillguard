//! Content checks shared by the payload types.

use chillguard_core::risk::{Incident, RefrigeratorId, RiskError, Status};
use thiserror::Error;

use crate::commands::MAX_INCIDENT_LIMIT;

/// What is wrong with the content of a message whose shape decoded.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ValidationError {
    /// A refrigerator identifier is empty.
    #[error("refrigerator_id must not be empty")]
    EmptyRefrigeratorId,
    /// An incident identifier is empty.
    #[error("incident_id must not be empty")]
    EmptyIncidentId,
    /// A temperature is NaN or infinite. (JSON cannot carry these; a sender
    /// that tries writes `null`, which fails the shape check instead.)
    #[error("temperature must be a finite number")]
    NonFiniteTemperature,
    /// An incident's lowest temperature is above its highest.
    #[error("incident temperatures are inconsistent")]
    InconsistentIncidentTemperatures,
    /// An incident recovered before it started.
    #[error("incident recovered before it started")]
    IncidentRecoveredBeforeStart,
    /// An incident has one of `recovered_at` and `recovery_temperature_c`
    /// without the other.
    #[error("incident recovery fields must be both set or both absent")]
    PartialIncidentRecovery,
    /// A status change whose two statuses are the same.
    #[error("status change must change the status")]
    StatusUnchanged,
    /// An alert whose severity disagrees with the incident it carries.
    #[error("alert severity does not match the incident")]
    AlertSeverityMismatch,
    /// An alert carrying an incident that is already closed.
    #[error("an alert must carry an open incident")]
    AlertForClosedIncident,
    /// A recovery event whose incident is still open or recovered at another
    /// time.
    #[error("recovery event does not match its incident")]
    RecoveryMismatch,
    /// A reading and the configuration sent with it name different
    /// refrigerators.
    #[error("reading and configuration are for different refrigerators")]
    ReadingConfigMismatch,
    /// A refrigerator configuration that core rejects.
    #[error("invalid limits: {0}")]
    InvalidLimits(RiskError),
    /// A reply target that is not a valid Myrmic command name.
    #[error("reply_to is not a valid command name: {0}")]
    InvalidReplyTo(&'static str),
    /// An incident page size outside `1..=MAX_INCIDENT_LIMIT`.
    #[error("limit {limit} is outside 1..={MAX_INCIDENT_LIMIT}")]
    LimitOutOfRange {
        /// The requested page size.
        limit: u16,
    },
}

impl ValidationError {
    /// The error as a `&'static str`, for the SDK's `Result`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::EmptyRefrigeratorId => "refrigerator_id must not be empty",
            Self::EmptyIncidentId => "incident_id must not be empty",
            Self::NonFiniteTemperature => "temperature must be a finite number",
            Self::InconsistentIncidentTemperatures => "incident temperatures are inconsistent",
            Self::IncidentRecoveredBeforeStart => "incident recovered before it started",
            Self::PartialIncidentRecovery => {
                "incident recovery fields must be both set or both absent"
            }
            Self::StatusUnchanged => "status change must change the status",
            Self::AlertSeverityMismatch => "alert severity does not match the incident",
            Self::AlertForClosedIncident => "an alert must carry an open incident",
            Self::RecoveryMismatch => "recovery event does not match its incident",
            Self::ReadingConfigMismatch => {
                "reading and configuration are for different refrigerators"
            }
            Self::InvalidLimits(_) => "invalid limits",
            Self::InvalidReplyTo(reason) => reason,
            Self::LimitOutOfRange { .. } => "limit is out of range",
        }
    }
}

pub(crate) fn refrigerator_id(id: &RefrigeratorId) -> Result<(), ValidationError> {
    if id.as_str().is_empty() {
        return Err(ValidationError::EmptyRefrigeratorId);
    }
    Ok(())
}

pub(crate) fn temperature(temperature_c: f64) -> Result<(), ValidationError> {
    if !temperature_c.is_finite() {
        return Err(ValidationError::NonFiniteTemperature);
    }
    Ok(())
}

pub(crate) fn status_change(from: Status, to: Status) -> Result<(), ValidationError> {
    if from == to {
        return Err(ValidationError::StatusUnchanged);
    }
    Ok(())
}

/// Checks an incident record as core would have produced it.
pub(crate) fn incident(incident: &Incident) -> Result<(), ValidationError> {
    if incident.incident_id.as_str().is_empty() {
        return Err(ValidationError::EmptyIncidentId);
    }
    refrigerator_id(&incident.refrigerator_id)?;
    temperature(incident.initial_temperature_c)?;
    temperature(incident.highest_temperature_c)?;
    temperature(incident.lowest_temperature_c)?;
    if incident.lowest_temperature_c > incident.highest_temperature_c {
        return Err(ValidationError::InconsistentIncidentTemperatures);
    }
    match (incident.recovered_at, incident.recovery_temperature_c) {
        (None, None) => {}
        (Some(recovered_at), Some(recovery_temperature_c)) => {
            temperature(recovery_temperature_c)?;
            if recovered_at < incident.started_at {
                return Err(ValidationError::IncidentRecoveredBeforeStart);
            }
        }
        _ => return Err(ValidationError::PartialIncidentRecovery),
    }
    Ok(())
}
