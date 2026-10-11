//! Myrmic handler exports for the cell.
//!
//! Only functions carrying an SDK handler attribute belong here; keep them to
//! decode, call `chillguard-core`, persist, emit.

// The SDK's handler attributes expand to `#[unsafe(no_mangle)] extern "C"`
// exports, which the workspace-wide `unsafe_code = "deny"` rejects. The allow
// is confined to this module; nothing here may contain hand-written `unsafe`.
#![allow(unsafe_code)]

use chillguard_core::asset::{AssetState, StatusUpdate};
use chillguard_core::risk::{RejectReason, RiskError, TemperatureReading, Timestamp};
use chillguard_protocol::commands::{GetStatus, SetLimits};
use chillguard_protocol::events::{ReadingAccepted, ReadingObserved, StatusChanged};
use chillguard_protocol::responses::StatusSnapshot;
use chillguard_protocol::{Envelope, Payload};
use myrmic_sdk::db::state::State;
use myrmic_sdk::db::{FieldValue, Measurement, Scope, publish_measurement};
use myrmic_sdk::{Bytes, Callback, Metadata, Result, String, Vec, vec};

use crate::config::AssetInit;

/// The event accepted readings are published under. A test pins it to the
/// protocol.
const READING_ACCEPTED: &str = "reading_accepted";

/// The refrigerator's configuration, latest reading and status view.
const ASSET: State<AssetState> = State::new_const("asset");
/// The `record_history` setting of the current deployment.
const RECORD_HISTORY: State<bool> = State::new_const("record_history");

/// A value for a log field, or `none`.
fn or_none<T: core::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(
        || String::from("none"),
        |value| myrmic_sdk::format!("{value}"),
    )
}

fn load() -> Result<AssetState> {
    ASSET.load()?.ok_or("asset state is missing")
}

/// Decodes a command's payload. A command that returns an error is
/// redelivered without end, so a payload that can never be accepted is
/// logged and dropped: the caller gets `None` and returns `Ok`.
fn decode<T: Payload>(command: &str, payload: &[u8]) -> Result<Option<T>> {
    match Envelope::decode(payload) {
        Ok(envelope) => Ok(Some(envelope.body)),
        Err(error) => {
            myrmic_sdk::warn!("{command} rejected reason=\"{error}\"")?;
            Ok(None)
        }
    }
}

/// Runs once per incarnation, before any other handler: on deploy and again
/// on every restart.
#[myrmic_sdk::init]
fn init(md: Metadata, init: AssetInit) -> Result {
    if let Err(reason) = init.validate() {
        myrmic_sdk::error!("init refused reason=\"{reason}\"")?;
        return Err(reason);
    }
    // Cannot fail: `validate` ran the same checks.
    let fresh = AssetState::new(init.config).map_err(RiskError::as_str)?;
    // State survives restarts and redeployments. What is stored for this
    // refrigerator wins over the init payload: its configuration may have
    // been changed by `set_limits`, and a restart must not undo that.
    let stored = ASSET.load().ok().flatten();
    let stored = stored.filter(|s| s.config().refrigerator_id == fresh.config().refrigerator_id);
    let resumed = stored.is_some();
    let asset = if let Some(stored) = stored {
        stored
    } else {
        ASSET.save(&fresh)?;
        fresh
    };
    RECORD_HISTORY.save(&init.record_history)?;
    let config = asset.config();
    myrmic_sdk::info!(
        "fridge-asset-cell started id={} refrigerator_id={} resumed={resumed} status={} \
         observed_at={} lower_limit_c={} upper_limit_c={} record_history={}",
        md.id,
        config.refrigerator_id,
        asset.status(),
        or_none(
            asset
                .last_reading()
                .map(|last| last.observed_at.as_millis())
        ),
        or_none(config.lower_limit_c),
        config.upper_limit_c,
        init.record_history,
    )?;
    Ok(())
}

/// Every sensor's readings arrive here; core decides which one counts.
#[myrmic_sdk::evt]
fn reading_observed(_md: Metadata, event: Envelope<ReadingObserved>) -> Result {
    let reading = event.body.reading;
    let mut asset = load()?;
    match asset.accept_reading(&reading) {
        Ok(()) => {}
        // Every asset cell hears every sensor; another refrigerator's
        // reading is routine and not worth a line.
        Err(RejectReason::WrongRefrigerator { .. }) => return Ok(()),
        Err(reason) => {
            myrmic_sdk::warn!(
                "reading rejected refrigerator_id={} temperature_c={:.3} observed_at={} reason={}",
                reading.refrigerator_id,
                reading.temperature_c,
                reading.observed_at.as_millis(),
                reason.as_str(),
            )?;
            return Ok(());
        }
    }
    ASSET.save(&asset)?;
    let config = asset.config().clone();
    let accepted = Envelope::new(ReadingAccepted { reading, config });
    myrmic_sdk::publish(READING_ACCEPTED, &accepted)?;

    let reading = &accepted.body.reading;
    if RECORD_HISTORY.load()? == Some(true) {
        record(reading)?;
    }
    myrmic_sdk::info!(
        "reading accepted refrigerator_id={} temperature_c={:.3} observed_at={}",
        reading.refrigerator_id,
        reading.temperature_c,
        reading.observed_at.as_millis(),
    )?;
    Ok(())
}

