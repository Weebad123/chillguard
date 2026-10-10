# 0005: State ownership between the asset cell and the risk monitor cell

Status: **accepted** (2026-10-09): three cells, with the asset cell as the
entry point for readings. See "Decision" below. The options that were weighed
are kept as written, including the recommendation that was not taken.

## Context

docs/architecture.md draws `Asset ⇄ Risk Monitor` with one two-way arrow
labelled "State / reading updates". Read literally, either cell may update
what the other holds, which gives some state two writers. Decision 0001
requires that each piece of state has exactly one cell that writes it.

What the runtime gives us to work with (docs/myrmic-notes.md; **[local]**
marks what was observed on this machine):

1. A cell's private state is scoped to that cell. Another cell cannot write
   it. Public scopes exist but are shared and unprotected.
2. Inside one handler, storage writes and outbound messages commit together
   when the handler returns `Ok`. There is no such guarantee across cells.
3. An **event** is delivered to the cells that are subscribed when it is
   published, and is dropped if a subscriber's handler fails. It is never
   sent again.
4. A **command** is redelivered until its handler returns `Ok`: every five
   seconds, without bound **[local]**.
5. One handler runs at a time per cell.
6. Delivery of an event from one cell to an `#[evt]` handler in another cell
   had not been observed when the options were written. It has been since
   (section 11 of the notes).

And what core needs: `risk::evaluate(state, input, config)` takes, for every
reading, the monitor state and the refrigerator's configuration. The monitor
state contains the last accepted reading (it is how out-of-order readings,
gaps and a stale sensor are detected, decision 0003) and the open incident.

## Option A: the split as proposed

| State | Writer | Who else needs it |
|---|---|---|
| Config: limits, durations, hysteresis | Asset | Monitor, for every evaluation |
| Latest reading, `last_reading_at` | Asset | Monitor, for ordering, gaps, staleness |
| Status, `out_of_range_since`, exposure, open incident | Monitor | Asset, to show status |
| Incident history | Monitor | |
| Status view (read model) | Asset, from `status_changed` events | |

The two-way arrow becomes one-way flows, each with one writer at its end:

```
Sensor ──event reading_observed──► Asset      (stores the latest reading)
Sensor ──event reading_observed──► Monitor    (evaluates)
Asset  ──config────────────────► Monitor    (Monitor's working copy)
Monitor ─event status_changed───► Asset      (read model)
```

Each store has one writer. The problems are in what the stores hold:

1. **The latest reading exists twice.** The monitor cannot evaluate without
   the last accepted reading, so it stores one whatever this table says. The
   asset's copy is the same fact, written by another cell from the same
   event. The two can differ: the event is dropped per subscriber when a
   handler fails (fact 3), and to agree on which readings count (late,
   duplicate, NaN, wrong refrigerator) the asset would have to apply core's
   acceptance rules too.
2. **The config exists twice.** The monitor evaluates with its copy. The copy
   must arrive by command, not by event, or one dropped event leaves the
   monitor on old limits for good. It also needs a revision number, so that a
   redelivered older config cannot replace a newer one.
3. **The status read model can be wrong without anyone noticing.**
   `status_changed` is an event. If the asset's handler fails once, or the
   asset is being redeployed at that moment, that change is gone (fact 3).
   The asset then shows the previous status until the next change, and a
   refrigerator that went critical and stays critical produces no next
   change. Showing "normal" for a critical refrigerator is the failure this
   product exists to prevent.
4. **No snapshot is consistent.** The `StatusSnapshot` of the wire contract
   (0004) carries status, last reading, open incident and config. Under this
   split no cell owns all four, and they are committed in two cells at
   different moments (fact 2).

After this split the asset cell is the authority on one thing the monitor
does not also hold: accepting `SetLimits`.

## Option A′: three cells, made safe

If three cells are kept, four changes remove the problems above:

1. The monitor is the authority on the last *accepted* reading. The asset
   keeps a latest reading only as a display copy, or not at all.
2. The asset forwards config to the monitor as a **command** carrying a
   revision number.
3. The status reaches the asset by **command** as well, or `GetStatus` is
   answered by the monitor and the read model is dropped.
4. `GetIncidents` and `Acknowledge` are handled by the monitor, which owns
   the incidents. (0004 currently names the asset cell for both.)

## Option B: one refrigerator cell

Merge the asset cell and the risk monitor cell. One cell per refrigerator
holds the config, the monitor state and the incident history. The sensor
cell stays separate.

- One writer for everything, by construction. No second copy of the reading
  or the config, no read model, nothing to forward.
- A reading is evaluated, the state saved and the events published in one
  handler: one atomic commit (fact 2). A snapshot is always consistent.
- It is the shape core already has. `evaluate` wants the config, the last
  reading and the incident together; the cell decodes, calls it, persists,
  emits.
- `GetStatus`, `GetIncidents`, `SetLimits` and `Acknowledge` have one
  addressee.

What it costs:

- It departs from the three-cell diagram. The separation the diagram is
  after, state on one side and rules on the other, already exists in the
  code as cell versus core (0001); it would no longer also be a process
  boundary.
- One cell is one queue (fact 5): a query waits behind a reading. With one
  reading a second and handlers that take milliseconds this does not matter;
  it would for a long history query.
- The monitor can no longer be placed or replaced separately from the asset
  state. State survives delete-and-redeploy under the same name **[local]**,
  so replacing the merged cell does not lose it.
- There is less cell-to-cell messaging to show. Sensor to refrigerator and
  refrigerator to dashboard remain.

## Trade-offs side by side

