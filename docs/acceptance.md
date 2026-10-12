# Acceptance: docs/architecture.md section 8, "Required"

Each required item, the code that provides it, and the tests that pin it.
Checked against v0.1.0 on 2026-10-11 with the host test suite (`make test`)
and a full `make e2e` run against Myrmic 0.7.0. Unit tests of the risk
rules are in `crates/chillguard-core/src/risk/tests.rs` unless another file
is named; "e2e" is a scenario of `scripts/e2e.sh`.

| # | Required | Status |
|---|---|---|
| 1 | One simulated refrigerator | met |
| 2 | Timestamped readings at a configurable interval | met |
| 3 | Configurable temperature limits | met |
| 4 | Correct excursion-duration tracking | met, with the documented policy for gaps |
| 5 | Warning and critical alerts with duplicate suppression | met |
| 6 | Recovery event and incident history | met; history is per node, see gaps |
| 7 | CLI or simple dashboard showing state | met (CLI; a web dashboard was added after v0.1.0) |
| 8 | Automated tests for normal, warming, critical, recovery, stale-reading, and restart scenarios | met |
| 9 | README with setup, run, and demo instructions | met |

## 1. One simulated refrigerator

- **Code.** `chillguard_core::simulator` (first-order thermal model, seeded
  noise, three presets); `sensor-cell` plays it on a timer and publishes
  `reading_observed`; `fridge-asset-cell` and `risk-monitor-cell` are
  deployed for one `refrigerator_id` each (`config/*.{prod,dev}.json`).
- **Tests.** Simulator: `same_seed_always_gives_the_same_output`,
  `output_for_a_seed_is_pinned`, `power_failure_warms_monotonically_within_noise_bounds`,
  `recovery_converges_toward_the_setpoint`,
  `cabinet_temperature_does_not_depend_on_the_sampling_interval`,
  `presets_have_the_documented_shape` (`src/simulator.rs`);
  `presets_end_in_the_expected_status_with_the_expected_incidents`
  (`tests/simulator_presets.rs`) plays every preset through the risk rules.
  Live: every e2e scenario runs the cells for one refrigerator.

## 2. Timestamped readings at a configurable interval

- **Code.** `TemperatureReading { refrigerator_id, temperature_c, observed_at, source }`
  (`core::risk::model`); the sensor cell stamps `observed_at` from the
  swarm clock (`myrmic_sdk::now()`) and arms `interval(tick, interval_ms)`
  from its init payload (`SensorConfig.interval_ms`, refused at zero).
- **Tests.** `readings_carry_identity_time_and_source`,
  `time_scale_plays_the_scenario_faster_but_stamps_real_time`
  (`src/simulator.rs`); `wire_names_match_the_protocol` and the golden file
  `reading_observed.json` pin the wire shape; `SensorConfig::validate`
  refuses `interval_ms: 0`. Live: e2e `normal` asserts a reading was
  accepted and in range; the `reading` log line carries `observed_at`.

## 3. Configurable temperature limits

- **Code.** `RefrigeratorConfig { lower_limit_c?, upper_limit_c, critical_after, stale_after, hysteresis_c }`
  with `validate()` (`core::risk::config`); set at deploy time in
  `config/fridge-asset-cell.<profile>.json`, changed at run time with the
  `set_limits` command (`fridge-asset-cell`, `AssetState::set_config`) or
  `chillguard set-limits`; stored limits survive restarts and redeploys.
- **Tests.** `edge_reading_exactly_on_a_limit_is_in_range`,
  `edge_no_lower_limit_means_cold_readings_are_in_range`,
  `invalid_config_is_an_error_for_every_input`;
  `set_config_replaces_validates_and_reports_change` (`src/asset.rs`);
  `limits_are_validated_by_core` (`protocol/tests/malformed.rs`);
  `set_limits_changes_only_what_was_given`,
  `set_limits_refuses_a_result_core_would_refuse` (`cli/tests/args.rs`).
  Live: e2e `normal` changes the upper limit through the CLI, reads it
  back, restores it, and sees an inverted range refused; the notes record
  limits surviving a runtime restart (docs/myrmic-notes.md, section 11).

