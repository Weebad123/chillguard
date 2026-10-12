# Changelog

All notable changes to ChillGuard are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/). Until 1.0.0 a minor version may
change the wire contract; the contract's own version is `SCHEMA_VERSION` in
`chillguard-protocol` (docs/decisions/0004-wire-contract.md).

## [Unreleased]

### Added

- `dashboard-cell` and a web dashboard at `/chillguard/`, served through a
  Myrmic gateway: a card per refrigerator (temperature, status, exposure,
  active alert), the incident history, and in development deployments
  buttons that send `simulate_scenario`. One HTML file, no dependencies, no
  build step, laid out for a phone. The cell asks the asset cell and the
  risk monitor on a timer and stores their answers as files for the page to
  poll; it owns no state and changes nothing
  (docs/decisions/0006-web-dashboard.md).
- `chillguard start` and `chillguard stop` (`make start`, `make stop`): a
  playground for trying ChillGuard on one machine. `start` runs a Myrmic
  runtime of its own, in memory and off the network, deploys the four cells
  with the `dev` payloads, waits until the dashboard shows the refrigerator
  and opens `http://localhost:8642/chillguard/` in the browser; a second
  `start` finds it running and puts back any cell that went missing; `stop`
  removes it. docs/playground/README.md is the guide.
- `config/runtime.dashboard.yml`: the loopback-only runtime plus an embedded
  gateway on port 8080. `config/dashboard-cell.{prod,dev}.json`: refresh
  every 30 s and view-only under `prod`, every 2 s with the scenario buttons
  under `dev`.
- `playground` scenario in `scripts/e2e.sh`: `chillguard start`, the page,
  the rest of the CLI through the endpoint it prints, a second `start`,
  `stop`.
- `dashboard` scenario in `scripts/e2e.sh`: the route responds, the page and
  the cells' answers are served, the message API is absent without
  `dev_controls`, and with it a posted `simulate_scenario` reaches the
  sensor and the served status follows.
- docs/myrmic-notes.md, section 16: what was verified about the gateway,
  the blob store and queries between cells; section 17: what the playground
  relies on, and what a suspended machine does to a runtime's cells.

### Changed

- `make build-cells`, `make deploy` and `make e2e` now include the dashboard
  cell. A deployment that does not want it names the other three:
  `scripts/deploy.sh sensor-cell fridge-asset-cell risk-monitor-cell`.
- The staleness guard of `scripts/deploy.sh` and `scripts/e2e.sh` also
  counts a cell's `assets/` as sources.

### Fixed

- README: `short_door_open` under the `dev` profile raises a warning only;
  it said a warning and a critical.

### Security

- The playground's dashboard has the scenario buttons on and its port is
  reachable from the network; what is exposed is the playground's simulated
  refrigerator and nothing else. `chillguard start` prints that.
- The gateway listens on every interface and has no login. A `prod`
  dashboard serves files only. `dev_controls: true` mounts the gateway's
  message API, through which an HTTP client can send any command to any
  cell; it is off by default (docs/security.md, "the gateway").

## [0.1.0] - 2026-10-11

The MVP of docs/architecture.md section 8: one simulated refrigerator on one
Myrmic node, monitored end to end, with an operator CLI. Built and tested
against Myrmic 0.7.0.

### Added

- `chillguard-core`: the risk state machine (`normal`, `warning`,
  `critical`, `sensor_stale`), excursion exposure from reading timestamps,
  escalation after a configurable duration, recovery with optional
  hysteresis, stale-sensor detection from ticks, duplicate and out-of-order
  rejection, and a restart-safe serialisable state. Policies for every edge
  are in docs/decisions/0003-risk-policies.md.
- `chillguard-core::simulator`: a deterministic first-order thermal model
  with seeded noise and three presets (`normal_day`, `power_failure_2h`,
  `short_door_open`), playable at a compressed clock.
- `chillguard-core::asset`: the refrigerator's configuration, latest
  accepted reading and mirrored status view, with the same acceptance rule
  the monitor applies.
- `chillguard-protocol`: the versioned envelope and every event, command and
  response the cells exchange, validated on encode and decode, with golden
  files pinning the bytes.
- Three Myrmic cells: `sensor-cell` (simulated readings on a timer),
  `fridge-asset-cell` (owns configuration and the latest reading, answers
  `get_status`, applies `set_limits`) and `risk-monitor-cell` (evaluates,
  publishes `status_changed`, `alert_raised`, `incident_recovered`, keeps
  the incident table, answers `get_incidents`, records `acknowledge`).
- `chillguard`, the operator CLI: `status`, `incidents`, `ack`,
  `set-limits`, `simulate`, `watch`, text and `--json` output, driving the
  cells through the Myrmic CLI.
- `scripts/e2e.sh`: four end-to-end scenarios (normal, excursion, stale,
  restart) on a throwaway loopback-only runtime, driven and checked through
  the operator CLI.
- Deployment profiles in `config/`: `prod` (default; real-time clock,
  development commands refused) and `dev` (60x clock, `simulate` accepted);
  `config/runtime.local.yml` keeps a runtime off the network.
- CI (`.github/workflows/ci.yml`): formatting, clippy with warnings as
  errors, unit and integration tests, rustdoc, the Wasm build of every cell
  with a checksum-verified Myrmic CLI, cargo-deny for licences and
  advisories, and line coverage of `chillguard-core` enforced at 90 % or
  better. The end-to-end harness is a manual job.
- Documentation: README, docs/security.md (threat model and access limits),
  docs/operations.md, docs/acceptance.md (section 8 criteria mapped to code
  and tests), five decision records and the verified Myrmic notes.

### Security

- Development-only commands are off unless a cell is deployed with
  `dev_mode: true`; a compressed sensor clock needs it too.
- Every inbound message is checked for schema version, shape and content;
  command handlers decode their own payloads so a bad command is logged
  once instead of retried forever.
- `unwrap`, `expect`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`,
  indexing and `process::exit` are denied outside tests; `unsafe_code` is
  denied everywhere but the cells' handler exports.
- The CLI's duration arguments are bounded (at most a year) so no deadline
  arithmetic can overflow, and writes to a closed stderr are not fatal.

[Unreleased]: https://github.com/Weebad123/chillguard/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Weebad123/chillguard/releases/tag/v0.1.0
