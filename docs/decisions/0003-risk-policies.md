# 0003: Risk evaluation policies

Status: accepted (2026-10-09)

## Context

`chillguard_core::risk::evaluate` is the state machine of docs/architecture.md
section 6: it takes the monitor state, one input (a reading or a tick) and the
refrigerator's configuration, and returns the next state and the events to
emit. Section 6 names the transitions. It leaves open what happens when the
input is not a clean, ordered, gap-free stream, and section 5 asks for exactly
that to be decided and written down.

Three facts about the runtime bear on the answers (docs/myrmic-notes.md):

- A command whose handler returns an error is delivered again, without a
  retry bound. An event whose handler fails is dropped.
- A cell reads its persisted state through an 8 KiB buffer, so state that
  grows without bound eventually cannot be read back.
- Readings can reach a handler twice, and nothing guarantees their order.

## Decision

Each policy names the test that pins it. Unit tests are in
`crates/chillguard-core/src/risk/tests.rs`.

### 1. A bad reading is an event, never an error

A reading that cannot be used leaves the state exactly as it was and yields a
single `ReadingRejected { reason }`. `evaluate` returns `Err(RiskError)` only
for an invalid configuration (non-finite or inverted limits, a negative or
too-wide hysteresis, a zero `stale_after`).

Why: an error would make a cell's command handler fail, and the runtime would
redeliver the same bad reading forever. A rejected reading also does not count
as a sign of life: a sensor that only sends unusable readings goes stale.

Tests: `invalid_config_is_an_error_for_every_input`, and the property test
`rejected_readings_leave_the_state_unchanged`.

### 2. Out-of-order and duplicate readings are rejected

A reading is accepted only if `observed_at` is later than the last accepted
reading. An equal timestamp is rejected as `Duplicate` (the first reading for
a timestamp stands, whatever the second one says); an earlier one as
`OutOfOrder`.

Why: exposure is computed from reading timestamps and must never run
backwards, and the state keeps only the last reading, so an old reading has
nowhere to be inserted. This also makes a redelivered reading harmless.

Cost: what a late reading would have told us is lost. A late out-of-range
reading does not move the start of an incident back.

Tests: `edge_out_of_order_reading_is_rejected_and_state_unchanged`,
`edge_duplicate_reading_is_rejected_and_state_unchanged`.

### 3. Time never moves backwards in the state

The timestamp of the last accepted reading is a high-water mark. If the
sensor's clock steps back, its readings are rejected as `OutOfOrder` until its
clock passes the mark again. A tick earlier than the last reading counts as
zero elapsed time and changes nothing.

Known limitation: for the length of a backwards step the monitor is blind. It
rejects every reading, and ticks from the same stepped-back clock cannot
declare the sensor stale either. One reading with a timestamp far in the
future has the same effect until real time catches up. The only signal is the
stream of `ReadingRejected` events, so a cell must log them. Core does not
try to detect this; revisit when readings come from a device with its own
clock.

Tests: `edge_clock_backwards_tick_is_ignored`,
`edge_clock_backwards_readings_are_rejected_until_time_catches_up`.

### 4. A reading for another refrigerator is rejected

`reading.refrigerator_id` must equal `config.refrigerator_id`; otherwise the
reading is rejected as `WrongRefrigerator { expected, actual }`.

Test: `edge_wrong_refrigerator_reading_is_rejected_and_state_unchanged`.

### 5. A non-finite temperature is rejected

NaN and the infinities are rejected as `NonFiniteTemperature`. NaN compares
false against every limit, so it would otherwise read as "in range" and close
an open incident.

Test: `edge_non_finite_temperature_is_rejected_and_state_unchanged`.

### 6. Exposure accrues across gaps, and the incident says so

`exposure` is the time from the incident's first out-of-range reading to the
latest reading that kept it open. Silence between two such readings counts in
full: the refrigerator is assumed to have stayed out of range. Escalation to
critical uses the same number, so an incident can become critical on the first
reading after a long silence.

`has_data_gap` is set when two consecutive readings of an incident are more
than `stale_after` apart. That includes the reading that ends the incident.
The gap is judged from reading timestamps alone, whether or not a tick
happened to notice the silence.

The time between the last out-of-range reading and the recovering reading is
**not** added to `exposure`, and a recovering reading never escalates. When
the sensor falls silent during a warning and comes back in range, the incident
closes as a warning with `has_data_gap` set. The upper bound of the exposure
is `recovered_at - started_at`; both are on the record.

