# Operating ChillGuard

How to run the operator CLI, read what the cells log, and look at the
runtime's telemetry. Everything here was run against Myrmic 0.7.0 on
2026-10-10; where the Myrmic observability guide
(<https://book.myrmic.dev/docs/guides/observability>) and what was observed
differ, this says so.

Contents: [the operator CLI](#the-operator-cli) ·
[the playground](#the-playground) ·
[the web dashboard](#the-web-dashboard) ·
[what is running](#what-is-running) · [logs](#logs) ·
[log fields](#log-fields) · [traces](#traces) · [metrics](#metrics) ·
[live stream](#live-stream) · [retention and verbosity](#retention-and-verbosity) ·
[when something looks wrong](#when-something-looks-wrong)

## The operator CLI

`chillguard` is a native program (`crates/chillguard-cli`). Build it with
`make cli`; the binary is `target/debug/chillguard`.

It talks to the cells by running the Myrmic CLI, so `myrmic` must be
installed and able to reach the runtime. A command is a `myrmic send`. A
query is a `myrmic send` whose answer the cell publishes as an event, which
the CLI reads from a `myrmic subscribe`; a round trip takes about a quarter
of a second.

```sh
chillguard status                         # status, latest reading, limits, open incident
chillguard incidents --limit 20           # newest incidents first (default 10, at most 100)
chillguard ack fridge-1-1791599824157     # record that someone has seen an incident
chillguard set-limits --upper 10 --critical-after 30m
chillguard simulate short_door_open       # dev mode only; also normal_day, power_failure_2h
chillguard watch                          # status changes, alerts, recoveries as they happen
chillguard watch --readings --for 5m      # plus every reading, for five minutes
chillguard start                          # a playground on this machine, with the web dashboard
chillguard stop                           # stop the playground
```

Options that work with every command:

| Option | Environment | Meaning |
|---|---|---|
| `--prefix <P>` | `CHILLGUARD_PREFIX` | Prefix of the cells' names, the `SRN_PREFIX` they were deployed with. Without it the cells are `sensor-cell`, `fridge-asset-cell`, `risk-monitor-cell`. |
| `--connect <endpoint>` | `MYRMIC_CONNECT` | Talk to one runtime (`tcp/127.0.0.1:20123`) instead of discovering one by multicast. |
| `--timeout <duration>` | | How long to wait for an answer. Default `10s`. |
| `--json` | | JSON instead of text. `watch --json` prints one object per line. |
| `--myrmic <path>` | `MYRMIC_BIN` | The Myrmic CLI to run. Default `myrmic`. |

The exit status is 0 on success, 1 when the command failed (the reason is on
stderr), 2 when the arguments were wrong.

What each command does beyond sending a message:

- **`status`** asks the asset cell for its snapshot and the risk monitor for
  its newest incident. The status shown is the asset's view; the incident is
  the monitor's. When the two cannot both be right the output carries a
  `note` line (see [when something looks wrong](#when-something-looks-wrong)).
  If the monitor does not answer, the status is still shown and the incident
  line says why it is unknown.
- **`ack`** checks that the incident exists, sends the acknowledgement, and
  waits until the monitor's incident list reports it. Acknowledging twice is
  not an error; an unknown id is.
- **`set-limits`** reads the limits in force, changes only what was given,
  checks the result with the same rules the cells use, sends it, and waits
  until the asset cell reports it. `--no-lower` removes the lower limit.
- **`simulate`** only sends. The sensor cell does not answer, and ignores the
  command unless it was deployed with `dev_mode` on. Its log line says which:
  `scenario started` or `simulate_scenario rejected`.
- **`watch`** shows the events of the refrigerator behind `--prefix` only,
  although events of every refrigerator travel the same network.
- **`start`** and **`stop`** do not talk to a deployment; they make and
  remove one, for trying ChillGuard out. See [the playground](#the-playground).

Example, a door left open with the test configuration (`config/e2e/`):

```text
$ chillguard watch
watching fridge-ops: status normal, last reading 4.0 °C, limits 2.0 to 8.0 °C, hysteresis 0.5 °C (Ctrl-C to stop)
2026-10-10T09:28:35Z  ALERT      warning   fridge-ops-1791624515095  out of range at 12.0 °C
2026-10-10T09:28:35Z  status     normal -> warning
2026-10-10T09:28:40Z  status     warning -> critical
2026-10-10T09:28:40Z  ALERT      critical  fridge-ops-1791624515095  out of range for 5s, 11.8 to 12.0 °C
2026-10-10T09:28:56Z  status     critical -> normal
2026-10-10T09:28:56Z  recovered  fridge-ops-1791624515095  was critical, out of range for 20s, now 7.4 °C

$ chillguard incidents
INCIDENT                  SEVERITY  STARTED               RECOVERED             EXPOSURE  RANGE           GAP  ACKNOWLEDGED
fridge-ops-1791624515095  critical  2026-10-10T09:28:35Z  2026-10-10T09:28:56Z  20s       7.9 to 12.0 °C  no   2026-10-10T09:28:46Z

1 incident for fridge-ops as of 2026-10-10T09:29:16Z
```

`EXPOSURE` is the time from the first out-of-range reading to the last one;
`GAP` says whether readings were missing for longer than `stale_after`
during the incident, in which case the exposure includes time nobody
measured (docs/decisions/0003-risk-policies.md, policy 6). A recovered
incident is an observation. It does not say the contents are safe.

## The playground

`chillguard start` puts a complete ChillGuard on this machine for as long
as you want to look at it, and `chillguard stop` removes it. The guide for
someone using it is [docs/playground/README.md](playground/README.md); this
is what it does, for whoever has to explain it.

1. Finds the built cells (`--cells`, else `target/wasm32-unknown-unknown/release`
   beside the program or under the current directory) and refuses to go on
   without all four.
2. Asks `myrmic runtimes <name> info` whether the playground's runtime
   (`chillguard-play`, or `--name`) is running. If not: checks that the
   dashboard's port (`--port`, 8642) and the runtime's loopback port
   (`--swarm-port`, 20124) are free, writes a runtime configuration with
   multicast off, a loopback listener and an embedded gateway, and starts it
   with `myrmic runtimes start <file> --name <name> --tmp --detached`.
3. Waits until `myrmic --connect tcp/127.0.0.1:<swarm port> network --once`
   lists the runtime by name.
4. Deploys `sensor-cell`, `fridge-asset-cell`, `risk-monitor-cell` and
   `dashboard-cell`, without a prefix, each with its `config/<cell>.dev.json`
   as compiled into the program. A cell that is already deployed is left
   alone.
5. Waits until the gateway serves `/chillguard/` and
   `/chillguard/status/0.json`, which is there once the asset cell has
   answered the dashboard cell.
6. Prints the address and the endpoint, and opens the address with
   `xdg-open` unless `--no-open` was given.

A `start` that fails after step 2 stops the runtime it started. A `start`
while the playground runs does steps 3 to 6 again, which is also how cells
that went missing are put back (below).

Where things are:

| What | Where |
|---|---|
| The runtime's configuration and the two ports | `$XDG_DATA_HOME/chillguard/playground/<name>/` (`~/.local/share/...` without the variable); removed by `stop` |
| The runtime's log | `myrmic runtimes chillguard-play logs` |
| The cells' state, the incidents | In the runtime's memory (`--tmp`). Gone with `stop`, a reboot, or a kill. |
| The other commands' way in | `export MYRMIC_CONNECT=tcp/127.0.0.1:20124`, no `--prefix` |

Things that were observed:

- **A computer that sleeps loses the cells, not the runtime.** After the
  machine had been suspended for an hour the runtime logged
  `hygiene: cleaning up after dead cell(s) cells=4` and
  `fencing: killing cell ... why=SelfSuperseded`, and the page answered
  `404 myrmic gateway: no application is mounted at this path`. The runtime
  itself kept running, with its memory. `chillguard start` then found it,
  deployed the four cells again, and they resumed from their state
  (`resumed=true`, the earlier incident still listed).
- **A taken port is not reported by the runtime.** A runtime whose gateway
  port is in use starts, logs nothing about it and serves nothing, which is
  why `start` checks both ports first.
- **The dashboard's port is open to the network** (docs/security.md, "the
  gateway"), and the playground has the page's scenario buttons on, so
  anyone who can reach the port can send commands to the playground's
  cells. They are a simulated refrigerator in a runtime that no other
  runtime can join; that is the whole of what is exposed.

## The web dashboard

`dashboard-cell` puts one page at `/chillguard/` on the swarm's gateways.
The runtime started from `config/runtime.dashboard.yml` embeds a gateway on
port 8080, so the page is at `http://<host>:8080/chillguard/`, for a phone
on the same network too. Set up and security: README, "Dashboard", and
docs/security.md, "the gateway".

| On the page | Where it comes from |
|---|---|
| A card per refrigerator: temperature, status, range, how long before the snapshot the reading was taken | The asset cell's answer to `get_status`, as `chillguard status` shows it. The status is the asset's view. |
| Exposure and active alert on the card | The open incident in the risk monitor's answer to `get_incidents`: its exposure so far, severity, start, temperature range, whether it was acknowledged. |
| Incident history | The same answer: the newest 20 incidents of each refrigerator, newest first. |
| Development controls | Only when the cell was deployed with `dev_controls: true` (`PROFILE=dev`). A button sends `simulate_scenario` to that refrigerator's sensor cell through the gateway, as `chillguard simulate <preset>` does through the Myrmic CLI. |

The dashboard cell asks both cells every `refresh_ms` and stores each answer
as a file beside the page (`status/<n>.json`, `incidents/<n>.json`, `<n>`
counting the refrigerators of its init payload from 0; `config.json` says
how many there are). The page fetches those files at the same pace, and
pauses while its tab is in the background. They are the wire messages of
docs/decisions/0004-wire-contract.md, so `curl` shows what the page sees:

```sh
curl -s http://localhost:8080/chillguard/status/0.json
curl -s http://localhost:8080/chillguard/incidents/0.json
```

What the page says when something is wrong:

| On the page | Meaning | Look at |
|---|---|---|
| `Not updating: ...` beside the title | The page cannot fetch from the gateway: the runtime is down, or the dashboard cell is not deployed (`config.json` is a 404). What is shown is what was last fetched. | `myrmic runtimes list`, `myrmic cells status`; `make deploy`. |
| A card with `No answer` and no values | The asset cell has left three refreshes unanswered, and the dashboard cell has removed its last snapshot rather than leave it looking current. Also shown until the first answer after a deploy, and when the dashboard's payload names a cell that does not exist. | `answer withdrawn` in the log names the cell; `myrmic cells status`; the names in `config/dashboard-cell.<profile>.json` against the `SRN_PREFIX` the cells were deployed with. |
| `unknown: the risk monitor is not answering` for exposure and alert | The same for the risk monitor. The card's status still comes from the asset cell. | as above |
| `No answer` with values and "This snapshot is ... old" | The file is there but old: the dashboard cell is deployed and no longer refreshing. | The dashboard cell's log; redeploy it (`scripts/deploy.sh dashboard-cell`). |
| A scenario button answers `... was not sent: ... has no placement` | The sensor cell named in the dashboard's payload is not deployed. | as above |
| A scenario button reports `Sent`, and nothing happens | The sensor refused: it was deployed without `dev_mode`. | `simulate_scenario rejected ... reason=dev_mode_off` in the log. |

Each refresh makes the asset log `status sent ... via=command` and the
monitor `incidents sent ... via=command`. A cell that is away is asked once,
not once per refresh: the runtime keeps commands for a cell that is not
deployed and delivers them all when it returns.

## What is running

| Question | Command |
|---|---|
| Which runtimes did this machine start? | `myrmic runtimes list` |
| One runtime's identity, data folder and database size | `myrmic runtimes <name> info` |
| Which nodes are on the network? | `myrmic network status` (also lists connected CLI clients, such as a running `chillguard watch`) |
| Which cells are deployed, where, since when? | `myrmic cells status` |

The cells of one refrigerator are the three names behind `--prefix`; the
dashboard cell is `<prefix>dashboard-cell`. After a
runtime restart `myrmic cells status` is empty: a restart releases the cells
and they must be deployed again (`make deploy`, with the same `PROFILE` and
`SRN_PREFIX` as before); their state is kept (docs/myrmic-notes.md, section
11). `make deploy` uses the `prod` payloads, under which `chillguard
simulate` is refused; `make deploy PROFILE=dev` uses the development ones
(config/README.md).

## Logs

There are two places to read logs, and they differ.

**The runtime's log file**, on the machine that runs it. Always there, no
setup:

```sh
myrmic runtimes <name> logs | grep wasm_log                       # everything the cells logged
myrmic runtimes <name> logs | grep wasm_log | grep ' WARN \| ERROR '
myrmic runtimes <name> logs | grep 'refrigerator_id=fridge-1 '    # one refrigerator
myrmic runtimes <name> logs | grep 'incident_id=fridge-1-1791599824157'   # one incident, start to end
myrmic runtimes <name> logs | grep ALERT                          # every alert raised
```

A cell's line looks like this; the cell's text is between `logging:` and
`sri=`, and the runtime appends its own fields:

```text
2026-10-10T09:35:51.971Z  WARN cell_task::message_handler:wasm_log: sorg_execution::wasm::host_functions::logging: ALERT severity=warning refrigerator_id=fridge-ops incident_id=fridge-ops-1791624951964 status=warning temperature_c=12.000 observed_at=1791624951964 sri=86496902-… (more runtime fields)
```

**The telemetry store**, from any machine that can reach the swarm, for all
runtimes at once:

```sh
myrmic telemetry logs
myrmic telemetry logs --trace-id <id>     # the lines of one command or event
myrmic telemetry logs -v                  # also DEBUG; -vv also TRACE (per the guide; the cells log neither)
```

```text
WARN  [2026-10-10T09:35:51.971+00:00] | trace_id = c3dc3d4102158f7a0f2865dfbdb26c44 | ALERT severity=warning refrigerator_id=fridge-ops incident_id=fridge-ops-1791624951964 status=warning temperature_c=12.000 observed_at=1791624951964
```

This prints **nothing** until a retention period is set; see
[retention and verbosity](#retention-and-verbosity). Only what is logged
while a retention is set is stored.

Logs are not rolled back when a handler fails. The cells log last, after
saving and publishing, so a cell's line stands for work that was done.

## Log fields

Every cell writes `message words key=value key=value ...`. The same thing has
the same key in every cell, and values that also travel on the wire are
written the way the wire writes them.

| Field | Meaning | Form |
|---|---|---|
| `refrigerator_id` | The refrigerator the line is about. On every line that concerns one. | text |
| `incident_id` | The incident the line is about; `none` where a line always has the field and there is no incident. | `<refrigerator_id>-<start in ms>` |
| `status` | The status after what the line reports. | `normal`, `warning`, `critical`, `sensor_stale` |
| `previous_status` | The status before a change. | as `status` |
| `severity` | How serious an alert or incident is. | `warning`, `critical` |
| `temperature_c` | The temperature of the reading the line is about. | °C, three decimals |
| `observed_at` | When that reading was taken. `none` on a start line of a cell that has no reading yet. | ms since 1970, UTC |
| `at` | When something that is not a reading happened: a status change taking effect, an acknowledgement. | ms since 1970, UTC |
| `reason` | Why something was refused. | a short name, or quoted text |
| `cell` | In the dashboard's lines: the asset or monitor cell the line is about. | SRI; `myrmic cells status` shows the name |

The lines, by cell:

| Cell | Level | Line |
|---|---|---|
| sensor | INFO | `sensor-cell started id= refrigerator_id= interval_ms= dev_mode= time_scale= resumed=` |
| sensor | INFO | `reading refrigerator_id= temperature_c= observed_at= phase=` |
| sensor | INFO | `scenario started refrigerator_id= preset= seed=` |
| sensor | WARN | `simulate_scenario rejected ... reason=` |
| asset | INFO | `fridge-asset-cell started id= refrigerator_id= resumed= status= observed_at= lower_limit_c= upper_limit_c= record_history=` |
| asset | INFO | `reading accepted refrigerator_id= temperature_c= observed_at=` |
| asset | WARN | `reading rejected refrigerator_id= temperature_c= observed_at= reason=` |
| asset | INFO | `status view changed refrigerator_id= status= previous_status= at=` |
| asset | WARN | `status view was behind refrigerator_id= previous_status= announced_previous_status=` |
| asset | WARN | `status change ignored refrigerator_id= status= at= reason=` |
| asset | INFO | `limits changed refrigerator_id= lower_limit_c= upper_limit_c= critical_after_ms= stale_after_ms= hysteresis_c=` |
| asset | INFO | `status sent refrigerator_id= status= reply_to= via=` |
| asset | WARN | `set_limits rejected reason=`, `get_status rejected reason=` |
| monitor | INFO | `risk-monitor-cell started id= refrigerator_id= resumed= status= incident_id= incidents= config_known=` |
| monitor | INFO | `status changed refrigerator_id= incident_id= status= previous_status= at=` |
| monitor | WARN | `ALERT severity=warning refrigerator_id= incident_id= status= temperature_c= observed_at=` |
| monitor | WARN | `ALERT severity=critical refrigerator_id= incident_id= status= temperature_c= observed_at= exposure_ms= highest_temperature_c= data_gap=` |
| monitor | INFO | `recovered refrigerator_id= incident_id= status= temperature_c= observed_at= severity= exposure_ms=` |
| monitor | WARN | `sensor stale refrigerator_id= incident_id= status= since=` |
| monitor | INFO | `sensor resumed refrigerator_id= incident_id= status= temperature_c= observed_at=` |
| monitor | WARN | `reading rejected refrigerator_id= temperature_c= observed_at= reason=` |
| monitor | INFO | `incident acknowledged refrigerator_id= incident_id= severity= at=` |
| monitor | INFO | `incidents sent refrigerator_id= count= truncated= reply_to= via=` |
| monitor | INFO | `tick armed refrigerator_id= period_ms=` |
| dashboard | INFO | `dashboard-cell started id= mount= refrigerators= refresh_ms= dev_controls=` |
| dashboard | WARN | `answer withdrawn cell= file= reason=no_answer refreshes=` |
| dashboard | INFO | `answers resumed cell= file=` |
| dashboard | WARN | `query repeated cell= command= reason=no_answer` (every 30 refreshes while a cell stays silent) |
| dashboard | WARN | `answer ignored cell= kind= reason=`, `answer not stored cell= file= reason=`, `query not sent cell= command= reason=` |

Every transition is logged at INFO and every alert at WARN. A cell logs at
ERROR only when its deployment is refused (`init refused reason=`).

## Traces

Every command and event a cell handles is one trace, and its log lines carry
the trace id. `myrmic send` prints the id of the command it sent:

```text
$ myrmic send fridge-asset-cell get_status '{"v":1,"body":{"reply_to":"probe"}}'
INFO  trace ID = 07f727c1a5666ce354d74e42483570a9
INFO  successfully sent command
$ myrmic telemetry logs --trace-id 07f727c1a5666ce354d74e42483570a9
INFO  [2026-10-10T09:35:41.798+00:00] | trace_id = 07f727c1… | status sent refrigerator_id=fridge-ops status=normal reply_to=probe via=event
```

To follow one reading through the cells, take the `trace_id` of its `ALERT`
or `status changed` line from `myrmic telemetry logs` and pass it back with
`--trace-id`.

```sh
myrmic telemetry traces --trace-id <id> > one-trace.json    # six spans, 2 KB, for one get_status
myrmic telemetry traces > trace.json                        # everything stored
```

The output is the Trace Event Format; open it at <https://ui.perfetto.dev/>.
Export one trace, not all of them: two minutes of three cells at one reading
a second produced 784 KB. Traces need a retention too.

## Metrics

```sh
myrmic telemetry metrics
```

prints the latest value of every metric, attributes indented below it. The
output is coloured even when piped; strip the colour to process it:

```sh
myrmic telemetry metrics | sed -E 's/\x1b\[[0-9;]*m//g' | grep -A4 '^cell_events_processed'
```

Metrics are exported at intervals, so in the first minute after a retention
is set there may be none. There are 46 metric names; these are the ones
whose meaning was checked against what the cells were doing. Each carries the
cell's `sri=` as an attribute.

| Metric | What it tells you |
|---|---|
| `cell_events_processed` with `event=reading_observed` (asset) or `event=reading_accepted` (monitor) | Readings are flowing: it grows by one per sensor interval. |
| `cell_commands_processed` with `cmd=get_status`, `get_incidents`, `set_limits`, `acknowledge` | Queries and commands a cell handled. |
| `cell_events_sent` with `event=...` | What a cell published, one series per event name. |
| `process.memory.usage`, `process.disk.io`, `system.memory.usage` | The runtime process: 76 MiB of memory with the three cells. |

`cell_events_sent` has one series per event name. That is why the operator
CLI asks for its answers under a fixed name per cell and query
(`cg_reply_get_status_<cell>`) and never a new one per request.

## Live stream

```sh
myrmic telemetry debug --timeout 60s        # add --json for one JSON object per line
```

shows events, commands and cell log lines of the whole swarm as they happen,
with nothing stored and no retention needed:

```text
[2026-10-10T09:37:11.117Z] EVENT=reading_observed payload={"body":{"reading":{…}},"v":1}
[2026-10-10T09:37:11.117Z] INFO reading refrigerator_id=fridge-ops temperature_c=4.011 observed_at=1791625031116 phase=Normal
[2026-10-10T09:37:11.121Z] INFO reading accepted refrigerator_id=fridge-ops temperature_c=4.011 observed_at=1791625031116
```

For one refrigerator's changes only, `chillguard watch` is the shorter view.

## Retention and verbosity

**Retention.** The telemetry query commands (`logs`, `traces`, `metrics`)
read a store that is empty until a retention period is set.

- While running: `myrmic telemetry set-db-retention 1h` (answers
  `DB retention set to '1h' on all connected nodes`). It lasts until the
  runtime restarts. `myrmic telemetry no-db-retention` turns storage off.
- At every start: in the runtime configuration,

  ```yaml
  telemetry:
    db_retention: "1h"
  ```

  `config/runtime.dev.yml` does not set it, so a dev runtime stores nothing
  until asked to.

The retention is also what bounds the store: every line and span is kept
until it expires. The cells write two INFO lines per reading, so one
refrigerator at one reading a second adds about 170 000 lines a day at
`info`. Keep the period short.

**Verbosity.** `telemetry.logs.env_filter` in the runtime configuration sets
it at start (`info` in `config/runtime.dev.yml`);
`myrmic telemetry set-filter "info"` changes it on all running nodes. The
cells log under the target `sorg_execution::wasm::host_functions::logging`,
so a filter that names only `swarm=...` hides them. The cells write nothing
below INFO, so `debug` shows more of the runtime, not of ChillGuard, and at
hundreds of lines a second.

**A caution about the configuration file.** The runtime refuses an unknown
key only at the top level of the file. A misspelt key inside `telemetry`
(or `execution`) is ignored without a message, so check that a setting took
effect.

**Sending telemetry elsewhere.** The guide describes exporting to an
OpenTelemetry collector (`otel_endpoint` under `telemetry`), for Grafana and
the like. That needs a Myrmic build with the `open-telemetry` feature and has
not been tried here.

## When something looks wrong

| What you see | What it means | What to do |
|---|---|---|
| `chillguard`: `... is not deployed on a runtime this client can reach` | No cell of that name. Wrong `--prefix`, or the runtime was restarted. | `myrmic cells status`; `make deploy`. |
| `chillguard`: `myrmic: no myrmic nodes discovered within ...` | No runtime reachable. | `myrmic runtimes list`; give `--connect` if the runtime does not use multicast. |
| `chillguard`: `no answer from ... within 10s` | The cell is deployed and did not reply. | Its log: a `get_status rejected` line, or errors about missing state. |
| `status` shows a `note` that view and monitor disagree | The asset's status view missed a `status_changed` event; the monitor's incident is the truth. | Trust the `incident` line. The view corrects itself at the next status change. In the asset's log: `status view was behind`. |
| `sensor stale` in the monitor's log, status `sensor_stale` | No accepted reading for longer than `stale_after`. | Is the sensor cell deployed and logging `reading`? Is the asset logging `reading accepted` or `reading rejected`? |
| `reading rejected ... reason=out_of_order` | A reading older than the last accepted one: late delivery, or a clock that went back. | If it persists, readings are being refused until the clock catches up (docs/decisions/0003-risk-policies.md, policy 3). |
| `reading rejected ... reason=duplicate` | The same timestamp twice. Harmless once. | |
| Alerts that open and close within seconds | Readings crossing a limit back and forth. | Raise the hysteresis: `chillguard set-limits --hysteresis 0.5`. |
| The same `ERROR ... errored out while serving command` every 5 s | A command whose handler keeps failing is being retried. | Read the message; the cells drop commands they can never accept, so this points at storage or the runtime. |
| The web page shows `No answer` or `Not updating` | See [the web dashboard](#the-web-dashboard). | |
| The playground's page says `no application is mounted at this path` | The runtime runs and its cells are gone, typically after the computer slept. | `chillguard start` again; see [the playground](#the-playground). |
| Readings of another site appear, or state changes by itself | Two deployments on one network share a refrigerator id or cell names. | Give each deployment its own `--prefix` and `refrigerator_id` (docs/myrmic-notes.md, sections 10 and 13). |
