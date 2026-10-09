//! Per-refrigerator settings and their validation.

use core::time::Duration;

use serde::{Deserialize, Serialize};

use super::error::RiskError;
use super::model::RefrigeratorId;

/// The settings the risk rules need for one refrigerator.
///
/// The safe range is `lower_limit_c..=upper_limit_c`: a reading exactly on a
/// limit is in range.
///
/// The durations are serialised as whole milliseconds; see
/// [`crate::duration_millis`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefrigeratorConfig {
    /// The refrigerator these settings belong to. Readings carrying another
    /// identifier are rejected.
    pub refrigerator_id: RefrigeratorId,
    /// Lowest in-range temperature in degrees Celsius, if there is one.
    #[serde(default)]
    pub lower_limit_c: Option<f64>,
    /// Highest in-range temperature in degrees Celsius.
    pub upper_limit_c: f64,
    /// How long an excursion may last before it becomes critical. Zero makes
    /// every excursion critical from its first reading.
    #[serde(with = "crate::duration_millis")]
    pub critical_after: Duration,
    /// How long the sensor may stay silent before it counts as stale. Also
    /// the threshold above which the time between two readings of an incident
    /// counts as a data gap. Must be longer than zero.
    #[serde(with = "crate::duration_millis")]
    pub stale_after: Duration,
    /// How far inside the limits a reading must be to end an incident, in
    /// degrees Celsius. Zero, the default, turns hysteresis off.
    #[serde(default)]
    pub hysteresis_c: f64,
}

impl RefrigeratorConfig {
    /// Settings with an upper limit only and hysteresis turned off.
    #[must_use]
    pub const fn new(
        refrigerator_id: RefrigeratorId,
        upper_limit_c: f64,
        critical_after: Duration,
        stale_after: Duration,
    ) -> Self {
        Self {
            refrigerator_id,
            lower_limit_c: None,
            upper_limit_c,
            critical_after,
            stale_after,
            hysteresis_c: 0.0,
        }
    }

    /// Checks that the settings describe a usable range.
    ///
    /// # Errors
    ///
    /// Returns the first fault found: a non-finite limit, a lower limit above
    /// the upper limit, a negative or non-finite hysteresis, a hysteresis that
    /// leaves no recovery range, or a zero `stale_after`.
    pub fn validate(&self) -> Result<(), RiskError> {
        let lower = self.lower_limit_c;
        let upper = self.upper_limit_c;
        let hysteresis = self.hysteresis_c;

        if !upper.is_finite() || lower.is_some_and(|lower| !lower.is_finite()) {
            return Err(RiskError::NonFiniteLimit);
        }
        if let Some(lower) = lower
            && lower > upper
        {
            return Err(RiskError::LimitsInverted { lower, upper });
        }
        if !hysteresis.is_finite() || hysteresis < 0.0 {
            return Err(RiskError::InvalidHysteresis);
        }
        if let Some(lower) = lower
            && lower + hysteresis > upper - hysteresis
        {
            return Err(RiskError::HysteresisTooWide { hysteresis });
        }
        if self.stale_after.is_zero() {
            return Err(RiskError::ZeroStaleAfter);
        }
        Ok(())
    }

    /// Whether a temperature lies outside the safe range. The limits
    /// themselves are in range.
    #[must_use]
    pub fn is_out_of_range(&self, temperature_c: f64) -> bool {
        temperature_c > self.upper_limit_c
            || self
                .lower_limit_c
                .is_some_and(|lower| temperature_c < lower)
    }

    /// Whether a temperature ends an open incident: inside the safe range by
    /// at least the hysteresis. Without hysteresis this is the opposite of
    /// [`Self::is_out_of_range`].
    pub(super) fn is_recovered(&self, temperature_c: f64) -> bool {
        temperature_c <= self.upper_limit_c - self.hysteresis_c
            && self
                .lower_limit_c
                .is_none_or(|lower| temperature_c >= lower + self.hysteresis_c)
    }
}
