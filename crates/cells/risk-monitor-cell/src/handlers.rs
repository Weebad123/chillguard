//! Myrmic handler exports for the cell.
//!
//! Only functions carrying an SDK handler attribute belong here; keep them to
//! decode, call `chillguard-core`, persist, emit.
//!
//! # Persist, then emit
//!
//! Every handler writes its state and incident rows before it publishes, and
//! logs last. The runtime commits a handler's storage writes and outbound
//! messages together when the handler returns `Ok`, and discards both when
//! it fails (docs/myrmic-notes.md, section 6), so the order inside a handler
//! does not decide what the outside sees. What it decides is what a reader
//! of this file sees: the state an event describes is the state that was
//! saved. Logs are not rolled back, so a log line is written only once the
//! work it reports is done.
//!
//! An alert is raised once per incident and severity because the `Incident`
//! that records it (`severity`) is saved in the same transaction as the
//! `alert_raised` that announces it. After a restart, `evaluate` finds the
//! severity already reached and does not raise it again.

// The SDK's handler attributes expand to `#[unsafe(no_mangle)] extern "C"`
// exports, which the workspace-wide `unsafe_code = "deny"` rejects. The allow
// is confined to this module; nothing here may contain hand-written `unsafe`.
#![allow(unsafe_code)]

use core::time::Duration;

use chillguard_core::risk::{
    DomainEvent, Incident, Input, MonitorState, RefrigeratorConfig, RefrigeratorId, Timestamp,
    evaluate,
};
use chillguard_protocol::commands::{Acknowledge, GetIncidents};
use chillguard_protocol::events::{MonitorEvent, ReadingAccepted};
use chillguard_protocol::responses::{Acknowledgement, IncidentList};
use chillguard_protocol::{Envelope, Payload};
use myrmic_sdk::db::state::State;
use myrmic_sdk::db::table::Table;
use myrmic_sdk::{Bytes, Callback, InMemory, Metadata, Result, String, TimerHandle, Vec};
use serde::{Deserialize, Serialize};

use crate::config::MonitorInit;

/// Shortest tick period. `stale_after / 2` below this would only burn the
/// cell's handler budget; a `stale_after` that short is not a real setting.
const MIN_TICK: Duration = Duration::from_millis(250);

/// The refrigerator this monitor is for.
const FRIDGE: State<RefrigeratorId> = State::new_const("refrigerator_id");
/// Core's state between two inputs.
const MONITOR: State<MonitorState> = State::new_const("monitor");
/// The configuration that came with the last accepted reading. Ticks are
/// evaluated against it.
const CONFIG: State<RefrigeratorConfig> = State::new_const("config");
/// Every incident this monitor has opened, keyed by incident id. Ids are
/// `<refrigerator>-<started_at ms>`, so key order is time order.
const INCIDENTS: Table<IncidentRecord> = Table::new("incidents");
/// The running tick timer and the period it was armed with. In Wasm memory
/// on purpose: a handle from an earlier incarnation would name a timer that
/// no longer exists, or a new one (docs/myrmic-notes.md, section 10).
static TIMER: InMemory<Option<(TimerHandle, Duration)>> = InMemory::empty();

/// A row of the incident table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct IncidentRecord {
    incident: Incident,
    /// When a person acknowledged the incident, if they have.
    acknowledged_at: Option<Timestamp>,
}

