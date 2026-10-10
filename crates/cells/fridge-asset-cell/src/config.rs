//! The cell's deploy-time configuration.

use chillguard_core::risk::RefrigeratorConfig;
use myrmic_sdk::Message;
use serde::{Deserialize, Serialize};

/// The init payload: `myrmic deploy ... --init-file config/fridge-asset-cell.dev.json`.
///
/// Unknown top-level fields are refused, so a misspelt key fails the
/// deployment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Message)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssetInit {
    /// The refrigerator's configuration for a first deployment. A cell that
    /// already holds state for this refrigerator keeps its stored
    /// configuration, which `set_limits` may have changed since.
    pub config: RefrigeratorConfig,
    /// Whether every accepted reading is also appended to the time-series
    /// store. Off by default: the store has no expiry.
    #[serde(default)]
    pub record_history: bool,
}
