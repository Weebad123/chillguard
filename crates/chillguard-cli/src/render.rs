//! What the operator sees: text and JSON for every command.
//!
//! Everything here is a pure function from protocol types to output, so the
//! exact text is pinned by snapshot tests.

use std::time::{Duration, UNIX_EPOCH};

use chillguard_core::risk::{Incident, RefrigeratorConfig, RefrigeratorId, Status, Timestamp};
use chillguard_protocol::commands::ScenarioPreset;
use chillguard_protocol::events::{AlertRaised, IncidentRecovered, ReadingAccepted, StatusChanged};
use chillguard_protocol::responses::{IncidentList, StatusSnapshot};
use chillguard_protocol::{Kind, Payload, Severity};
use serde_json::{Value, json};

use crate::cli::DEFAULT_PLAYGROUND;
use crate::playground::Playground;

/// What `status` shows: the asset cell's snapshot and, from the risk monitor
/// cell, the incident in progress.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusReport {
    /// The asset cell's answer.
    pub snapshot: StatusSnapshot,
    /// What the risk monitor cell said about an open incident.
    pub incident: OpenIncident,
}

/// The incident in progress, as far as it could be found out.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenIncident {
    /// The monitor reports no open incident.
    None,
    /// The monitor reports this incident as open.
    Open {
        /// The incident.
        incident: Incident,
        /// When it was acknowledged, if it was.
        acknowledged_at: Option<Timestamp>,
    },
    /// The monitor could not be asked; the text says why.
    Unknown(String),
}

/// How an `ack` ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckOutcome {
    /// This call acknowledged the incident.
    Acknowledged(Timestamp),
    /// The incident had been acknowledged before.
    AlreadyAcknowledged(Timestamp),
}

/// One line of `watch`.
#[derive(Debug, Clone, PartialEq)]
pub enum WatchEvent {
    /// The status changed.
    Status(StatusChanged),
    /// An alert was raised.
    Alert(AlertRaised),
    /// An incident recovered.
    Recovered(IncidentRecovered),
    /// A reading was accepted.
    Reading(ReadingAccepted),
}

impl WatchEvent {
    /// The refrigerator the event is about.
    #[must_use]
    pub const fn refrigerator_id(&self) -> &RefrigeratorId {
        match self {
            Self::Status(event) => &event.refrigerator_id,
            Self::Alert(event) => &event.incident.refrigerator_id,
            Self::Recovered(event) => &event.incident.refrigerator_id,
            Self::Reading(event) => &event.reading.refrigerator_id,
        }
    }
}

// ---------------------------------------------------------------------------
// Building blocks
// ---------------------------------------------------------------------------

/// A timestamp as UTC, to the second: `2026-10-10T02:37:04Z`.
#[must_use]
pub fn time(at: Timestamp) -> String {
    let at = UNIX_EPOCH + Duration::from_millis(at.as_millis());
    humantime::format_rfc3339_seconds(at).to_string()
}

/// A duration to the second (`11m 54s`), or to the millisecond below one.
#[must_use]
pub fn span(duration: Duration) -> String {
    let rounded = if duration.as_secs() > 0 {
        Duration::from_secs(duration.as_secs())
    } else {
        Duration::from_millis(duration.subsec_millis().into())
    };
    humantime::format_duration(rounded).to_string()
}

fn celsius(temperature_c: f64) -> String {
    format!("{temperature_c:.1} °C")
}

fn range(incident: &Incident) -> String {
    format!(
        "{:.1} to {}",
        incident.lowest_temperature_c,
        celsius(incident.highest_temperature_c)
    )
}

fn limits(config: &RefrigeratorConfig) -> String {
    let range = match config.lower_limit_c {
        Some(lower) => format!("{lower:.1} to {}", celsius(config.upper_limit_c)),
        None => format!("up to {}", celsius(config.upper_limit_c)),
    };
    if config.hysteresis_c > 0.0 {
        format!("{range}, hysteresis {}", celsius(config.hysteresis_c))
    } else {
        range
    }
}

fn lower_limit(config: &RefrigeratorConfig) -> String {
    config
        .lower_limit_c
        .map_or_else(|| "none".to_string(), celsius)
}

/// Left-aligned columns, two spaces apart, no trailing spaces.
fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: &mut dyn Iterator<Item = &str>| {
        let mut text = String::new();
        for (cell, width) in cells.zip(&widths) {
            text.push_str(cell);
            // `width` is the widest cell of the column, so this cannot go
            // below two; saturating keeps that a fact and not an assumption.
            let padding = width.saturating_sub(cell.chars().count()).saturating_add(2);
            text.extend(std::iter::repeat_n(' ', padding));
        }
        text.trim_end().to_string()
    };
    let mut lines = vec![line(&mut headers.iter().copied())];
    lines.extend(
        rows.iter()
            .map(|row| line(&mut row.iter().map(String::as_str))),
    );
    lines.join("\n")
}