Why not count up to the recovering reading: with regular readings it would
turn an excursion last seen out of range at minute 20 into a critical one
because the in-range reading arrived at minute 30. Critical is only raised on
evidence, which is an out-of-range reading at or past the critical duration.

A silence *before* the first out-of-range reading is not part of the
incident. It shows up as `SensorStale` and `SensorResumed`.

Tests: `edge_gap_during_excursion_accrues_exposure_and_flags_data_gap`,
`edge_gap_before_recovery_flags_data_gap_without_adding_exposure`, scenario
`sensor_dropout_mid_excursion`.

### 7. Only a tick declares the sensor stale, and only a reading escalates

The sensor is stale when a tick arrives *more than* `stale_after` after the
last accepted reading (exactly `stale_after` is not stale). Before any reading
has arrived, the first tick starts the count, so a sensor that never reports
goes stale too. Staleness is announced once and leaves an open incident
untouched. A tick never escalates an incident: passing the critical duration
in silence is not evidence, and the next reading settles it (policy 6).

Tests: `rule_5_any_state_goes_stale_on_late_tick_and_keeps_incident_open`,
`rule_5_sensor_that_never_reported_goes_stale`,
`edge_tick_never_changes_anything_but_staleness`.

### 8. Limits are inclusive

The safe range is `lower_limit_c..=upper_limit_c`: a reading exactly on a
limit is in range. The critical duration is inclusive the other way:
an excursion of exactly `critical_after` is critical. A `critical_after` of
zero makes every incident critical from its first reading; the warning and the
critical alert are then emitted together.

Tests: `edge_reading_exactly_on_a_limit_is_in_range`,
`rule_2_warning_to_critical_uses_reading_timestamps_not_counts`,
`edge_zero_critical_duration_raises_warning_and_critical_together`.

### 9. Flapping: optional hysteresis, off by default

`hysteresis_c` defaults to zero. With it off, every crossing of a limit is
its own incident with its own warning and recovery. With it set, an incident
still opens on the plain limits but only ends on a reading at least
`hysteresis_c` inside them. Readings in between keep the incident open and
its exposure running.

Tests: `edge_flapping_without_hysteresis_opens_an_incident_per_crossing`,
`edge_hysteresis_keeps_one_incident_open_while_flapping`,
`edge_hysteresis_does_not_change_when_an_incident_opens`.

### 10. Shape of the state

- The status is derived from the open incident and a stale flag, not stored,
  so a state cannot say "warning" without an incident.
- The state holds the last reading and at most one open incident. A closed
  incident leaves the state inside its `Recovered` event; whoever handles the
  event keeps the history. The state therefore stays far below the 8 KiB read
  limit.
- An incident's identifier is `<refrigerator_id>-<started_at in ms>`:
  replaying the same readings gives the same identifier, and a monitor that
  lost its state cannot reuse an old one.
- An incident keeps the highest severity it reached. A recovered critical
  incident is still a critical incident.
- Besides the fields in docs/architecture.md section 5, an incident carries
  `lowest_temperature_c` (the peak of a cold excursion), `exposure` and
  `has_data_gap`.
- The configuration is validated and applied on every call. A changed limit or
  duration takes effect with the next input; it never lowers a severity.
- Events of one call are ordered: `StatusChanged` first, then `SensorStale` or
  `SensorResumed`, then warning, critical, recovered.

Restart: the whole state is serde data without format-specific attributes.
Scenario `restart_resumes_from_serialized_state` saves and reloads it after
every input, through Postcard and through JSON, and gets the same states and
events as an uninterrupted run. Time the process was down is a silence like
any other and falls under policies 6 and 7.

## Consequences

- Duplicate suppression, ordering and idempotency are properties of core and
  are tested there. A cell only has to persist the state it gets back.
- A cell must drive `Input::Tick` from a timer. Without ticks no sensor is
  ever reported stale.
- A cell must persist the incident carried by `Recovered`; core does not keep
  it.
- A cell must surface `ReadingRejected` events. They are the only trace of
  the situations in policies 2 to 5.
- `evaluate` fails only on an invalid configuration. A cell should validate
  the configuration when it receives it, so that this can never fail a
  reading handler that the runtime would then retry without end.
- `exposure` can understate the true excursion by the interval before the
  recovering reading (policy 6). Anyone reporting exposure to a person should
  show `has_data_gap` next to it.

## Open points

- Detecting a sensor clock that jumped (policy 3) is not solved.
- Core now defines `TemperatureReading` and the event types. Whether
  `chillguard-protocol` reuses them or converts at the cell boundary is still
  the open point of decision 0001.
