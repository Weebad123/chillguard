//! Responses: sent back to a caller under the command name it asked for.
//!
//! A response is a command on the wire, so it is redelivered until the
//! caller's handler succeeds. Responses carry what was true when the
//! responding cell handled the query; a caller that needs to know how old
//! that is reads `as_of`.

use alloc::vec::Vec;

use chillguard_core::asset::AssetState;
use chillguard_core::risk::{
    Incident, IncidentId, LastReading, MonitorState, RefrigeratorConfig, RefrigeratorId, Status,
    Timestamp,
};
use serde::{Deserialize, Serialize};

use crate::envelope::{Kind, Payload};
use crate::validate::{self, ValidationError};

/// A refrigerator's state at one moment. The answer to
/// [`crate::commands::GetStatus`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusSnapshot {
    /// The refrigerator described.
    pub refrigerator_id: RefrigeratorId,
    /// Its current status.
    pub status: Status,
    /// The most recent accepted reading, if any.
    pub last_reading: Option<LastReading>,
    /// The incident in progress, if any.
    pub open_incident: Option<Incident>,
    /// The limits in force.
    pub config: RefrigeratorConfig,
    /// When the snapshot was taken.
    pub as_of: Timestamp,
}

impl StatusSnapshot {
    /// Builds a snapshot from the monitor state a cell holds.
    #[must_use]
    pub fn from_state(state: &MonitorState, config: &RefrigeratorConfig, as_of: Timestamp) -> Self {
        Self {
            refrigerator_id: config.refrigerator_id.clone(),
            status: state.status(),
            last_reading: state.last_reading(),
            open_incident: state.open_incident().cloned(),
            config: config.clone(),
            as_of,
        }
    }
}

impl StatusSnapshot {
    /// Builds a snapshot from what the asset cell holds.
    ///
    /// The status is the asset's view, as fresh as the last `status_changed`
    /// it received. The asset does not hold incidents, so `open_incident` is
    /// always `None` here, whatever the status; incidents are the risk
    /// monitor's to report.
    #[must_use]
    pub fn from_asset(asset: &AssetState, as_of: Timestamp) -> Self {
        Self {
            refrigerator_id: asset.config().refrigerator_id.clone(),
            status: asset.status(),
            last_reading: asset.last_reading(),
            open_incident: None,
            config: asset.config().clone(),
            as_of,
        }
    }
}

impl Payload for StatusSnapshot {
    const KIND: Kind = Kind::Response;

    fn validate(&self) -> Result<(), ValidationError> {
        validate::refrigerator_id(&self.refrigerator_id)?;
        if let Some(last) = self.last_reading {
            validate::temperature(last.temperature_c)?;
        }
        if let Some(incident) = &self.open_incident {
            validate::incident(incident)?;
        }
        self.config
            .validate()
            .map_err(ValidationError::InvalidLimits)
    }
}

/// A person has seen an incident.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acknowledgement {
    /// The incident that was acknowledged.
    pub incident_id: IncidentId,
    /// When the first acknowledgement arrived.
    pub acknowledged_at: Timestamp,
}

/// A page of incidents, newest first. The answer to
/// [`crate::commands::GetIncidents`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncidentList {
    /// The refrigerator the incidents belong to.
    pub refrigerator_id: RefrigeratorId,
    /// The incidents, newest first. An open incident, if any, comes first.
    pub incidents: Vec<Incident>,
    /// The acknowledgements of the incidents in `incidents`; an incident
    /// without an entry here has not been acknowledged. Absent on the wire
    /// means none.
    #[serde(default)]
    pub acknowledgements: Vec<Acknowledgement>,
    /// Whether more incidents exist than were returned.
    pub truncated: bool,
    /// When the list was assembled.
    pub as_of: Timestamp,
}

impl IncidentList {
    /// When `incident_id` was acknowledged, if it has been.
    #[must_use]
    pub fn acknowledged_at(&self, incident_id: &IncidentId) -> Option<Timestamp> {
        self.acknowledgements
            .iter()
            .find(|acknowledgement| acknowledgement.incident_id == *incident_id)
            .map(|acknowledgement| acknowledgement.acknowledged_at)
    }
}

impl Payload for IncidentList {
    const KIND: Kind = Kind::Response;

    fn validate(&self) -> Result<(), ValidationError> {
        validate::refrigerator_id(&self.refrigerator_id)?;
        self.incidents.iter().try_for_each(validate::incident)?;
        for acknowledgement in &self.acknowledgements {
            let listed = self
                .incidents
                .iter()
                .any(|incident| incident.incident_id == acknowledgement.incident_id);
            if !listed {
                return Err(ValidationError::AcknowledgementWithoutIncident);
            }
        }
        Ok(())
    }
}
