//! The commands: what each one sends, waits for and prints.

use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

use chillguard_core::risk::{IncidentId, RefrigeratorConfig};
use chillguard_protocol::commands::{
    Acknowledge, MAX_INCIDENT_LIMIT, SetLimits as SetLimitsCommand, SimulateScenario,
};
use chillguard_protocol::events::{AlertRaised, IncidentRecovered, ReadingAccepted, StatusChanged};
use chillguard_protocol::{Envelope, Kind, Payload};
use serde_json::Value;

use crate::cli::{Command, SetLimits, Settings};
use crate::client::Client;
use crate::error::Error;
use crate::playground;
use crate::render::{self, AckOutcome, OpenIncident, StatusReport, WatchEvent};
use crate::wire::ReceivedEvent;

/// How often a command's effect is looked for after it was sent.
const POLL_EVERY: Duration = Duration::from_millis(300);

/// Runs one command and writes its output to `out`.
///
/// # Errors
///
/// Returns what went wrong, in words meant for the operator.
pub fn run(command: &Command, settings: &Settings, out: &mut dyn Write) -> Result<(), Error> {
    let client = Client::new(settings);
    let json = settings.json;
    match command {
        Command::Status => {
            let report = status_report(&client, settings)?;
            emit(
                out,
                json,
                &render::status_json(&report),
                &render::status(&report),
            )
        }
        Command::Incidents { limit } => {
            let list = client.incidents(&settings.monitor(), *limit)?;
            emit(
                out,
                json,
                &render::incidents_json(&list),
                &render::incidents(&list),
            )
        }
        Command::Ack { incident_id } => {
            let outcome = acknowledge(&client, settings, incident_id)?;
            emit(
                out,
                json,
                &render::ack_json(incident_id, outcome),
                &render::ack(incident_id, outcome),
            )
        }
        Command::SetLimits(changes) => {
            let (previous, now) = set_limits(&client, settings, changes)?;
            emit(
                out,
                json,
                &render::limits_change_json(&previous, &now),
                &render::limits_change(&previous, &now),
            )
        }
        Command::Simulate { preset, seed } => {
            let target = settings.sensor();
            client.command(
                &target,
                SimulateScenario {
                    preset: *preset,
                    seed: *seed,
                },
            )?;
            emit(
                out,
                json,
                &render::simulate_json(&target, *preset, *seed),
                &render::simulate(&target, *preset, *seed),
            )
        }
        Command::Watch { readings, duration } => {
            watch(&client, settings, *readings, *duration, out)
        }
        Command::Start(args) => {
            // Starting takes several seconds; the steps go to stderr so
            // that stdout holds the result alone. A closed stderr must not
            // stop the start.
            let mut progress = |step: &str| {
                let _ = writeln!(std::io::stderr(), "{step}...");
            };
            let playground = playground::start(args, settings, &mut progress)?;
            emit(
                out,
                json,
                &render::playground_json(&playground),
                &render::playground(&playground),
            )?;
            if !args.no_open && !playground::open_browser(&playground.url) {
                let _ = writeln!(
                    std::io::stderr(),
                    "note: no browser could be opened from here; open the address yourself"
                );
            }
            Ok(())
        }
        Command::Stop { name } => {
            let was_running = playground::stop(name, settings)?;
            emit(
                out,
                json,
                &render::stopped_json(name, was_running),
                &render::stopped(name, was_running),
            )
        }
    }
}

fn emit(out: &mut dyn Write, json: bool, value: &Value, text: &str) -> Result<(), Error> {
    if json {
        writeln!(out, "{value:#}")?;
    } else {
        writeln!(out, "{text}")?;
    }
    Ok(())
}

/// The asset's snapshot plus the monitor's open incident. The snapshot is
/// required; a monitor that does not answer is reported, not fatal.
fn status_report(client: &Client, settings: &Settings) -> Result<StatusReport, Error> {
    let snapshot = client.status(&settings.asset())?;
    let incident = match client.incidents(&settings.monitor(), 1) {
        Err(error) => OpenIncident::Unknown(error.to_string()),
        Ok(list) => match list.incidents.first() {
            Some(incident) if incident.is_open() => OpenIncident::Open {
                incident: incident.clone(),
                acknowledged_at: list.acknowledged_at(&incident.incident_id),
            },
            _ => OpenIncident::None,
        },
    };
    Ok(StatusReport { snapshot, incident })
}

/// Acknowledges an incident and waits until the monitor reports it. The
/// `acknowledge` command has no answer, so the incident list is the proof.
fn acknowledge(
    client: &Client,
    settings: &Settings,
    incident_id: &str,
) -> Result<AckOutcome, Error> {
    let monitor = settings.monitor();
    let id = IncidentId::new(incident_id);
    let list = client.incidents(&monitor, MAX_INCIDENT_LIMIT)?;
    if !list
        .incidents
        .iter()
        .any(|incident| incident.incident_id == id)
    {
        return Err(Error::Invalid(format!(
            "{} has no incident `{incident_id}` (looked at its newest {}); \
             see `chillguard incidents`",
            list.refrigerator_id,
            list.incidents.len()
        )));
    }
    if let Some(at) = list.acknowledged_at(&id) {
        return Ok(AckOutcome::AlreadyAcknowledged(at));
    }
    client.command(
        &monitor,
        Acknowledge {
            incident_id: id.clone(),
        },
    )?;
    let deadline = Instant::now() + settings.timeout;
    loop {
        thread::sleep(POLL_EVERY);
        let list = client.incidents(&monitor, MAX_INCIDENT_LIMIT)?;
        if let Some(at) = list.acknowledged_at(&id) {
            return Ok(AckOutcome::Acknowledged(at));
        }
        if Instant::now() >= deadline {
            return Err(Error::Invalid(format!(
                "the acknowledgement of `{incident_id}` was sent, but `{monitor}` does not report \
                 it after {}",
                render::span(settings.timeout)
            )));
        }
    }
}