fn now() -> Result<Timestamp> {
    let millis = u64::try_from(myrmic_sdk::now()?.as_millis()).map_err(|_| "clock out of range")?;
    Ok(Timestamp::from_millis(millis))
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

/// Arms the tick timer for `config.stale_after`, replacing the running one
/// when the period changed. A stale sensor is only ever noticed by a tick,
/// and a tick every half `stale_after` notices it within that much again.
fn arm_timer(config: &RefrigeratorConfig) -> Result {
    let period = (config.stale_after / 2).max(MIN_TICK);
    let armed = TIMER.with(|slot| slot.as_ref().map(|(_, period)| *period))?;
    if armed == Some(period) {
        return Ok(());
    }
    if let Some((previous, _)) = TIMER.with(Option::take)? {
        previous.cancel()?;
    }
    let handle = myrmic_sdk::interval(Callback::of::<tick>(), period).build()?;
    TIMER.with(|slot| *slot = Some((handle, period)))?;
    myrmic_sdk::info!(
        "tick armed refrigerator_id={} period_ms={}",
        config.refrigerator_id,
        period.as_millis()
    )?;
    Ok(())
}

/// Runs core on one input, then persists, publishes and logs what it
/// returned. The one place where the monitor's state changes.
fn apply(input: Input, config: &RefrigeratorConfig) -> Result {
    let fridge = &config.refrigerator_id;
    let reading = match &input {
        Input::Reading(reading) => Some(reading.clone()),
        Input::Tick { .. } => None,
    };
    let state = MONITOR.load()?.unwrap_or_default();
    let (next, events) = evaluate(&state, input, config).map_err(|_| "invalid configuration")?;

    // Persist: the monitor state, the open incident whenever a reading
    // touched it (exposure, peaks, data gap, severity), and the record a
    // recovery closes. Core says which; writing rows only when an alert
    // fired left `get_incidents` reporting an open incident as of its last
    // alert (regression test in core: `changed_incident_is_due_...`).
    MONITOR.save(&next)?;
    let recovered = events.iter().find_map(|event| match event {
        DomainEvent::Recovered { incident, .. } => Some(incident),
        _ => None,
    });
    if let Some(incident) = next.changed_incident(&state).or(recovered) {
        let key = String::from(incident.incident_id.as_str());
        let acknowledged_at = INCIDENTS.get(&key)?.and_then(|row| row.acknowledged_at);
        INCIDENTS.insert_with(
            &key,
            &IncidentRecord {
                incident: incident.clone(),
                acknowledged_at,
            },
        )?;
    }

    // Emit.
    for event in &events {
        if let Some(outbound) = MonitorEvent::from_domain(fridge, event) {
            let bytes = outbound.encode().map_err(|error| error.as_str())?;
            myrmic_sdk::publish(outbound.name(), &bytes)?;
        }
    }

    // Log. Every line names the refrigerator, the incident (or `none`) and
    // the status after this input; lines caused by a reading also carry its
    // temperature and timestamp.
    let status = next.status();
    let incident_id = next
        .open_incident()
        .map_or("none", |incident| incident.incident_id.as_str());
    let reading = match &reading {
        Some(reading) => myrmic_sdk::format!(
            " temperature_c={:.3} observed_at={}",
            reading.temperature_c,
            reading.observed_at.as_millis()
        ),
        None => String::new(),
    };
    for event in &events {
        match event {
            DomainEvent::StatusChanged { from, at, .. } => myrmic_sdk::info!(
                "status changed refrigerator_id={fridge} incident_id={incident_id} status={status} \
                 previous_status={from} at={}",
                at.as_millis()
            )?,
            DomainEvent::WarningRaised { incident } => myrmic_sdk::warn!(
                "ALERT severity=warning refrigerator_id={fridge} incident_id={} \
                 status={status}{reading}",
                incident.incident_id
            )?,
            DomainEvent::CriticalRaised { incident } => myrmic_sdk::warn!(
                "ALERT severity=critical refrigerator_id={fridge} incident_id={} \
                 status={status}{reading} exposure_ms={} highest_temperature_c={:.3} data_gap={}",
                incident.incident_id,
                incident.exposure.as_millis(),
                incident.highest_temperature_c,
                incident.has_data_gap
            )?,
            DomainEvent::Recovered { incident, .. } => myrmic_sdk::info!(
                "recovered refrigerator_id={fridge} incident_id={} status={status}{reading} \
                 severity={} exposure_ms={}",
                incident.incident_id,
                incident.severity,
                incident.exposure.as_millis()
            )?,
            DomainEvent::SensorStale { since } => myrmic_sdk::warn!(
                "sensor stale refrigerator_id={fridge} incident_id={incident_id} status={status} \
                 since={}",
                since.as_millis()
            )?,
            DomainEvent::SensorResumed { .. } => myrmic_sdk::info!(
                "sensor resumed refrigerator_id={fridge} incident_id={incident_id} \
                 status={status}{reading}"
            )?,
            DomainEvent::ReadingRejected { reason } => myrmic_sdk::warn!(
                "reading rejected refrigerator_id={fridge}{reading} reason={}",
                reason.as_str()
            )?,
        }
    }
    Ok(())
}

/// Runs once per incarnation, before any other handler: on deploy and again
/// on every restart.
#[myrmic_sdk::init]
fn init(md: Metadata, init: MonitorInit) -> Result {
    init.validate()?;
    // Stored state is for one refrigerator; a payload naming another one
    // starts over rather than evaluating its readings against the old state.
    let stored = FRIDGE.load().ok().flatten();
    let resumed = stored.as_ref() == Some(&init.refrigerator_id);
    if !resumed {
        FRIDGE.save(&init.refrigerator_id)?;
        MONITOR.save(&MonitorState::new())?;
    }
    // Timers die with the incarnation: re-arm when a configuration is known.
    // Until the first accepted reading there is none, and nothing to watch.
    let config = if resumed { CONFIG.load()? } else { None };
    if let Some(config) = &config {
        arm_timer(config)?;
    }
    let state = MONITOR.load()?.unwrap_or_default();
    myrmic_sdk::info!(
        "risk-monitor-cell started id={} refrigerator_id={} resumed={resumed} status={} \
         incident_id={} incidents={} config_known={}",
        md.id,
        init.refrigerator_id,
        state.status(),
        state
            .open_incident()
            .map_or("none", |incident| incident.incident_id.as_str()),
        INCIDENTS.count()?,
        config.is_some(),
    )?;
    Ok(())
}

/// A reading the asset cell accepted, with the limits in force.
#[myrmic_sdk::evt]
fn reading_accepted(_md: Metadata, event: Envelope<ReadingAccepted>) -> Result {
    let ReadingAccepted { reading, config } = event.body;
    // Every monitor hears every asset; another refrigerator's reading is
    // routine and not worth a line.
    if FRIDGE.load()?.as_ref() != Some(&config.refrigerator_id) {
        return Ok(());
    }
    CONFIG.save(&config)?;
    arm_timer(&config)?;
    apply(Input::Reading(reading), &config)
}

/// The periodic handler: lets core notice a sensor that has gone quiet.
#[myrmic_sdk::cmd]
fn tick(_md: Metadata) -> Result {
    let Some(config) = CONFIG.load()? else {
        return Ok(());
    };
    apply(Input::Tick { now: now()? }, &config)
}

/// Query: the most recent incidents, newest first.
#[myrmic_sdk::cmd]
fn get_incidents(md: Metadata, payload: Bytes) -> Result {
    let Some(GetIncidents { limit, reply_to }) = decode("get_incidents", &payload)? else {
        return Ok(());
    };
    let rows = INCIDENTS.list_rev()?;
    let truncated = rows.len() > usize::from(limit);
    let mut incidents = Vec::new();
    let mut acknowledgements = Vec::new();
    for row in rows.into_iter().take(usize::from(limit)) {
        if let Some(acknowledged_at) = row.acknowledged_at {
            acknowledgements.push(Acknowledgement {
                incident_id: row.incident.incident_id.clone(),
                acknowledged_at,
            });
        }
        incidents.push(row.incident);
    }
    let refrigerator_id = FRIDGE.load()?.ok_or("refrigerator_id is missing")?;
    let list = Envelope::new(IncidentList {
        refrigerator_id,
        incidents,
        acknowledgements,
        truncated,
        as_of: now()?,
    });
    // A cell gets the answer as a command. A caller without an identity (the
    // Myrmic CLI) cannot be sent anything, so for it the answer is published
    // as an event under the name it asked for.
    let via = if md.sender.is_nil() {
        myrmic_sdk::publish(reply_to.as_str(), &list)?;
        "event"
    } else {
        Callback::to(reply_to.as_str())?.invoke(md.sender, &list)?;
        "command"
    };
    myrmic_sdk::info!(
        "incidents sent refrigerator_id={} count={} truncated={truncated} reply_to={reply_to} \
         via={via}",
        list.body.refrigerator_id,
        list.body.incidents.len()
    )?;
    Ok(())
}

/// Command: a person has seen an incident. Acknowledging twice keeps the
/// first acknowledgement.
#[myrmic_sdk::cmd]
fn acknowledge(_md: Metadata, payload: Bytes) -> Result {
    let Some(Acknowledge { incident_id }) = decode("acknowledge", &payload)? else {
        return Ok(());
    };
    let key = String::from(incident_id.as_str());
    let Some(mut row) = INCIDENTS.get(&key)? else {
        myrmic_sdk::warn!(
            "acknowledge rejected incident_id={incident_id} reason=unknown_incident"
        )?;
        return Ok(());
    };
    let fridge = &row.incident.refrigerator_id;
    if let Some(at) = row.acknowledged_at {
        myrmic_sdk::info!(
            "acknowledge repeated refrigerator_id={fridge} incident_id={incident_id} at={}",
            at.as_millis()
        )?;
        return Ok(());
    }
    let at = now()?;
    row.acknowledged_at = Some(at);
    INCIDENTS.insert_with(&key, &row)?;
    myrmic_sdk::info!(
        "incident acknowledged refrigerator_id={} incident_id={incident_id} severity={} at={}",
        row.incident.refrigerator_id,
        row.incident.severity,
        at.as_millis()
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chillguard_protocol::Kind;
    use myrmic_sdk::Handler;

    use super::*;

    /// Ties the command names the attributes fixed to the protocol. The
    /// `#[evt]` handler has no marker type to check; `make e2e` and the
    /// deployment test are what catch a misnamed one.
    #[test]
    fn wire_names_match_the_protocol() {
        assert_eq!(
            GetIncidents::KIND,
            Kind::Command(<get_incidents as Handler>::NAME)
        );
        assert_eq!(
            Acknowledge::KIND,
            Kind::Command(<acknowledge as Handler>::NAME)
        );
        assert_eq!(ReadingAccepted::KIND, Kind::Event("reading_accepted"));
    }

    #[test]
    fn tick_period_is_at_most_half_the_stale_duration() {
        for stale_ms in [1, 400, 1_000, 300_000, 3_600_000] {
            let stale = Duration::from_millis(stale_ms);
            let period = (stale / 2).max(MIN_TICK);
            assert!(period <= stale / 2 || period == MIN_TICK, "{stale_ms}");
            assert!(period >= MIN_TICK);
        }
        assert_eq!(
            (Duration::from_mins(5) / 2).max(MIN_TICK),
            Duration::from_mins(5) / 2
        );
    }
}