/// Appends a reading to the cell's private time series `temp`. The history
/// is a convenience: a failure is logged and does not undo the reading.
fn record(reading: &TemperatureReading) -> Result {
    let sample = Measurement {
        name: String::from("temp"),
        tags: Vec::new(),
        fields: vec![(String::from("c"), FieldValue::F64(reading.temperature_c))],
        ts: Some(reading.observed_at.as_millis()),
    };
    if let Err(error) = publish_measurement(Scope::private(), sample) {
        myrmic_sdk::warn!("history append failed reason={error:?}")?;
    }
    Ok(())
}

/// Keeps the status view in step with what the risk monitor announces.
#[myrmic_sdk::evt]
fn status_changed(_md: Metadata, event: Envelope<StatusChanged>) -> Result {
    let StatusChanged {
        refrigerator_id,
        from,
        to,
        at,
    } = event.body;
    let mut asset = load()?;
    match asset.apply_status_change(&refrigerator_id, from, to, at) {
        StatusUpdate::OtherRefrigerator => {}
        StatusUpdate::Outdated => myrmic_sdk::warn!(
            "status change ignored refrigerator_id={refrigerator_id} status={to} at={} \
             reason=older_than_the_view",
            at.as_millis(),
        )?,
        StatusUpdate::Applied {
            previous,
            missed_a_change,
        } => {
            ASSET.save(&asset)?;
            if missed_a_change {
                myrmic_sdk::warn!(
                    "status view was behind refrigerator_id={refrigerator_id} \
                     previous_status={previous} announced_previous_status={from}"
                )?;
            }
            myrmic_sdk::info!(
                "status view changed refrigerator_id={refrigerator_id} status={to} \
                 previous_status={previous} at={}",
                at.as_millis(),
            )?;
        }
    }
    Ok(())
}

/// Query: the refrigerator's state as this cell knows it.
#[myrmic_sdk::cmd]
fn get_status(md: Metadata, payload: Bytes) -> Result {
    let Some(GetStatus { reply_to }) = decode("get_status", &payload)? else {
        return Ok(());
    };
    let now = u64::try_from(myrmic_sdk::now()?.as_millis()).map_err(|_| "clock out of range")?;
    let snapshot = StatusSnapshot::from_asset(&load()?, Timestamp::from_millis(now));
    let snapshot = Envelope::new(snapshot);
    // A cell gets the answer as a command. A caller without an identity (the
    // Myrmic CLI) cannot be sent anything, so for it the answer is published
    // as an event under the name it asked for.
    let via = if md.sender.is_nil() {
        myrmic_sdk::publish(reply_to.as_str(), &snapshot)?;
        "event"
    } else {
        Callback::to(reply_to.as_str())?.invoke(md.sender, &snapshot)?;
        "command"
    };
    myrmic_sdk::info!(
        "status sent refrigerator_id={} status={} reply_to={reply_to} via={via}",
        snapshot.body.refrigerator_id,
        snapshot.body.status,
    )?;
    Ok(())
}

/// Command: replace the refrigerator's limits and durations.
#[myrmic_sdk::cmd]
fn set_limits(_md: Metadata, payload: Bytes) -> Result {
    let Some(SetLimits { config }) = decode("set_limits", &payload)? else {
        return Ok(());
    };
    let mut asset = load()?;
    match asset.set_config(config) {
        Ok(true) => {
            ASSET.save(&asset)?;
            let config = asset.config();
            myrmic_sdk::info!(
                "limits changed refrigerator_id={} lower_limit_c={} upper_limit_c={} \
                 critical_after_ms={} stale_after_ms={} hysteresis_c={}",
                config.refrigerator_id,
                or_none(config.lower_limit_c),
                config.upper_limit_c,
                config.critical_after.as_millis(),
                config.stale_after.as_millis(),
                config.hysteresis_c,
            )?;
        }
        Ok(false) => myrmic_sdk::info!(
            "limits unchanged refrigerator_id={}",
            asset.config().refrigerator_id
        )?,
        Err(error) => myrmic_sdk::warn!("set_limits rejected reason=\"{error}\"")?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chillguard_protocol::Kind;
    use myrmic_sdk::Handler;

    use super::*;

    /// The names in this file are literals because the handler attributes
    /// need them to be; this ties the ones that can be tied to the protocol.
    /// The two `#[evt]` handlers have no marker type to check; `make e2e` and
    /// the deployment test are what catch a misnamed one.
    #[test]
    fn wire_names_match_the_protocol() {
        assert_eq!(ReadingAccepted::KIND, Kind::Event(READING_ACCEPTED));
        assert_eq!(
            GetStatus::KIND,
            Kind::Command(<get_status as Handler>::NAME)
        );
        assert_eq!(
            SetLimits::KIND,
            Kind::Command(<set_limits as Handler>::NAME)
        );
        assert_eq!(ReadingObserved::KIND, Kind::Event("reading_observed"));
        assert_eq!(StatusChanged::KIND, Kind::Event("status_changed"));
    }
}