/// The configuration that results from applying `changes` to `current`.
///
/// # Errors
///
/// [`Error::Invalid`] with core's reason when the result is not a valid
/// configuration; nothing is sent in that case.
pub fn merge_limits(
    current: &RefrigeratorConfig,
    changes: &SetLimits,
) -> Result<RefrigeratorConfig, Error> {
    let mut config = current.clone();
    if changes.no_lower {
        config.lower_limit_c = None;
    } else if let Some(lower) = changes.lower {
        config.lower_limit_c = Some(lower);
    }
    if let Some(upper) = changes.upper {
        config.upper_limit_c = upper;
    }
    if let Some(critical_after) = changes.critical_after {
        config.critical_after = critical_after;
    }
    if let Some(stale_after) = changes.stale_after {
        config.stale_after = stale_after;
    }
    if let Some(hysteresis) = changes.hysteresis {
        config.hysteresis_c = hysteresis;
    }
    config
        .validate()
        .map_err(|error| Error::Invalid(format!("these limits are not valid: {error}")))?;
    Ok(config)
}

/// Reads the limits in force, applies the changes, sends the result and
/// waits until the asset cell reports it. Returns the limits before and now.
fn set_limits(
    client: &Client,
    settings: &Settings,
    changes: &SetLimits,
) -> Result<(RefrigeratorConfig, RefrigeratorConfig), Error> {
    let asset = settings.asset();
    let previous = client.status(&asset)?.config;
    let wanted = merge_limits(&previous, changes)?;
    if wanted == previous {
        return Ok((previous, wanted));
    }
    client.command(
        &asset,
        SetLimitsCommand {
            config: wanted.clone(),
        },
    )?;
    let deadline = Instant::now() + settings.timeout;
    loop {
        thread::sleep(POLL_EVERY);
        if client.status(&asset)?.config == wanted {
            return Ok((previous, wanted));
        }
        if Instant::now() >= deadline {
            return Err(Error::Invalid(format!(
                "the new limits were sent, but `{asset}` still reports the previous ones after {}; \
                 its log says why (`set_limits rejected`)",
                render::span(settings.timeout)
            )));
        }
    }
}

/// Prints the refrigerator's state once, then its events as they arrive.
fn watch(
    client: &Client,
    settings: &Settings,
    readings: bool,
    duration: Option<Duration>,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let snapshot = client.status(&settings.asset())?;
    let mut names = vec![
        event_name::<StatusChanged>(),
        event_name::<AlertRaised>(),
        event_name::<IncidentRecovered>(),
    ];
    if readings {
        names.push(event_name::<ReadingAccepted>());
    }
    let mut subscription = client.subscribe(&names)?;
    if !settings.json {
        writeln!(out, "{}", render::watch_header(&snapshot))?;
        out.flush()?;
    }
    let end = duration.map(|duration| Instant::now() + duration);
    loop {
        // Without an end the wait is only a pause between checks.
        let until = end.unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
        match subscription.next_event(until)? {
            Some(received) => {
                let Some(event) = decode(&received) else {
                    continue;
                };
                // Events travel the whole network; only this refrigerator's count.
                if *event.refrigerator_id() != snapshot.refrigerator_id {
                    continue;
                }
                if settings.json {
                    writeln!(out, "{}", render::watch_event_json(&event))?;
                } else {
                    writeln!(out, "{}", render::watch_event(&event))?;
                }
                out.flush()?;
            }
            None if end.is_some() => return Ok(()),
            None => {}
        }
    }
}

/// Reads a received event as one of the events `watch` shows. An event that
/// does not decode is reported on stderr and skipped.
fn decode(received: &ReceivedEvent) -> Option<WatchEvent> {
    fn body<T: Payload>(received: &ReceivedEvent) -> Option<T> {
        match Envelope::<T>::decode(&received.payload_bytes()) {
            Ok(envelope) => Some(envelope.body),
            Err(error) => {
                // A closed stderr must not stop the watch; the line is dropped.
                let _ = writeln!(
                    std::io::stderr(),
                    "warning: skipped a `{}` event: {error}",
                    received.name
                );
                None
            }
        }
    }
    let name = received.name.as_str();
    if name == event_name::<StatusChanged>() {
        body(received).map(WatchEvent::Status)
    } else if name == event_name::<AlertRaised>() {
        body(received).map(WatchEvent::Alert)
    } else if name == event_name::<IncidentRecovered>() {
        body(received).map(WatchEvent::Recovered)
    } else if name == event_name::<ReadingAccepted>() {
        body(received).map(WatchEvent::Reading)
    } else {
        None
    }
}

fn event_name<T: Payload>() -> &'static str {
    match T::KIND {
        Kind::Event(name) | Kind::Command(name) => name,
        Kind::Response => "response",
    }
}
