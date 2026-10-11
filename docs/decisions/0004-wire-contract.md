# 0004: Wire contract between cells

Status: accepted (2026-10-09)

## Context

The three cells exchange messages through the Myrmic runtime. What the
runtime offers, and what it does when things go wrong, is recorded in
docs/myrmic-notes.md and shapes the contract:

- Two kinds of message. An **event** is published under a name and delivered
  to every cell subscribed at that moment; if a handler fails, the event is
  dropped. A **command** is sent to one cell under a name; if the handler
  fails, the command is redelivered without bound.
- Commands are fire-and-forget. A reply is a command the responder sends back
  to the caller, under a name the caller chose (`Callback`). A `Callback` on
  the wire is the bare command name, not JSON, so it cannot sit inside a JSON
  payload.
- Payload types get their encoding by implementing the SDK's `Encoder` and
  `Decoder`. `#[derive(myrmic_sdk::Message)]` does that by delegating to the
  SDK's `Json` codec: compact `serde_json`, no wrapper. Bare scalars and
  `Vec<u8>` have their own encodings; `Vec<T>` of anything else has none.
- When a payload fails to decode into a handler's parameter type, the handler
  never runs and the error, a `&'static str`, is logged. An event is then
  dropped. A command is redelivered every five seconds without bound, like
  any other failed command (observed; docs/myrmic-notes.md section 10).
- Event and command names are ASCII letters, digits and underscores.

Decision 0001 puts these types in `chillguard-protocol`, which depends on
`myrmic-sdk` for the traits and on `chillguard-core` for the domain types.

## Decision

### Events and commands

| Kind | Message | Name | Published or handled by |
|---|---|---|---|
| event | `ReadingObserved { reading }` | `reading_observed` | sensor cell publishes; asset cell subscribes |
| event | `ReadingAccepted { reading, config }` | `reading_accepted` | asset cell publishes each reading it accepts, with the config in force; monitor cell subscribes (0005) |
| event | `StatusChanged { refrigerator_id, from, to, at }` | `status_changed` | monitor cell publishes; asset cell subscribes for its status view |
| event | `AlertRaised { severity, incident }` | `alert_raised` | monitor cell publishes, once per severity per incident |
| event | `IncidentRecovered { incident, at, temperature_c }` | `incident_recovered` | monitor cell publishes |
| command | `GetStatus { reply_to }` | `get_status` | asset cell answers with `StatusSnapshot`; `open_incident` is always absent in its answer (0005) |
| command | `GetIncidents { limit, reply_to }` | `get_incidents` | monitor cell answers with `IncidentList` (it owns the incidents, 0005) |
| command | `SetLimits { config }` | `set_limits` | asset cell applies |
| command | `SimulateScenario { preset, seed }` | `simulate_scenario` | sensor cell applies, development only |
| command | `Acknowledge { incident_id }` | `acknowledge` | monitor cell records |
| response | `StatusSnapshot { refrigerator_id, status, last_reading, open_incident, config, as_of }` | the caller's `reply_to` | |
| response | `IncidentList { refrigerator_id, incidents, acknowledgements, truncated, as_of }` | the caller's `reply_to` | |

Facts that happened are events: anyone may listen, and a listener that was
not there does not change what happened. Requests for work are commands: they
have one owner and they are retried until done, so every command is safe to
apply twice (`SetLimits` with the same config, `Acknowledge` of an
acknowledged incident, `SimulateScenario` restarting the same scenario).

Stale and resumed sensors have no event of their own: the `StatusChanged`
they cause carries the information. A rejected reading is logged by the cell
that rejected it, not published. `events::MonitorEvent::from_domain` is the
one place that maps core's `DomainEvent` to these.

### Envelope and schema version

Every message is `{"v": <u16>, "body": {...}}`. `v` is `SCHEMA_VERSION`,
currently `1`. Version 1 is the contract as first deployed; changes made
before that, such as moving durations to milliseconds and correcting the
preset name `power_failure_2h` (first written as `power_failure2h`), did not
bump it. The
envelope is one generic type, `Envelope<T>`, and the body types implement a
`Payload` trait that names their kind, their wire name, and their content
checks.

`Envelope<T>` implements the SDK's `Encoder` and `Decoder` by hand, calling
the same `Json` codec the derive would. The derive cannot be used because it
adds no bounds for a generic type, and because decoding does more than the
codec: it checks the version, then the shape, then the content, and reports
which one failed.

### Compatibility policy

A receiver accepts exactly `SCHEMA_VERSION`. Any other `v` is refused as
`UnsupportedVersion` before the body is read. All cells are built from one
workspace and deployed together, so a version mismatch is a deployment fault
and must fail loudly, not be papered over.

Changes that **do not** bump the version:

- Adding a field with a serde default (`#[serde(default)]` or `Option`).
  Receivers ignore unknown fields, so an older receiver skips it and a newer
  receiver fills the default when it is absent.
