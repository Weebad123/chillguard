//! The refrigerator asset: what is known about one refrigerator besides its
//! risk state.
//!
//! [`AssetState`] holds the refrigerator's configuration, the latest reading
//! that was accepted, and a view of its status. The first two are owned here:
//! this is where they are decided. The status is not. It is decided by the
//! risk state machine ([`crate::risk::evaluate`]) and only mirrored here from
//! the status changes that machine announces, so it can lag behind
//! (docs/decisions/0005-state-ownership.md).
//!
//! Like the rest of core this is pure: every method takes its inputs as
//! parameters and the caller persists the result.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::risk::{
    LastReading, RefrigeratorConfig, RefrigeratorId, RejectReason, RiskError, Status,
    TemperatureReading, Timestamp, check_reading,
};

/// Everything the asset remembers about one refrigerator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetState {
    config: RefrigeratorConfig,
    last_reading: Option<LastReading>,
    status: Status,
    /// When the mirrored status last changed; `None` until the first change
    /// is seen.
    status_changed_at: Option<Timestamp>,
}

/// Why a new configuration was not taken.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ConfigError {
    /// The configuration does not describe a usable range.
    #[error("{0}")]
    Invalid(#[from] RiskError),
    /// The configuration is for another refrigerator.
    #[error("configuration is for refrigerator {actual}, this is {expected}")]
    WrongRefrigerator {
        /// The refrigerator this asset describes.
        expected: RefrigeratorId,
        /// The refrigerator named in the configuration.
        actual: RefrigeratorId,
    },
}

/// What became of an announced status change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusUpdate {
    /// The view now shows the announced status.
    Applied {
        /// The status the view showed before.
        previous: Status,
        /// Whether the change started from a status other than the one the
        /// view showed: at least one earlier change never arrived.
        missed_a_change: bool,
    },
    /// The change is older than one already applied and was ignored.
    Outdated,
    /// The change is about another refrigerator and was ignored.
    OtherRefrigerator,
}

impl AssetState {
    /// The state of an asset that has seen no reading and no status change.
    /// Its status view starts at `Normal`, which is also where the risk state
    /// machine starts.
    ///
    /// # Errors
    ///
    /// Returns the fault when `config` is invalid.
    pub fn new(config: RefrigeratorConfig) -> Result<Self, RiskError> {
        config.validate()?;
        Ok(Self {
            config,
            last_reading: None,
            status: Status::Normal,
            status_changed_at: None,
        })
    }

    /// The configuration in force.
    #[must_use]
    pub const fn config(&self) -> &RefrigeratorConfig {
        &self.config
    }

    /// The latest accepted reading, if any.
    #[must_use]
    pub const fn last_reading(&self) -> Option<LastReading> {
        self.last_reading
    }

    /// The mirrored status. It is as fresh as the last status change that
    /// arrived.
    #[must_use]
    pub const fn status(&self) -> Status {
        self.status
    }

    /// When the mirrored status last changed.
    #[must_use]
    pub const fn status_changed_at(&self) -> Option<Timestamp> {
        self.status_changed_at
    }

    /// Takes a reading as the latest one, if [`check_reading`] accepts it.
    ///
    /// # Errors
    ///
    /// Returns why the reading was refused. The state is unchanged.
    pub fn accept_reading(&mut self, reading: &TemperatureReading) -> Result<(), RejectReason> {
        check_reading(reading, self.last_reading, &self.config)?;
        self.last_reading = Some(LastReading {
            temperature_c: reading.temperature_c,
            observed_at: reading.observed_at,
        });
        Ok(())
    }

    /// Replaces the configuration. Returns whether anything changed, so that
    /// applying the same configuration twice is harmless and visible.
    ///
    /// # Errors
    ///
    /// Returns why the configuration was refused. The state is unchanged.
    pub fn set_config(&mut self, config: RefrigeratorConfig) -> Result<bool, ConfigError> {
        if config.refrigerator_id != self.config.refrigerator_id {
            return Err(ConfigError::WrongRefrigerator {
                expected: self.config.refrigerator_id.clone(),
                actual: config.refrigerator_id,
            });
        }
        config.validate()?;
        let changed = config != self.config;
        self.config = config;
        Ok(changed)
    }

    /// Mirrors a status change announced by the risk state machine.
    ///
    /// A change older than the one already applied is ignored, so that late
    /// delivery cannot move the view backwards. A change that starts from
    /// another status than the view shows is applied and reported: the view
    /// had missed something.
    pub fn apply_status_change(
        &mut self,
        refrigerator_id: &RefrigeratorId,
        from: Status,
        to: Status,
        at: Timestamp,
    ) -> StatusUpdate {
        if *refrigerator_id != self.config.refrigerator_id {
            return StatusUpdate::OtherRefrigerator;
        }
        if self.status_changed_at.is_some_and(|applied| at < applied) {
            return StatusUpdate::Outdated;
        }
        let previous = self.status;
        self.status = to;
        self.status_changed_at = Some(at);
        StatusUpdate::Applied {
            previous,
            missed_a_change: from != previous,
        }
    }
}

