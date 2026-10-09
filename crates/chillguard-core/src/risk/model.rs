//! Inputs to the risk evaluation: time, identity, and readings.

use alloc::string::String;
use core::fmt;
use core::time::Duration;

use serde::{Deserialize, Serialize};

/// A point in time, in milliseconds since the UNIX epoch.
///
/// Core never reads a clock; every timestamp is handed in by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(u64);

impl Timestamp {
    /// Builds a timestamp from milliseconds since the UNIX epoch.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// Milliseconds since the UNIX epoch.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// Time elapsed from `earlier` to `self`; zero when `earlier` is the
    /// later of the two.
    #[must_use]
    pub const fn saturating_duration_since(self, earlier: Self) -> Duration {
        Duration::from_millis(self.0.saturating_sub(earlier.0))
    }
}

/// Identifies one refrigerator.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RefrigeratorId(String);

impl RefrigeratorId {
    /// Wraps an identifier.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The identifier as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RefrigeratorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a reading came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingSource {
    /// Produced by the simulator.
    Simulated,
    /// Measured by a physical sensor.
    Sensor,
}

/// One timestamped temperature observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemperatureReading {
    /// The refrigerator the reading belongs to.
    pub refrigerator_id: RefrigeratorId,
    /// Observed temperature in degrees Celsius.
    pub temperature_c: f64,
    /// When the temperature was observed.
    pub observed_at: Timestamp,
    /// Where the reading came from. No rule depends on it.
    pub source: ReadingSource,
}

/// What the state machine reacts to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    /// A temperature reading arrived.
    Reading(TemperatureReading),
    /// Time passed. A tick is the only input that can detect a stale sensor.
    Tick {
        /// The current time.
        now: Timestamp,
    },
}