- Adding a new message type.
- Tightening or loosening a content check.

Changes that **do** bump the version:

- Removing or renaming a field, or changing its type or its JSON shape.
- Adding, removing or renaming an enum variant (`Status`, `Severity`,
  `ReadingSource`, `ScenarioPreset`): the variant names are on the wire.
- Changing a wire name.
- Changing the meaning of a field without changing its shape.

Bumping the version is one constant, and then the golden files are
regenerated (`UPDATE_GOLDEN=1 cargo test -p chillguard-protocol --test
golden`) and the diff reviewed. The golden test is the alarm: any change to
the bytes of any message fails it.

Supporting more than one version at a time (for a rolling upgrade) is not
done. If it becomes necessary, the probe already reads `v` first, so a
decoder per version can be chosen from it.

### Encoding rules

- JSON, compact, through `myrmic_sdk::Json`. Field order is declaration
  order. Numbers are written as `serde_json` writes them (`5.0`, not `5`).
- Core's types travel as core serialises them: `Timestamp` as an integer of
  milliseconds, identifiers as strings, enums as `snake_case` strings, and
  `Duration` as an integer of milliseconds
  (`chillguard_core::duration_millis`). A duration with a part smaller than a
  millisecond fails to encode; it is never rounded.
- No bare scalars and no raw bytes: every message is an object inside the
  envelope, so a handler can tell a wrong message from a right one.
- NaN and infinity have no JSON form; `serde_json` writes `null` for them.
  Encoding refuses a non-finite temperature before that can happen, and a
  `null` where a number belongs fails the shape check on the way in.

### Validation

Decoding checks the content after the shape: identifiers are not empty,
temperatures are finite, a status change changes the status, an alert's
severity matches the incident it carries and that incident is open, a
recovery event agrees with its incident, incident records are internally
consistent, `SetLimits` and the config in a snapshot pass core's
`RefrigeratorConfig::validate`, `reply_to` is a valid Myrmic command name
(checked by the SDK's own validator), and a `GetIncidents` limit is within
`1..=MAX_INCIDENT_LIMIT` (100). Encoding runs the same checks, so a cell
cannot put on the wire what no cell would accept.

A content failure is `ProtocolError::Invalid(ValidationError)`. Through the
SDK's `Decoder` every error becomes a fixed string the host logs.

### Development-only commands

`SimulateScenario` is marked `Payload::DEV_ONLY`. The protocol cannot enforce
the gate: the sensor cell must carry a flag in its configuration and refuse
the command when the flag is off. Rejecting it with an error would make the
runtime redeliver it forever, so the cell logs and returns `Ok`. The sensor
cell does this with `dev_mode` in its init payload.

## Consequences

- A cell publishes `Envelope::new(body)`, and an event handler takes
  `Envelope<T>` as its payload parameter. A **command** handler takes
  `myrmic_sdk::Bytes` and calls `Envelope::decode` itself: a payload that is
  malformed, of another version or invalid can never succeed, so the handler
  logs it and returns `Ok`. Letting the SDK decode it would leave the command
  in a retry loop for good.
- A query carries `reply_to` as a string field. For a caller that is a
  cell, the responding cell builds `Callback::to(reply_to.as_str())` and
  invokes it on `md.sender`: the response arrives as a command. A caller
  without an identity cannot be sent anything, and that is what every
  command from the Myrmic CLI is (nil sender). For it the responding cell
  publishes the response as an **event** named `reply_to`, and the caller
  subscribes to that name before it sends the query. This is how the
  operator CLI (`crates/chillguard-cli`) gets its answers.
- A `reply_to` used this way must come from a small fixed set, not be made
  up per request: the runtime keeps a metric series per event name. The
  operator CLI uses `cg_reply_<command>_<cell name>`. Anyone subscribed to
  that name sees the answer, as with any event.
- The dashboard, when it exists, decodes the same envelopes; the golden files
  are its reference.
- Durations on the wire are integers of milliseconds under core's field
  names: `"critical_after": 1800000`. The asset cell's init payload,
  `config/fridge-asset-cell.dev.json`, carries the same `RefrigeratorConfig`
  in the same form.
- Core gives `IncidentId` no public constructor. A cell receives one from the
  wire or from an `Incident`; nothing else can forge one, which is intended,
  but it means a test must deserialise one to build an `Acknowledge`.

## Open points

- The init payloads of the cells (`--init`) are not part of this contract.
  Each cell defines its own (`config.rs` in the cell crate) and refuses
  unknown keys; the examples are in `config/`.
- Paging of `GetIncidents` beyond the newest `limit` is not supported.
- Acknowledgements are reported next to the incidents, not inside them:
  `IncidentList.acknowledgements` lists `{incident_id, acknowledged_at}` for
  the listed incidents that have one. `Incident` stays core's type, and the
  field was added with a default, so it did not bump the version.
