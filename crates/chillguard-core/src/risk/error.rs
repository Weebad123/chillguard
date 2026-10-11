//! Errors of the risk evaluation.

use thiserror::Error;

/// Why [`evaluate`](super::evaluate) refused to run.
///
/// Every variant is a configuration fault. Bad readings are not errors: they
/// are reported as [`DomainEvent::ReadingRejected`](super::DomainEvent) so
/// that one bad message cannot fail the caller.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum RiskError {
    /// A temperature limit is NaN or infinite.
    #[error("temperature limits must be finite numbers")]
    NonFiniteLimit,
    /// The lower limit is above the upper limit.
    #[error("lower limit {lower} °C is above upper limit {upper} °C")]
    LimitsInverted {
        /// The configured lower limit.
        lower: f64,
        /// The configured upper limit.
        upper: f64,
    },
    /// The hysteresis is negative, NaN, or infinite.
    #[error("hysteresis must be a finite number that is zero or positive")]
    InvalidHysteresis,
    /// The hysteresis is so wide that no temperature would count as recovered.
    #[error("hysteresis {hysteresis} °C leaves no temperature that counts as recovered")]
    HysteresisTooWide {
        /// The configured hysteresis.
        hysteresis: f64,
    },
    /// `stale_after` is zero, which would make every sensor stale.
    #[error("stale_after must be longer than zero")]
    ZeroStaleAfter,
}

impl RiskError {
    /// The fault as a fixed string, for callers whose errors are
    /// `&'static str` (a cell's handlers). Names the field at fault without
    /// the values that [`fmt::Display`](core::fmt::Display) adds.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonFiniteLimit => "temperature limits must be finite numbers",
            Self::LimitsInverted { .. } => "lower_limit_c is above upper_limit_c",
            Self::InvalidHysteresis => {
                "hysteresis_c must be a finite number that is zero or positive"
            }
            Self::HysteresisTooWide { .. } => {
                "hysteresis_c leaves no temperature that counts as recovered"
            }
            Self::ZeroStaleAfter => "stale_after must be longer than zero",
        }
    }
}
