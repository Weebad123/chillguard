//! The cell's deploy-time configuration.

use chillguard_core::risk::RefrigeratorId;
use myrmic_sdk::Message;
use serde::{Deserialize, Serialize};

/// The init payload: `myrmic deploy ... --init-file config/risk-monitor-cell.dev.json`.
///
/// The refrigerator's limits are not part of it: they arrive with every
/// `reading_accepted`, from the asset cell that owns them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Message)]
#[serde(deny_unknown_fields)]
pub(crate) struct MonitorInit {
    /// The refrigerator this monitor evaluates. Readings for any other are
    /// ignored.
    pub refrigerator_id: RefrigeratorId,
}

impl MonitorInit {
    /// Refuses a configuration the cell cannot run with. An error here fails
    /// the deployment.
    pub(crate) fn validate(&self) -> myrmic_sdk::Result {
        if self.refrigerator_id.as_str().is_empty() {
            return Err("refrigerator_id must not be empty");
        }
        Ok(())
    }
}
