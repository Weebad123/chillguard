//! Events: published by one cell, observed by any number of subscribers.
//!
//! An event states a fact that has happened. It is delivered to the cells
//! that are subscribed at the time of publishing, and it is dropped if a
//! handler fails, so nothing here is a request for work.

use chillguard_core::risk::{
    DomainEvent, Incident, RefrigeratorId, Severity, Status, TemperatureReading, Timestamp,
};
use serde::{Deserialize, Serialize};

use crate::envelope::{Envelope, Kind, Payload};
use crate::validate::{self, ValidationError};

/// A sensor took a reading. Published by the sensor cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingObserved {
    /// The reading, as the sensor produced it.
    pub reading: TemperatureReading,
}

impl Payload for ReadingObserved {
    const KIND: Kind = Kind::Event("reading_observed");

    fn validate(&self) -> Result<(), ValidationError> {
        validate::refrigerator_id(&self.reading.refrigerator_id)?;
        validate::temperature(self.reading.temperature_c)
    }
}

/// A refrigerator's status changed. Published by the risk monitor cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusChanged {
    /// The refrigerator whose status changed.
    pub refrigerator_id: RefrigeratorId,
    /// The status before.
    pub from: Status,
    /// The status after.
    pub to: Status,
    /// When the change took effect: the reading's timestamp, or the time of
    /// the tick that found the sensor stale.
    pub at: Timestamp,
}

impl Payload for StatusChanged {
    const KIND: Kind = Kind::Event("status_changed");

    fn validate(&self) -> Result<(), ValidationError> {
        validate::refrigerator_id(&self.refrigerator_id)?;
        validate::status_change(self.from, self.to)
    }
}

/// An incident opened (warning) or escalated (critical). Published by the
/// risk monitor cell, once per severity per incident.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertRaised {
    /// The severity being raised.
    pub severity: Severity,
    /// The incident at the moment the alert was raised. Its own `severity`
    /// is the same value.
    pub incident: Incident,
}

impl Payload for AlertRaised {
    const KIND: Kind = Kind::Event("alert_raised");

    fn validate(&self) -> Result<(), ValidationError> {
        validate::incident(&self.incident)?;
        if self.incident.severity != self.severity {
            return Err(ValidationError::AlertSeverityMismatch);
        }
        if !self.incident.is_open() {
            return Err(ValidationError::AlertForClosedIncident);
        }
        Ok(())
    }
}

/// A reading ended an incident. Published by the risk monitor cell.
///
/// This records an observation. It does not mean the refrigerator's contents
/// are safe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncidentRecovered {
    /// The closed incident, with the severity it reached.
    pub incident: Incident,
    /// Timestamp of the reading that ended it.
    pub at: Timestamp,
    /// Temperature of that reading, in degrees Celsius.
    pub temperature_c: f64,
}

impl Payload for IncidentRecovered {
    const KIND: Kind = Kind::Event("incident_recovered");

    fn validate(&self) -> Result<(), ValidationError> {
        validate::incident(&self.incident)?;
        validate::temperature(self.temperature_c)?;
        if self.incident.recovered_at != Some(self.at)
            || self.incident.recovery_temperature_c != Some(self.temperature_c)
        {
            return Err(ValidationError::RecoveryMismatch);
        }
        Ok(())
    }
}

/// Any of the events the risk monitor publishes, ready to hand to
/// `myrmic_sdk::publish`.
#[derive(Debug, Clone, PartialEq)]
pub enum MonitorEvent {
    /// See [`StatusChanged`].
    StatusChanged(Envelope<StatusChanged>),
    /// See [`AlertRaised`].
    AlertRaised(Envelope<AlertRaised>),
    /// See [`IncidentRecovered`].
    IncidentRecovered(Envelope<IncidentRecovered>),
}

impl MonitorEvent {
    /// Translates what core's `evaluate` returned into the event to publish,
    /// if that event is part of the contract.
    ///
    /// Stale and resumed sensors are reported through the status change they
    /// cause, and a rejected reading is the cell's business to log, so those
    /// three domain events map to `None`.
    #[must_use]
    pub fn from_domain(refrigerator_id: &RefrigeratorId, event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::StatusChanged { from, to, at } => {
                Some(Self::StatusChanged(Envelope::new(StatusChanged {
                    refrigerator_id: refrigerator_id.clone(),
                    from: *from,
                    to: *to,
                    at: *at,
                })))
            }
            DomainEvent::WarningRaised { incident } => {
                Some(Self::AlertRaised(Envelope::new(AlertRaised {
                    severity: Severity::Warning,
                    incident: incident.clone(),
                })))
            }
            DomainEvent::CriticalRaised { incident } => {
                Some(Self::AlertRaised(Envelope::new(AlertRaised {
                    severity: Severity::Critical,
                    incident: incident.clone(),
                })))
            }
            DomainEvent::Recovered {
                incident,
                at,
                temperature,
            } => Some(Self::IncidentRecovered(Envelope::new(IncidentRecovered {
                incident: incident.clone(),
                at: *at,
                temperature_c: *temperature,
            }))),
            DomainEvent::SensorStale { .. }
            | DomainEvent::SensorResumed { .. }
            | DomainEvent::ReadingRejected { .. } => None,
        }
    }

    /// The event name to publish under.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::StatusChanged(_) => event_name(StatusChanged::KIND),
            Self::AlertRaised(_) => event_name(AlertRaised::KIND),
            Self::IncidentRecovered(_) => event_name(IncidentRecovered::KIND),
        }
    }

    /// Encodes the event for the wire.
    ///
    /// # Errors
    ///
    /// See [`Envelope::encode`].
    pub fn encode(&self) -> Result<alloc::vec::Vec<u8>, crate::ProtocolError> {
        match self {
            Self::StatusChanged(envelope) => envelope.encode(),
            Self::AlertRaised(envelope) => envelope.encode(),
            Self::IncidentRecovered(envelope) => envelope.encode(),
        }
    }
}

const fn event_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Event(name) => name,
        // Every variant of `MonitorEvent` is an event; the match above only
        // passes event kinds.
        Kind::Command(_) | Kind::Response => "",
    }
}
