//! The cell's deploy-time configuration.

use chillguard_core::risk::RefrigeratorId;
use myrmic_sdk::Message;
use serde::{Deserialize, Serialize};

/// The init payload: `myrmic deploy ... --init-file config/sensor-cell.<profile>.json`.
///
/// Unknown fields are refused, so a misspelt key fails the deployment
/// instead of silently leaving a setting at its default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Message)]
#[serde(deny_unknown_fields)]
pub(crate) struct SensorConfig {
    /// The refrigerator the readings are stamped with.
    pub refrigerator_id: RefrigeratorId,
    /// Time between two readings, in milliseconds.
    pub interval_ms: u64,
    /// Whether the cell accepts development commands (`simulate_scenario`).
    #[serde(default)]
    pub dev_mode: bool,
    /// Seed for the simulator's noise.
    #[serde(default)]
    pub seed: u64,
    /// How much faster than the clock the scenario plays; `1.0` is real
    /// time. Readings are still stamped with the real clock. For tests and
    /// demonstrations: any other value needs `dev_mode`.
    #[serde(default = "real_time")]
    pub time_scale: f64,
}

const fn real_time() -> f64 {
    1.0
}

impl SensorConfig {
    /// Refuses a configuration the cell cannot run with. An error here fails
    /// the deployment.
    pub(crate) fn validate(&self) -> myrmic_sdk::Result {
        if self.refrigerator_id.as_str().is_empty() {
            return Err("refrigerator_id must not be empty");
        }
        if self.interval_ms == 0 {
            return Err("interval_ms must be greater than zero");
        }
        if !self.time_scale.is_finite() || self.time_scale <= 0.0 {
            return Err("time_scale must be a positive finite number");
        }
        // Exactly the default is real time; anything else is a compressed
        // or stretched clock, which only a development deployment may run.
        if self.time_scale.to_bits() != real_time().to_bits() && !self.dev_mode {
            return Err("time_scale other than 1 needs dev_mode: true");
        }
        Ok(())
    }
}
