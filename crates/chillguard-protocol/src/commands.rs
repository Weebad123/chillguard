//! Commands: sent to one cell, redelivered until its handler succeeds.
//!
//! A command asks a cell to do something. Because the runtime retries a
//! failed command without bound, a handler must be able to see the same
//! command twice; every command here is safe to apply twice.
//!
//! Two commands are queries. They carry a [`ReplyTo`], the name of the
//! command on the caller that receives the response (see
//! [`crate::responses`]). The cell turns it into a `myrmic_sdk::Callback`
//! and invokes it on the sender.

use alloc::string::String;
use core::fmt;

use chillguard_core::risk::{IncidentId, RefrigeratorConfig};
use chillguard_core::simulator::Scenario;
use serde::{Deserialize, Serialize};

use crate::envelope::{Kind, Payload};
use crate::validate::ValidationError;

/// The largest page a [`GetIncidents`] may ask for.
///
/// A closed incident is about 350 bytes of JSON; a hundred of them stay well
/// inside what a cell can assemble and a receiver can read.
pub const MAX_INCIDENT_LIMIT: u16 = 100;

/// The name of the command on the caller that receives a response.
///
/// Must be a valid Myrmic command name: ASCII letters, digits and
/// underscores, not empty.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReplyTo(String);

impl ReplyTo {
    /// Wraps a command name after checking it against the SDK's rules.
    ///
    /// # Errors
    ///
    /// [`ValidationError::InvalidReplyTo`] with the SDK's reason.
    pub fn new(name: impl Into<String>) -> Result<Self, ValidationError> {
        let name = name.into();
        myrmic_sdk::Command::try_from(name.as_str()).map_err(ValidationError::InvalidReplyTo)?;
        Ok(Self(name))
    }

    /// The command name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self) -> Result<(), ValidationError> {
        Self::new(self.0.as_str()).map(|_| ())
    }
}

impl fmt::Display for ReplyTo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Asks the refrigerator asset cell for its current state. Answered with a
/// [`crate::responses::StatusSnapshot`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetStatus {
    /// Where to send the snapshot.
    pub reply_to: ReplyTo,
}

impl Payload for GetStatus {
    const KIND: Kind = Kind::Command("get_status");

    fn validate(&self) -> Result<(), ValidationError> {
        self.reply_to.validate()
    }
}

/// Asks for the most recent incidents, newest first. Answered with a
/// [`crate::responses::IncidentList`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetIncidents {
    /// How many incidents at most, `1..=MAX_INCIDENT_LIMIT`.
    pub limit: u16,
    /// Where to send the list.
    pub reply_to: ReplyTo,
}

impl Payload for GetIncidents {
    const KIND: Kind = Kind::Command("get_incidents");

    fn validate(&self) -> Result<(), ValidationError> {
        if !(1..=MAX_INCIDENT_LIMIT).contains(&self.limit) {
            return Err(ValidationError::LimitOutOfRange { limit: self.limit });
        }
        self.reply_to.validate()
    }
}

/// Replaces a refrigerator's limits and durations. Takes effect with the
/// next reading; a resulting status change is published as usual.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetLimits {
    /// The new configuration. Must pass core's validation.
    pub config: RefrigeratorConfig,
}

impl Payload for SetLimits {
    const KIND: Kind = Kind::Command("set_limits");

    fn validate(&self) -> Result<(), ValidationError> {
        crate::validate::refrigerator_id(&self.config.refrigerator_id)?;
        self.config
            .validate()
            .map_err(ValidationError::InvalidLimits)
    }
}

/// One of the simulator's built-in scenarios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioPreset {
    /// 24 hours of normal operation.
    NormalDay,
    /// A two-hour power failure with recovery.
    // Named explicitly: serde's `snake_case` puts no underscore before a
    // digit and would write `power_failure2h`.
    #[serde(rename = "power_failure_2h")]
    PowerFailure2h,
    /// Five minutes with the door open.
    ShortDoorOpen,
}

impl ScenarioPreset {
    /// The simulator scenario this preset names.
    #[must_use]
    pub fn scenario(self) -> Scenario {
        match self {
            Self::NormalDay => Scenario::normal_day(),
            Self::PowerFailure2h => Scenario::power_failure_2h(),
            Self::ShortDoorOpen => Scenario::short_door_open(),
        }
    }
}

/// Restarts the sensor cell's simulator on a preset scenario.
///
/// A development command: a cell accepts it only when its development
/// features are switched on, and rejects it otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulateScenario {
    /// The scenario to play.
    pub preset: ScenarioPreset,
    /// Seed for the simulator's noise. Zero when omitted.
    #[serde(default)]
    pub seed: u64,
}

impl Payload for SimulateScenario {
    const KIND: Kind = Kind::Command("simulate_scenario");
    const DEV_ONLY: bool = true;
}

/// Records that a person has seen an incident. Acknowledging an incident
/// twice, or one that is already closed, is not an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acknowledge {
    /// The incident being acknowledged.
    pub incident_id: IncidentId,
}

impl Payload for Acknowledge {
    const KIND: Kind = Kind::Command("acknowledge");

    fn validate(&self) -> Result<(), ValidationError> {
        if self.incident_id.as_str().is_empty() {
            return Err(ValidationError::EmptyIncidentId);
        }
        Ok(())
    }
}