## 4. Correct excursion-duration tracking

- **Code.** `Incident.exposure` is the time from the first out-of-range
  reading to the latest one that kept the incident open, from reading
  timestamps, never from counts; silences count in full and set
  `has_data_gap` (`core::risk::incident`, `evaluate::on_reading`). The
  policies for gaps, order and clock steps are docs/decisions/0003-risk-policies.md.
- **Tests.** `rule_2_warning_to_critical_uses_reading_timestamps_not_counts`,
  `rule_3_duplicate_suppression_updates_only_peak_and_exposure`,
  `edge_gap_during_excursion_accrues_exposure_and_flags_data_gap`,
  `edge_gap_before_recovery_flags_data_gap_without_adding_exposure`,
  `edge_out_of_order_reading_is_rejected_and_state_unchanged`,
  `edge_duplicate_reading_is_rejected_and_state_unchanged`,
  `edge_clock_backwards_*`; property
  `exposure_never_decreases_within_an_incident` (`tests/properties.rs`);
  scenario `sensor_dropout_mid_excursion` (`tests/scenarios.rs`). Live:
  e2e `excursion` checks `exposure >= critical_after` and no data gap;
  e2e `restart` checks exposure grew by at least the outage and the gap is
  recorded.

## 5. Warning and critical alerts with duplicate suppression

- **Code.** `evaluate` raises `WarningRaised` when an incident opens and
  `CriticalRaised` once when `exposure >= critical_after`; the severity is
  stored in the incident, so a redelivered or replayed reading cannot raise
  it again (`core::risk::evaluate`). The monitor cell persists the state in
  the same transaction as the `alert_raised` it publishes
  (`risk-monitor-cell/src/handlers.rs`, "persist, then emit").
- **Tests.** `rule_1_normal_to_warning_opens_incident_and_raises_warning_once`,
  `rule_2_critical_is_raised_once_per_incident`,
  `edge_zero_critical_duration_raises_warning_and_critical_together`;
  property `each_incident_emits_at_most_one_warning_and_one_critical` and
  `critical_requires_a_prior_out_of_range_reading`; scenario
  `restart_mid_incident_keeps_the_incident_and_its_alert_history`;
  `alert_must_match_its_incident` (protocol). Live: e2e `excursion` and
  `restart` count exactly one `ALERT severity=warning` and one
  `ALERT severity=critical` per scenario, across a runtime restart.

## 6. Recovery event and incident history

- **Code.** `Recovered { incident, at, temperature }` closes the incident
  with `recovered_at` and `recovery_temperature_c` and keeps the highest
  severity reached; optional hysteresis decides what counts as recovered
  (`core::risk`). The monitor cell publishes `incident_recovered`, keeps
  every incident in a table keyed by id, answers `get_incidents` (newest
  first, at most 100) and records `acknowledge`; `chillguard incidents` and
  `chillguard ack` show and use it.
- **Tests.** `rule_4_recovery_closes_incident_and_keeps_peak_severity`,
  `rule_4_next_excursion_after_recovery_is_a_new_incident`,
  `edge_hysteresis_*`, `changed_incident_is_due_after_every_reading_that_touches_it`;
  property `changed_incident_reports_exactly_the_changes`;
  `recovery_must_match_its_incident`, `incident_records_must_be_consistent`,
  `acknowledgements_must_belong_to_listed_incidents` (protocol); render
  snapshots `incidents*`, `ack*` (`cli/tests/render.rs`). Live: e2e
  `excursion` reads one closed incident that kept `critical`, acknowledges
  it, sees a second acknowledgement accepted and an unknown id refused;
  e2e `restart` reads the same incident id before and after the restart.
- **Note.** A recovery is an observation of the temperature; nothing in the
  code or the output says the contents are safe.

## 7. CLI or simple dashboard showing state

- **Code.** `crates/chillguard-cli` (`chillguard status | incidents | ack |
  set-limits | simulate | watch`, text or `--json`), talking to the cells
  through the Myrmic CLI; docs/operations.md documents it.