/// `label  value` lines with the values aligned.
fn fields(rows: &[(&str, String)]) -> String {
    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    rows.iter()
        .map(|(label, value)| format!("{label:<width$}  {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn to_json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

/// `status`, as text.
#[must_use]
pub fn status(report: &StatusReport) -> String {
    let snapshot = &report.snapshot;
    let config = &snapshot.config;
    let last_reading = snapshot.last_reading.map_or_else(
        || "none yet".to_string(),
        |last| {
            format!(
                "{} at {} ({} before this snapshot)",
                celsius(last.temperature_c),
                time(last.observed_at),
                span(snapshot.as_of.saturating_duration_since(last.observed_at))
            )
        },
    );
    let incident = match &report.incident {
        OpenIncident::None => "none".to_string(),
        OpenIncident::Unknown(why) => format!("unknown ({why})"),
        OpenIncident::Open {
            incident,
            acknowledged_at,
        } => format!(
            "{}: {}, open since {}, out of range for {}, {}{}, {}",
            incident.incident_id,
            incident.severity,
            time(incident.started_at),
            span(incident.exposure),
            range(incident),
            if incident.has_data_gap {
                ", data gap"
            } else {
                ""
            },
            acknowledged_at.map_or_else(
                || "not acknowledged".to_string(),
                |at| format!("acknowledged at {}", time(at))
            ),
        ),
    };
    let mut rows = vec![
        ("refrigerator", snapshot.refrigerator_id.to_string()),
        ("status", snapshot.status.to_string()),
        ("last reading", last_reading),
        ("limits", limits(config)),
        (
            "critical",
            format!("after {} out of range", span(config.critical_after)),
        ),
        (
            "stale",
            format!("after {} without a reading", span(config.stale_after)),
        ),
        ("incident", incident),
    ];
    if let Some(note) = disagreement(report) {
        rows.push(("note", note.to_string()));
    }
    rows.push(("snapshot", time(snapshot.as_of)));
    fields(&rows)
}

/// The status is the asset cell's view, fed by events that can be missed;
/// the incident comes from the monitor, which decides. When the two cannot
/// both be right, say so.
fn disagreement(report: &StatusReport) -> Option<&'static str> {
    let in_incident = matches!(report.snapshot.status, Status::Warning | Status::Critical);
    match &report.incident {
        OpenIncident::Open { .. } if report.snapshot.status == Status::Normal => Some(
            "the monitor has an open incident but the status shown is normal; \
             the asset's status view may have missed a change",
        ),
        OpenIncident::None if in_incident => Some(
            "the status shown is an incident status but the monitor has no open incident; \
             the asset's status view may have missed a change",
        ),
        _ => None,
    }
}

/// `status --json`.
#[must_use]
pub fn status_json(report: &StatusReport) -> Value {
    let (open_incident, acknowledged_at, monitor_error) = match &report.incident {
        OpenIncident::None => (Value::Null, Value::Null, Value::Null),
        OpenIncident::Open {
            incident,
            acknowledged_at,
        } => (to_json(incident), to_json(acknowledged_at), Value::Null),
        OpenIncident::Unknown(why) => (Value::Null, Value::Null, json!(why)),
    };
    json!({
        "snapshot": to_json(&report.snapshot),
        "open_incident": open_incident,
        "acknowledged_at": acknowledged_at,
        "monitor_error": monitor_error,
    })
}

// ---------------------------------------------------------------------------
// incidents
// ---------------------------------------------------------------------------

/// `incidents`, as text.
#[must_use]
pub fn incidents(list: &IncidentList) -> String {
    let fridge = &list.refrigerator_id;
    let as_of = time(list.as_of);
    if list.incidents.is_empty() {
        return format!("no incidents for {fridge} as of {as_of}");
    }
    let rows: Vec<Vec<String>> = list
        .incidents
        .iter()
        .map(|incident| {
            vec![
                incident.incident_id.to_string(),
                incident.severity.to_string(),
                time(incident.started_at),
                incident
                    .recovered_at
                    .map_or_else(|| "open".to_string(), time),
                span(incident.exposure),
                range(incident),
                if incident.has_data_gap { "yes" } else { "no" }.to_string(),
                list.acknowledged_at(&incident.incident_id)
                    .map_or_else(|| "no".to_string(), time),
            ]
        })
        .collect();
    let headers = [
        "INCIDENT",
        "SEVERITY",
        "STARTED",
        "RECOVERED",
        "EXPOSURE",
        "RANGE",
        "GAP",
        "ACKNOWLEDGED",
    ];
    let count = list.incidents.len();
    let more = if list.truncated {
        "; older ones exist, raise --limit to see them"
    } else {
        ""
    };
    format!(
        "{}\n\n{count} incident{} for {fridge} as of {as_of}{more}",
        table(&headers, &rows),
        if count == 1 { "" } else { "s" }
    )
}

/// `incidents --json`: the incident list as the monitor sent it.
#[must_use]
pub fn incidents_json(list: &IncidentList) -> Value {
    to_json(list)
}

// ---------------------------------------------------------------------------
// ack, set-limits, simulate
// ---------------------------------------------------------------------------

/// `ack`, as text.
#[must_use]
pub fn ack(incident_id: &str, outcome: AckOutcome) -> String {
    match outcome {
        AckOutcome::Acknowledged(at) => {
            format!("incident {incident_id} acknowledged at {}", time(at))
        }
        AckOutcome::AlreadyAcknowledged(at) => format!(
            "incident {incident_id} was already acknowledged at {}; nothing sent",
            time(at)
        ),
    }
}

/// `ack --json`.
#[must_use]
pub fn ack_json(incident_id: &str, outcome: AckOutcome) -> Value {
    let (at, already) = match outcome {
        AckOutcome::Acknowledged(at) => (at, false),
        AckOutcome::AlreadyAcknowledged(at) => (at, true),
    };
    json!({
        "incident_id": incident_id,
        "acknowledged_at": at.as_millis(),
        "already_acknowledged": already,
    })
}

/// `set-limits`, as text: every setting before and after, changed ones
/// marked.
#[must_use]
pub fn limits_change(previous: &RefrigeratorConfig, now: &RefrigeratorConfig) -> String {
    let fridge = &now.refrigerator_id;
    if previous == now {
        return format!(
            "limits for {fridge} are already as requested; nothing sent\n\n{}",
            limits_table(previous, now)
        );
    }
    format!(
        "limits changed for {fridge}\n\n{}",
        limits_table(previous, now)
    )
}

fn limits_table(previous: &RefrigeratorConfig, now: &RefrigeratorConfig) -> String {
    let settings = [
        ("lower limit", lower_limit(previous), lower_limit(now)),
        (
            "upper limit",
            celsius(previous.upper_limit_c),
            celsius(now.upper_limit_c),
        ),
        (
            "critical after",
            span(previous.critical_after),
            span(now.critical_after),
        ),
        (
            "stale after",
            span(previous.stale_after),
            span(now.stale_after),
        ),
        (
            "hysteresis",
            celsius(previous.hysteresis_c),
            celsius(now.hysteresis_c),
        ),
    ];
    let rows: Vec<Vec<String>> = settings
        .into_iter()
        .map(|(name, before, after)| {
            let changed = if before == after { "" } else { "changed" };
            vec![name.to_string(), before, after, changed.to_string()]
        })
        .collect();
    table(&["SETTING", "BEFORE", "NOW", ""], &rows)
}

/// `set-limits --json`.
#[must_use]
pub fn limits_change_json(previous: &RefrigeratorConfig, now: &RefrigeratorConfig) -> Value {
    json!({
        "changed": previous != now,
        "previous": to_json(previous),
        "config": to_json(now),
    })
}

/// `simulate`, as text.
#[must_use]
pub fn simulate(target: &str, preset: ScenarioPreset, seed: u64) -> String {
    format!(
        "sent scenario {preset} (seed {seed}) to {target}\n\
         The sensor only plays it when its dev_mode is on, and does not answer; \
         its log says `scenario started` or `simulate_scenario rejected`."
    )
}

/// `simulate --json`.
#[must_use]
pub fn simulate_json(target: &str, preset: ScenarioPreset, seed: u64) -> Value {
    json!({ "sent": true, "target": target, "preset": preset.as_str(), "seed": seed })
}

// ---------------------------------------------------------------------------
// start, stop
// ---------------------------------------------------------------------------

/// `start`, as text: where the dashboard is, what to do next, how to stop.
#[must_use]
pub fn playground(playground: &Playground) -> String {
    let Playground {
        runtime,
        url,
        connect,
        already_running,
    } = playground;
    let headline = if *already_running {
        "The ChillGuard playground was already running."
    } else {
        "The ChillGuard playground is running."
    };
    let stop = if runtime == DEFAULT_PLAYGROUND {
        "chillguard stop".to_string()
    } else {
        format!("chillguard stop --name {runtime}")
    };
    format!(
        "{headline}\n\
         \n\
         \x20 dashboard  {url}\n\
         \x20 runtime    {runtime} (in memory: nothing is kept once it stops)\n\
         \x20 endpoint   {connect}\n\
         \n\
         On the page, press a button under \"Development controls\" and watch the card.\n\
         From a terminal, the same refrigerator:\n\
         \n\
         \x20 export MYRMIC_CONNECT={connect}\n\
         \x20 chillguard status\n\
         \x20 chillguard simulate power_failure_2h\n\
         \n\
         Stop it with `{stop}`. The guide is docs/playground/README.md.\n\
         \n\
         Note: the dashboard's port is open to other machines on this network, and its\n\
         buttons work without a login. It is a simulated refrigerator; stop it when done."
    )
}

/// `start --json`.
#[must_use]
pub fn playground_json(playground: &Playground) -> Value {
    json!({
        "running": true,
        "already_running": playground.already_running,
        "url": playground.url,
        "runtime": playground.runtime,
        "connect": playground.connect,
    })
}

/// `stop`, as text.
#[must_use]
pub fn stopped(runtime: &str, was_running: bool) -> String {
    if was_running {
        format!(
            "The ChillGuard playground has stopped (runtime {runtime}). \
             What it held was in memory and is gone."
        )
    } else {
        format!("No ChillGuard playground was running (runtime {runtime}).")
    }
}

/// `stop --json`.
#[must_use]
pub fn stopped_json(runtime: &str, was_running: bool) -> Value {
    json!({ "running": false, "was_running": was_running, "runtime": runtime })
}

// ---------------------------------------------------------------------------
// watch
// ---------------------------------------------------------------------------

/// The line `watch` starts with.
#[must_use]
pub fn watch_header(snapshot: &StatusSnapshot) -> String {
    let reading = snapshot.last_reading.map_or_else(
        || "no reading yet".to_string(),
        |last| format!("last reading {}", celsius(last.temperature_c)),
    );
    format!(
        "watching {}: status {}, {reading}, limits {} (Ctrl-C to stop)",
        snapshot.refrigerator_id,
        snapshot.status,
        limits(&snapshot.config)
    )
}

/// One event of `watch`, as text.
#[must_use]
pub fn watch_event(event: &WatchEvent) -> String {
    let (at, kind, text) = match event {
        WatchEvent::Status(change) => (
            change.at,
            "status",
            format!("{} -> {}", change.from, change.to),
        ),
        WatchEvent::Alert(alert) => {
            let incident = &alert.incident;
            let at =
                Timestamp::from_millis(incident.started_at.as_millis().saturating_add(
                    u64::try_from(incident.exposure.as_millis()).unwrap_or(u64::MAX),
                ));
            let text = match alert.severity {
                Severity::Warning => format!(
                    "warning   {}  out of range at {}",
                    incident.incident_id,
                    celsius(incident.initial_temperature_c)
                ),
                Severity::Critical => format!(
                    "critical  {}  out of range for {}, {}{}",
                    incident.incident_id,
                    span(incident.exposure),
                    range(incident),
                    if incident.has_data_gap {
                        ", data gap"
                    } else {
                        ""
                    }
                ),
            };
            (at, "ALERT", text)
        }
        WatchEvent::Recovered(recovered) => (
            recovered.at,
            "recovered",
            format!(
                "{}  was {}, out of range for {}, now {}",
                recovered.incident.incident_id,
                recovered.incident.severity,
                span(recovered.incident.exposure),
                celsius(recovered.temperature_c)
            ),
        ),
        WatchEvent::Reading(accepted) => (
            accepted.reading.observed_at,
            "reading",
            celsius(accepted.reading.temperature_c),
        ),
    };
    format!("{}  {kind:<9}  {text}", time(at))
}

/// One event of `watch --json`: `{"event": <wire name>, "body": {...}}`.
#[must_use]
pub fn watch_event_json(event: &WatchEvent) -> Value {
    let (kind, body) = match event {
        WatchEvent::Status(body) => (StatusChanged::KIND, to_json(body)),
        WatchEvent::Alert(body) => (AlertRaised::KIND, to_json(body)),
        WatchEvent::Recovered(body) => (IncidentRecovered::KIND, to_json(body)),
        WatchEvent::Reading(body) => (ReadingAccepted::KIND, to_json(body)),
    };
    let name = match kind {
        Kind::Event(name) | Kind::Command(name) => name,
        Kind::Response => "response",
    };
    json!({ "event": name, "body": body })
}