#[cfg(test)]
mod tests {
    // Temperatures are literals that pass through unchanged.
    #![allow(clippy::float_cmp)]

    use alloc::format;
    use core::time::Duration;

    use super::*;
    use crate::risk::ReadingSource;

    const FRIDGE: &str = "fridge-1";

    fn config() -> RefrigeratorConfig {
        RefrigeratorConfig {
            refrigerator_id: RefrigeratorId::new(FRIDGE),
            lower_limit_c: Some(2.0),
            upper_limit_c: 8.0,
            critical_after: Duration::from_mins(30),
            stale_after: Duration::from_mins(5),
            hysteresis_c: 0.0,
        }
    }

    fn at(minute: u64) -> Timestamp {
        Timestamp::from_millis(1_791_555_000_000 + minute * 60_000)
    }

    fn reading(fridge: &str, minute: u64, temperature_c: f64) -> TemperatureReading {
        TemperatureReading {
            refrigerator_id: RefrigeratorId::new(fridge),
            temperature_c,
            observed_at: at(minute),
            source: ReadingSource::Simulated,
        }
    }

    fn asset() -> AssetState {
        AssetState::new(config()).expect("valid config")
    }

    #[test]
    fn new_asset_is_normal_and_empty() {
        let asset = asset();
        assert_eq!(asset.config(), &config());
        assert_eq!(asset.last_reading(), None);
        assert_eq!(asset.status(), Status::Normal);
        assert_eq!(asset.status_changed_at(), None);
    }

    #[test]
    fn new_asset_needs_a_valid_config() {
        let inverted = RefrigeratorConfig {
            lower_limit_c: Some(9.0),
            ..config()
        };
        assert_eq!(
            AssetState::new(inverted),
            Err(RiskError::LimitsInverted {
                lower: 9.0,
                upper: 8.0
            })
        );
    }

    #[test]
    fn accepted_reading_becomes_the_latest() {
        let mut asset = asset();
        assert_eq!(asset.accept_reading(&reading(FRIDGE, 1, 5.5)), Ok(()));
        assert_eq!(asset.accept_reading(&reading(FRIDGE, 2, 6.5)), Ok(()));
        assert_eq!(
            asset.last_reading(),
            Some(LastReading {
                temperature_c: 6.5,
                observed_at: at(2)
            })
        );
        // The asset stores readings; it does not judge them. An out-of-range
        // value is accepted and leaves the status view alone.
        assert_eq!(asset.accept_reading(&reading(FRIDGE, 3, 40.0)), Ok(()));
        assert_eq!(asset.status(), Status::Normal);
    }

    #[test]
    fn refused_readings_leave_the_state_unchanged() {
        struct Case {
            name: &'static str,
            reading: TemperatureReading,
            reason: RejectReason,
        }
        let cases = [
            Case {
                name: "another refrigerator",
                reading: reading("fridge-2", 11, 5.0),
                reason: RejectReason::WrongRefrigerator {
                    expected: RefrigeratorId::new(FRIDGE),
                    actual: RefrigeratorId::new("fridge-2"),
                },
            },
            Case {
                name: "not a number",
                reading: reading(FRIDGE, 11, f64::NAN),
                reason: RejectReason::NonFiniteTemperature,
            },
            Case {
                name: "infinite",
                reading: reading(FRIDGE, 11, f64::INFINITY),
                reason: RejectReason::NonFiniteTemperature,
            },
            Case {
                name: "older than the last one",
                reading: reading(FRIDGE, 9, 5.0),
                reason: RejectReason::OutOfOrder {
                    observed_at: at(9),
                    last_reading_at: at(10),
                },
            },
            Case {
                name: "same timestamp as the last one",
                reading: reading(FRIDGE, 10, 7.0),
                reason: RejectReason::Duplicate {
                    observed_at: at(10),
                },
            },
        ];

        let mut accepted = asset();
        accepted
            .accept_reading(&reading(FRIDGE, 10, 5.0))
            .expect("first reading");
        for case in cases {
            let mut asset = accepted.clone();
            assert_eq!(
                asset.accept_reading(&case.reading),
                Err(case.reason),
                "{}",
                case.name
            );
            assert_eq!(asset, accepted, "{}", case.name);
        }
    }