- **Tests.** Argument parsing (`cli/tests/args.rs`, 11 tests), rendering
  pinned by snapshots (`cli/tests/render.rs`, 13 tests and 24 snapshot
  files), the transport against a fake `myrmic` (`cli/tests/fake_myrmic.rs`,
  7 tests), unit tests of the subscriber's output parser (`src/wire.rs`).
  Live: every e2e assertion that reads state goes through
  `chillguard --json status` and `--json incidents`; `simulate`,
  `set-limits` and `ack` are exercised too.
- **Since v0.1.0.** A web dashboard (`crates/cells/dashboard-cell`, served
  through a Myrmic gateway; docs/decisions/0006-web-dashboard.md). It shows
  state and, in development deployments, starts the simulator's scenarios;
  it does not acknowledge or change limits. Tests: the cell's unit tests
  (init payload, which cell is asked when, what counts as a changed
  answer), and the e2e `dashboard` scenario, which fetches the page and the
  served answers through the gateway and posts a `simulate_scenario` the
  way the page's buttons do. The page's own rendering was checked by hand
  in headless Chromium at phone and desktop widths, not by a test.

## 8. Automated tests for the six scenarios

| Scenario | Host test (deterministic, milliseconds) | Live test (`make e2e`, real runtime) |
|---|---|---|
| normal | `normal_day` (`tests/scenarios.rs`); `presets_end_in_the_expected_status_with_the_expected_incidents` for `normal_day` | `normal`: 15 s of readings, status `normal`, no incidents, no alerts, no rejected readings |
| warming | `power_failure_escalates_to_critical_then_recovers` (warning on first out-of-range reading); `rule_1_...` | `excursion` and `restart`: status reaches `warning`, one warning alert |
| critical | same scenario and `rule_2_...`; `brief_excursion_recovers_before_critical` for the negative case | `excursion`: `critical` after `critical_after`, exposure checked; `restart`: `critical` reached and still `critical` after the restart |
| recovery | same scenario, `rule_4_...`, `sensor_dropout_mid_excursion` | `excursion` and `restart`: status back to `normal`, incident closed with `recovered_at`, severity kept |
| stale reading | `rule_5_any_state_goes_stale_on_late_tick_and_keeps_incident_open`, `rule_5_sensor_that_never_reported_goes_stale`, `rule_6_stale_resumes_to_normal_warning_or_critical`, `edge_tick_never_changes_anything_but_staleness` | `stale`: sensor cell deleted, `sensor_stale` within `stale_after` plus one tick, no alert, `normal` again after the sensor is redeployed |
| restart | `restart_resumes_from_serialized_state` (state round-tripped through Postcard and JSON after every input gives the same run), `restart_mid_incident_keeps_the_incident_and_its_alert_history`; `state_survives_serialisation` (asset); `restart_from_serialized_state_continues_the_same_sequence` (simulator) | `restart`: runtime stopped for `stale_after + 5 s` mid-incident and started again, cells redeployed: same incident id, still open, no second alert, exposure grew by the outage, data gap set, status `critical`, then recovery |

The e2e harness is the only test that covers the cells' wiring, persistence
and timers (docs/decisions/0001-core-vs-cells.md). It is not part of the
default CI run; it is a manual job (`.github/workflows/ci.yml`, job `e2e`).

## 9. README with setup, run, and demo instructions

- `README.md`: what ChillGuard is and is not (safety disclaimer),
  architecture with the diagram, prerequisites, setup, run, the
  power-failure walkthrough, testing, layout, limitations.

## Open points carried into the next phase

- Second refrigerator and second node, web dashboard, physical sensor,
  external notifications, replicated persistence: architecture.md section
  8, "Next phase".
- The gaps in docs/security.md: unauthenticated readings and status
  changes within the swarm, no second factor on `set_limits`, mutual TLS
  not exercised.
- Cells do not come back by themselves after a runtime restart; `make
  deploy` is needed (docs/myrmic-notes.md, section 11).
- The asset's status view is event-fed and can miss a change
  (docs/decisions/0005-state-ownership.md); `chillguard status` shows the
  monitor's incident next to it and a note when they disagree.