| | A (as proposed) | A′ | B |
|---|---|---|---|
| One writer per store | yes | yes | yes |
| One copy of each fact | no (reading, config) | config twice | yes |
| Status shown can go stale silently | yes | no | no |
| Consistent snapshot | no | from the monitor only | yes |
| New messages beyond 0004 | config forward | config forward with revision, status push | none |
| Cells per refrigerator | 3 | 3 | 2 |
| Matches the architecture diagram | yes | yes | no |
| Depends on cross-cell `#[evt]` (fact 6) | sensor→asset, sensor→monitor, monitor→asset | sensor→monitor | sensor→refrigerator |

## Recommendation

**Option B.** The rules need the config, the last reading and the incident
in one place and updated together, and the runtime only offers "together"
inside one cell. Splitting them across two cells buys a boundary that then
has to be bridged with copies and messages, each of which is a way to be
wrong.

If three cells are a requirement (for the demonstration, or the brief),
choose **A′**. Option A as written should not be built: its status read
model fails silently (problem 3).

## Decision

Three cells are kept. The split is the one of option A, with one change to
the flow of readings and two of the four safeguards of option A′.

| State | Writer |
|---|---|
| Config: limits, durations, hysteresis | Asset |
| Latest accepted reading | Asset |
| Status, exposure, open incident, incident history | Monitor |
| Status view | Asset, mirrored from `status_changed` |

```
Sensor  ──event reading_observed──► Asset     accepts or rejects, stores the latest
Asset   ──event reading_accepted──► Monitor   the reading plus the config in force
Monitor ──event status_changed────► Asset     status view
```

**The asset is the only cell that hears the sensor.** It applies core's
acceptance rule (`risk::check_reading`: right refrigerator, finite value,
later than the last one) and forwards what it accepts. The monitor evaluates
what the asset forwards. `risk::evaluate` applies the same function, so the
two cannot disagree on which readings count; a test in core feeds one stream
to both and compares.

**Accepted readings are forwarded as an event, `reading_accepted`.** A
command was the alternative, and what the runtime does with commands rules it
out for a stream (notes, sections 10 and 11): with the monitor not deployed
the send fails in the asset's own handler, which would stop the asset storing
readings; with the monitor failing, every reading is redelivered every five
seconds and a backlog of stale readings builds up. A lost event costs one
reading, the next arrives a second later, and core already has a policy for
gaps (0003).

**The config travels inside every `reading_accepted`.** The monitor judges a
reading by the limits the asset held when it accepted it, and keeps no config
of its own that could fall behind. This replaces "config by command, with a
revision number" (A′, point 2): there is no second copy to keep in step, and
`SetLimits` takes effect with the next reading. The monitor will need the
last config it saw for one thing, the stale-sensor timer.

**The status view stays an event-fed read model**, as decided. Problem 3 of
option A therefore stands: a `status_changed` that the asset misses is not
sent again. Two things limit the damage, neither removes it:

- The view refuses a change older than the one it shows, so late delivery
  cannot move it backwards.
- When a change arrives that does not start from the status the view shows,
  the asset applies it and logs `status view was behind`. A miss becomes
  visible at the *next* change. A refrigerator that goes critical and stays
  critical still shows its previous status until then.

**The asset's `StatusSnapshot` never carries an incident.** `open_incident`
is `None` whatever the status, because `status_changed` does not carry one
and the asset does not own incidents. `GetIncidents` and `Acknowledge` are
the monitor's to handle (A′, point 4).

**A restart must not undo `SetLimits`.** A runtime restart releases the
cells; they are deployed again and `#[init]` runs with its payload again
(notes, section 11). The asset's init therefore uses its payload only when it
holds no state for that refrigerator, and otherwise keeps what is stored.

**The monitor keeps the last configuration it saw**, for one purpose: its
tick. A stale sensor is only ever noticed by a tick, evaluated against
`stale_after`, and a tick carries no configuration of its own. The tick
period is `stale_after / 2` (at least 250 ms), re-armed when a reading
brings a different `stale_after`. Until the first accepted reading there is
no configuration and no tick; a sensor that never reports is noticed only
once something has been accepted.

**Incidents live in the monitor's table store**, one row per incident id,
written in the same handler as the event that announces the change. The
row also holds the acknowledgement (`acknowledged_at`), which the wire
contract does not carry yet: `IncidentList` returns incidents without it.

Still open: whether the status should reach the asset by command after all,
or `GetStatus` should move to the monitor (A′, point 3). The run of
2026-10-10 lost no `status_changed`; the question is what happens when one
is lost, and that has not been staged.

## What confirming each option would change

| | A′ | B |
|---|---|---|
| Crates | `fridge-asset-cell` and `risk-monitor-cell` both implemented | the two become one `refrigerator-cell`; `CELLS` in the Makefile and `scripts/cells.sh` shrink to two |
| Wire contract (0004) | `GetIncidents`, `Acknowledge` move to the monitor; one or two new commands between asset and monitor | the "handled by" column names the refrigerator cell; no new messages |
| docs/architecture.md | arrow replaced by the one-way flows | diagram redrawn with two cells |
| `chillguard-core` | none | none |

**Events are network-wide.** Every asset cell on the network hears every
sensor, and every monitor hears every asset. The cells reject what is not
for their refrigerator, so the identifier is the only thing keeping two
refrigerators apart; two deployments that name the same one will feed each
other (seen with the e2e harness, docs/myrmic-notes.md section 13).

## Not settled by this record

- The reply path of a query, a callback invoked on the caller, has not run:
  every `get_status` so far came from the CLI, which has no cell to reply to.
- Where incident history lives. It cannot be a single `State` value (reads go
  through an 8 KiB buffer); a `Table` keyed by incident id is the candidate.
- A public storage scope would let the monitor read the asset's config
  without a message. It is unprotected and has not been exercised here, so
  it is not proposed.
