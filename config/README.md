# Configuration

Two kinds of file: the **runtime configuration** a Myrmic runtime is started
with, and the **init payloads** each cell is deployed with. Every file here
is an example that works as it is; copy and change it for a real site.

## Runtime configuration

| File | What it is |
|---|---|
| `runtime.local.yml` | One host, off the network: multicast discovery off, listening on loopback only. Only a process on the host can reach the runtime; the CLI needs `--connect tcp/127.0.0.1:20123` or `MYRMIC_CONNECT`. The recommended starting point (docs/security.md). |
| `runtime.dashboard.yml` | `runtime.local.yml` plus an embedded gateway on port 8080, for the web dashboard. The swarm side is still loopback only; **the gateway listens on every interface** and has no setting to change that (docs/security.md, "the gateway"). |
| `runtime.dev.yml` | One host on the shared network: other runtimes on the LAN find it by multicast and receive its events. For a lab where that is wanted. |
| `e2e/runtime.yml` | What `scripts/e2e.sh` starts: like `runtime.dashboard.yml` with ports per run. |

```sh
myrmic runtimes start config/runtime.local.yml --name chillguard --detached
export MYRMIC_CONNECT=tcp/127.0.0.1:20123
```

The runtime refuses an unknown top-level key but ignores a misspelt key
inside a section (`execution`, `telemetry`, `zenoh`) without a message, so
check that a setting took effect (docs/myrmic-notes.md, section 14).

## Init payloads and profiles

`scripts/deploy.sh` (and so `make deploy`) passes `config/<cell>.<PROFILE>.json`
to `myrmic deploy --init-file`. There are two profiles:

| | `prod` (default) | `dev` (`make deploy PROFILE=dev`) |
|---|---|---|
| Sensor clock | real time | 60x: a two-hour power failure plays in two minutes |
| Reading interval | 10 s | 1 s |
| `simulate` command | refused and logged (`dev_mode` absent, so `false`) | accepted (`dev_mode: true`) |
| Critical after | 30 min | 60 s |
| Sensor stale after | 5 min | 30 s |
| Time-series history | off | on (`record_history: true`; the store has no expiry) |
| Dashboard refresh | every 30 s | every 2 s |
| Dashboard's scenario buttons | absent; the gateway serves files only (`dev_controls` absent, so `false`) | present; the gateway accepts commands over HTTP (`dev_controls: true`) |

Readings carry real timestamps in both profiles; only the simulated
refrigerator's thermal model runs fast under `dev`. The e2e payloads under
`e2e/` are the `dev` shape with even shorter durations; `e2e/README.md`.

`chillguard start` (the playground, docs/playground/README.md) deploys the
`dev` payloads too. They are compiled into the `chillguard` program from the
files here, so a change to a `*.dev.json` reaches the playground with the
next `make cli`.

### What each payload holds

| File | Fields | Checked at deploy |
|---|---|---|
| `sensor-cell.<profile>.json` | `refrigerator_id`, `interval_ms`, optional `dev_mode` (default `false`), `seed` (default `0`), `time_scale` (default `1.0`) | id not empty; interval above zero; time scale positive and finite; a time scale other than 1 only with `dev_mode` |
| `fridge-asset-cell.<profile>.json` | `config`: the refrigerator's `RefrigeratorConfig` (`refrigerator_id`, optional `lower_limit_c`, `upper_limit_c`, `critical_after` and `stale_after` in milliseconds, `hysteresis_c`); optional `record_history` (default `false`) | id not empty; the same rules the `set_limits` command applies: finite limits, lower below upper, hysteresis zero or positive and narrower than the range, `stale_after` above zero |
| `risk-monitor-cell.<profile>.json` | `refrigerator_id` | id not empty. Limits are not in it: they arrive with every `reading_accepted` from the asset cell. |
| `dashboard-cell.<profile>.json` | `refrigerators`: one entry per card, each naming the `sensor`, `asset` and `monitor` cells of one refrigerator as they were deployed (`SRN_PREFIX` included); `refresh_ms`; optional `dev_controls` (default `false`) | one to eight refrigerators; valid cell names; no cell named twice; `refresh_ms` at least 1000 |

Every payload refuses unknown keys, so a misspelt key fails the deployment.
A failed check fails the deployment too, and `myrmic deploy` prints the
reason after `Cell init error:`; a payload that is not valid JSON for the
shape prints `failed to deserialize json`, with no field named (that message
is the SDK's).

**The asset's `config` applies to a first deployment only.** A cell that
already holds state for that refrigerator keeps its stored configuration,
which `chillguard set-limits` may have changed since; a restart or a
redeploy must not undo an operator's change (docs/decisions/0005-state-ownership.md).
To start over, deploy under a new name (`SRN_PREFIX`).

**The dashboard's payload names the other cells.** The files here name them
without a prefix. A deployment made with `SRN_PREFIX=site-b-` needs a copy
that says `site-b-sensor-cell` and so on (point `CONFIG_DIR` at the copies),
or the dashboard asks cells that do not exist and every card says
`No answer`. One dashboard can show several deployments: list each one's
cells under `refrigerators`. It is mounted at `/chillguard` on every gateway
of the swarm, and a second dashboard cell on the same swarm is refused
(`mount already owned by another cell`).

**The dashboard writes to disk on every refresh.** Each status snapshot it
stores for the page stays in the runtime's database for good, because
Myrmic's blob store never deletes content: about 0.8 KiB per refresh and
refrigerator, 2.3 MiB a day at 30 s and 35 MiB a day at 2 s (measured;
docs/myrmic-notes.md, section 16, also has the larger short-run growth on
disk). `refresh_ms` is the lever. The incident list is written only when it
changes. Not deploying the cell costs nothing:
`scripts/deploy.sh sensor-cell fridge-asset-cell risk-monitor-cell`.

**`dev_controls` is more than buttons.** It makes the dashboard cell mount
the gateway's message API, through which an HTTP client can send any
command to any cell (docs/security.md, "the gateway"). Leave it off outside
development.

**One refrigerator id and one `SRN_PREFIX` per deployment.** Events travel
the whole network; two deployments that name the same refrigerator feed each
other's readings (docs/myrmic-notes.md, section 13).
