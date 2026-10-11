# Changelog

All notable changes to ChillGuard are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/). Until 1.0.0 a minor version may
change the wire contract; the contract's own version is `SCHEMA_VERSION` in
`chillguard-protocol` (docs/decisions/0004-wire-contract.md).

## [Unreleased]

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
