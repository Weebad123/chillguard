//! Typed requests to the cells, on top of [`Myrmic`].

use std::time::{Duration, Instant};

use chillguard_protocol::commands::{GetIncidents, GetStatus, ReplyTo};
use chillguard_protocol::responses::{IncidentList, StatusSnapshot};
use chillguard_protocol::{Envelope, Kind, Payload};

use crate::cli::Settings;
use crate::error::Error;
use crate::myrmic::{Myrmic, Subscription};

/// A query whose answer has not arrived is sent again this often. Queries
/// change nothing, so asking twice is harmless, and it covers an answer
/// published a moment before the subscription took hold.
const RESEND_EVERY: Duration = Duration::from_secs(2);

/// Sends commands and queries to the cells of one refrigerator.
#[derive(Debug, Clone)]
pub struct Client {
    myrmic: Myrmic,
    timeout: Duration,
}

impl Client {
    /// A client for `settings`.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        Self {
            myrmic: Myrmic {
                bin: settings.myrmic.clone(),
                connect: settings.connect.clone(),
                timeout: settings.timeout,
            },
            timeout: settings.timeout,
        }
    }

    /// Sends a command that has no answer.
    ///
    /// # Errors
    ///
    /// [`Error::Protocol`] when `body` is not valid for the wire, otherwise
    /// what [`Myrmic::send`] returns.
    pub fn command<T: Payload>(&self, target: &str, body: T) -> Result<(), Error> {
        let payload = Envelope::new(body).encode()?;
        self.myrmic.send(target, command_name::<T>(), &payload)
    }

    /// Asks the asset cell for its snapshot.
    ///
    /// # Errors
    ///
    /// See [`Client::request`].
    pub fn status(&self, asset: &str) -> Result<StatusSnapshot, Error> {
        self.request(asset, |reply_to| GetStatus { reply_to })
    }

    /// Asks the risk monitor cell for its newest incidents.
    ///
    /// # Errors
    ///
    /// See [`Client::request`].
    pub fn incidents(&self, monitor: &str, limit: u16) -> Result<IncidentList, Error> {
        self.request(monitor, |reply_to| GetIncidents { limit, reply_to })
    }

    /// Sends a query and waits for its answer.
    ///
    /// The answer comes back as an event named by the query's `reply_to`
    /// (see `reply_name` below), so the subscription is started first and the
    /// query sent once it is in place.
    ///
    /// # Errors
    ///
    /// [`Error::NoAnswer`] when nothing arrives within the timeout,
    /// [`Error::BadAnswer`] when what arrives is not a valid `R`, and what
    /// [`Myrmic`] returns when the CLI itself fails.
    pub fn request<Q, R>(&self, target: &str, query: impl Fn(ReplyTo) -> Q) -> Result<R, Error>
    where
        Q: Payload,
        R: Payload,
    {
        let command = command_name::<Q>();
        let name = reply_name(command, target);
        let reply_to = ReplyTo::new(name.clone())
            .map_err(|error| Error::Invalid(format!("internal: reply name: {error}")))?;
        let payload = Envelope::new(query(reply_to)).encode()?;

        let deadline = Instant::now() + self.timeout;
        let mut subscription = self.myrmic.subscribe(&[&name], deadline)?;
        loop {
            self.myrmic.send(target, command, &payload)?;
            let resend_at = (Instant::now() + RESEND_EVERY).min(deadline);
            if let Some(bytes) = answer(&mut subscription, &name, resend_at)? {
                return Envelope::<R>::decode(&bytes)
                    .map(|envelope| envelope.body)
                    .map_err(|source| Error::BadAnswer {
                        target: target.to_string(),
                        command,
                        source,
                    });
            }
            if Instant::now() >= deadline {
                return Err(Error::NoAnswer {
                    target: target.to_string(),
                    command,
                    waited: humantime::format_duration(self.timeout).to_string(),
                });
            }
        }
    }

    /// Starts a subscription to `events`, for `watch`.
    ///
    /// # Errors
    ///
    /// See [`Myrmic::subscribe`].
    pub fn subscribe(&self, events: &[&str]) -> Result<Subscription, Error> {
        self.myrmic.subscribe(events, Instant::now() + self.timeout)
    }
}

/// Waits until `until` for the event called `name`; other events are skipped.
fn answer(
    subscription: &mut Subscription,
    name: &str,
    until: Instant,
) -> Result<Option<Vec<u8>>, Error> {
    while let Some(event) = subscription.next_event(until)? {
        if event.name == name {
            return Ok(Some(event.payload_bytes()));
        }
    }
    Ok(None)
}

/// The wire name of a command payload.
fn command_name<T: Payload>() -> &'static str {
    match T::KIND {
        Kind::Command(name) | Kind::Event(name) => name,
        // Responses are never sent by this client.
        Kind::Response => "response",
    }
}

/// The event name under which `target` is asked to publish its answer to
/// `command`: `cg_reply_<command>_<target>`, in the characters a Myrmic name
/// allows.
///
/// The name is the same for every request of that query to that cell, on
/// purpose. The runtime keeps a metric series per event name
/// (`cell_events_sent{event=...}`), so a name made up per request would add
/// a series per request, without bound. With a stable name, another client
/// asking the same cell the same question at the same moment may see this
/// client's answer and the other way round; both are fresh answers to that
/// question, and each says when it was taken (`as_of`).
fn reply_name(command: &str, target: &str) -> String {
    let target: String = target
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("cg_reply_{command}_{target}")
}

#[cfg(test)]
mod tests {
    use chillguard_protocol::commands::{Acknowledge, SetLimits, SimulateScenario};

    use super::*;

    #[test]
    fn reply_names_are_valid_stable_and_distinct_per_question() {
        let status = reply_name("get_status", "site-a.cold_room/fridge-asset-cell");
        assert_eq!(
            status,
            "cg_reply_get_status_site_a_cold_room_fridge_asset_cell"
        );
        // The same question always gets the same name: one metric series.
        assert_eq!(
            status,
            reply_name("get_status", "site-a.cold_room/fridge-asset-cell")
        );
        let others = [
            reply_name("get_incidents", "site-a.cold_room/fridge-asset-cell"),
            reply_name("get_status", "site-b-fridge-asset-cell"),
            reply_name("get_status", "fridge-asset-cell"),
        ];
        for name in &others {
            assert_ne!(*name, status);
        }
        for name in others.iter().chain([&status]) {
            assert!(ReplyTo::new(name.clone()).is_ok(), "{name}");
        }
    }

    #[test]
    fn command_names_are_the_wire_names() {
        assert_eq!(command_name::<GetStatus>(), "get_status");
        assert_eq!(command_name::<GetIncidents>(), "get_incidents");
        assert_eq!(command_name::<SetLimits>(), "set_limits");
        assert_eq!(command_name::<SimulateScenario>(), "simulate_scenario");
        assert_eq!(command_name::<Acknowledge>(), "acknowledge");
    }
}
