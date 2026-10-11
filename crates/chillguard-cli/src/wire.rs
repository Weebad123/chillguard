//! The output format of `myrmic subscribe`.
//!
//! For every event the subscriber prints, on stdout, an empty line, a header
//!
//! ```text
//! [2026-10-10T09:19:51.954Z] event=<name> sender=<sri> payload=<n> bytes
//! ```
//!
//! and the payload as pretty-printed JSON over the following lines. (Its
//! keys come out sorted; that is the CLI's display, not the wire order.)
//! [`EventParser`] turns those lines back into events.

use serde_json::Value;

/// One event, as read from the subscriber.
#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedEvent {
    /// The event's name.
    pub name: String,
    /// The SRI of the cell that published it.
    pub sender: String,
    /// The payload.
    pub payload: Value,
}

impl ReceivedEvent {
    /// The payload as bytes, for `Envelope::decode`.
    #[must_use]
    pub fn payload_bytes(&self) -> Vec<u8> {
        self.payload.to_string().into_bytes()
    }
}

/// Reassembles events from the subscriber's stdout, one line at a time.
#[derive(Debug, Default)]
pub struct EventParser {
    /// The event whose payload lines are being collected.
    pending: Option<Pending>,
}

#[derive(Debug)]
struct Pending {
    name: String,
    sender: String,
    payload: String,
}

impl EventParser {
    /// A parser that has seen nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one line; returns the event that this line completed, if any.
    ///
    /// A header starts a new event and drops one whose payload never became
    /// valid JSON. Lines before the first header are ignored.
    pub fn push_line(&mut self, line: &str) -> Option<ReceivedEvent> {
        if let Some((name, sender)) = parse_header(line) {
            self.pending = Some(Pending {
                name,
                sender,
                payload: String::new(),
            });
            return None;
        }
        let pending = self.pending.as_mut()?;
        pending.payload.push_str(line);
        pending.payload.push('\n');
        let payload = serde_json::from_str(&pending.payload).ok()?;
        let Pending { name, sender, .. } = self.pending.take()?;
        Some(ReceivedEvent {
            name,
            sender,
            payload,
        })
    }
}

/// Reads `[<time>] event=<name> sender=<sri> payload=<n> bytes`.
fn parse_header(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix('[')?;
    let (_time, rest) = rest.split_once("] ")?;
    let mut fields = rest.split(' ');
    let name = fields.next()?.strip_prefix("event=")?;
    let sender = fields.next()?.strip_prefix("sender=")?;
    fields.next()?.strip_prefix("payload=")?;
    if fields.next() != Some("bytes") || fields.next().is_some() || name.is_empty() {
        return None;
    }
    Some((name.to_string(), sender.to_string()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Captured from `myrmic subscribe` 0.7.0.
    const CAPTURED: &str = r#"
[2026-10-10T09:19:51.954Z] event=cg_probe_1 sender=6033b371-07fe-5fc5-9f03-9a18a386682b payload=330 bytes
{
  "body": {
    "as_of": 1791623991952,
    "last_reading": {
      "observed_at": 1791623991101,
      "temperature_c": 4.079
    },
    "status": "normal"
  },
  "v": 1
}

[2026-10-10T09:19:52.954Z] event=other sender=00000000-0000-0000-0000-000000000000 payload=3 bytes
2.5

[2026-10-10T09:19:53.001Z] event=text sender=00000000-0000-0000-0000-000000000000 payload=4 bytes
"hi"
"#;

    fn parse_all(text: &str) -> Vec<ReceivedEvent> {
        let mut parser = EventParser::new();
        text.lines()
            .filter_map(|line| parser.push_line(line))
            .collect()
    }

    #[test]
    fn captured_output_parses_into_events() {
        let events = parse_all(CAPTURED);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].name, "cg_probe_1");
        assert_eq!(events[0].sender, "6033b371-07fe-5fc5-9f03-9a18a386682b");
        assert_eq!(events[0].payload["v"], json!(1));
        assert_eq!(events[0].payload["body"]["status"], json!("normal"));
        assert_eq!(
            events[0].payload["body"]["last_reading"]["temperature_c"],
            json!(4.079)
        );
        // Scalars arrive on one line.
        assert_eq!(events[1].payload, json!(2.5));
        assert_eq!(events[2].payload, json!("hi"));
    }

    #[test]
    fn payload_bytes_are_json_the_protocol_can_decode() {
        let events = parse_all(CAPTURED);
        let back: Value = serde_json::from_slice(&events[0].payload_bytes()).expect("json");
        assert_eq!(back, events[0].payload);
    }

    #[test]
    fn lines_before_the_first_header_are_ignored() {
        let events = parse_all(
            "INFO  subscribed to: x\n{\"a\":1}\n\n[t] event=x sender=s payload=1 bytes\n1\n",
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload, json!(1));
    }

    #[test]
    fn an_event_cut_short_is_dropped_when_the_next_header_arrives() {
        let text = "[t] event=a sender=s payload=9 bytes\n{\n  \"x\": 1,\n[t] event=b sender=s payload=4 bytes\ntrue\n";
        let events = parse_all(text);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "b");
    }

    #[test]
    fn lookalike_lines_are_not_headers() {
        for line in [
            "[2026] event= sender=s payload=1 bytes",
            "[2026] evt=a sender=s payload=1 bytes",
            "[2026] event=a sender=s payload=1 bytes more",
            "event=a sender=s payload=1 bytes",
            "[2026] event=a payload=1 bytes",
        ] {
            assert_eq!(parse_header(line), None, "{line}");
        }
    }
}