    #[test]
    fn asset_and_risk_machine_accept_the_same_readings() {
        use crate::risk::{DomainEvent, Input, MonitorState, evaluate};

        // One stream with every kind of bad reading in it, fed to both.
        let stream = [
            reading(FRIDGE, 1, 5.0),
            reading("fridge-2", 2, 5.0),
            reading(FRIDGE, 3, f64::NAN),
            reading(FRIDGE, 4, 9.0),
            reading(FRIDGE, 4, 9.5),
            reading(FRIDGE, 2, 5.0),
            reading(FRIDGE, 5, 5.0),
        ];
        let mut asset = asset();
        let mut monitor = MonitorState::new();
        for reading in stream {
            let by_asset = asset.accept_reading(&reading).is_ok();
            let (next, events) =
                evaluate(&monitor, Input::Reading(reading.clone()), &config()).expect("valid");
            let by_monitor = !events
                .iter()
                .any(|event| matches!(event, DomainEvent::ReadingRejected { .. }));
            monitor = next;
            assert_eq!(by_asset, by_monitor, "{reading:?}");
            assert_eq!(asset.last_reading(), monitor.last_reading());
        }
    }

    #[test]
    fn set_config_replaces_validates_and_reports_change() {
        let mut asset = asset();
        asset
            .accept_reading(&reading(FRIDGE, 1, 5.0))
            .expect("reading");
        let wider = RefrigeratorConfig {
            upper_limit_c: 10.0,
            hysteresis_c: 0.5,
            ..config()
        };

        assert_eq!(asset.set_config(wider.clone()), Ok(true));
        assert_eq!(asset.config(), &wider);
        assert_eq!(
            asset.last_reading().map(|last| last.observed_at),
            Some(at(1)),
            "a new configuration keeps the latest reading"
        );
        // The same configuration again: taken, nothing changed.
        assert_eq!(asset.set_config(wider.clone()), Ok(false));

        // Refused configurations leave the one in force.
        let invalid = RefrigeratorConfig {
            stale_after: Duration::ZERO,
            ..config()
        };
        let error = asset.set_config(invalid).expect_err("invalid");
        assert_eq!(error, ConfigError::Invalid(RiskError::ZeroStaleAfter));
        assert_eq!(
            format!("{error}"),
            "stale_after must be longer than zero",
            "the message names the fault"
        );

        let foreign = RefrigeratorConfig {
            refrigerator_id: RefrigeratorId::new("fridge-2"),
            ..config()
        };
        let error = asset.set_config(foreign).expect_err("foreign");
        assert_eq!(
            format!("{error}"),
            "configuration is for refrigerator fridge-2, this is fridge-1"
        );
        assert_eq!(asset.config(), &wider);
    }

    #[test]
    fn status_view_follows_announced_changes() {
        let fridge = RefrigeratorId::new(FRIDGE);
        let mut asset = asset();

        let update = asset.apply_status_change(&fridge, Status::Normal, Status::Warning, at(10));
        assert_eq!(
            update,
            StatusUpdate::Applied {
                previous: Status::Normal,
                missed_a_change: false
            }
        );
        assert_eq!(asset.status(), Status::Warning);
        assert_eq!(asset.status_changed_at(), Some(at(10)));

        // A change that does not start where the view stands: applied, and
        // reported as a sign that an earlier one was lost.
        let update = asset.apply_status_change(&fridge, Status::Critical, Status::Normal, at(90));
        assert_eq!(
            update,
            StatusUpdate::Applied {
                previous: Status::Warning,
                missed_a_change: true
            }
        );
        assert_eq!(asset.status(), Status::Normal);
    }

    #[test]
    fn status_view_ignores_outdated_and_foreign_changes() {
        let fridge = RefrigeratorId::new(FRIDGE);
        let mut asset = asset();
        asset.apply_status_change(&fridge, Status::Normal, Status::Critical, at(40));
        let before = asset.clone();

        // Delivered late: it happened before the change already applied.
        let update = asset.apply_status_change(&fridge, Status::Normal, Status::Warning, at(10));
        assert_eq!(update, StatusUpdate::Outdated);
        assert_eq!(asset, before);

        let other = RefrigeratorId::new("fridge-2");
        let update = asset.apply_status_change(&other, Status::Critical, Status::Normal, at(50));
        assert_eq!(update, StatusUpdate::OtherRefrigerator);
        assert_eq!(asset, before);

        // The same instant is not older: two changes may share a timestamp.
        let update = asset.apply_status_change(&fridge, Status::Critical, Status::Normal, at(40));
        assert!(matches!(update, StatusUpdate::Applied { .. }));
    }

    #[test]
    fn state_survives_serialisation() {
        let mut asset = asset();
        asset
            .accept_reading(&reading(FRIDGE, 1, 5.5))
            .expect("reading");
        asset.apply_status_change(
            &RefrigeratorId::new(FRIDGE),
            Status::Normal,
            Status::Warning,
            at(1),
        );
        // Postcard is what a cell's `State` stores.
        let bytes = postcard::to_allocvec(&asset).expect("serialises");
        let back: AssetState = postcard::from_bytes(&bytes).expect("deserialises");
        assert_eq!(back, asset);
        let text = serde_json::to_string(&asset).expect("serialises");
        let back: AssetState = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, asset);
    }
}
