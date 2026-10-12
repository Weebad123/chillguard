# Security

What ChillGuard protects, who can reach it, what the code checks, and what it
does not. Written against Myrmic 0.7.0 and its security page
(<https://book.myrmic.dev/docs/security>), which describes the mechanisms the
developer preview has today; the facts about the runtime that are used below
are the ones recorded in docs/myrmic-notes.md.

Contents: [what is at stake](#what-is-at-stake) ·
[trust boundary](#trust-boundary) · [entry points](#entry-points) ·
[what every inbound message goes through](#what-every-inbound-message-goes-through) ·
[development-only commands](#development-only-commands) ·
[the gateway](#the-gateway) ·
[limiting access to control commands](#limiting-access-to-control-commands) ·
[data at rest and in logs](#data-at-rest-and-in-logs) ·
[supply chain](#supply-chain) · [known gaps](#known-gaps) ·
[reporting](#reporting)

## What is at stake

ChillGuard tells a person whether a refrigerator is, or has been, outside its
safe temperature range. The things worth protecting, in order:

1. **A true status and true alerts.** Showing `normal` for a refrigerator
   that is warm is the failure the product exists to prevent. Anything that
   can feed false readings, suppress a status change, or raise the limits can
   cause it.
2. **The incident history.** A recovered incident is evidence that contents
   were exposed. It must not be lost or altered.
3. **The configuration.** Limits and durations decide what counts as an
   incident. A change must be deliberate and must survive restarts.
4. **Availability of monitoring.** A sensor that is silenced, or a monitor
   that is kept from evaluating, goes `sensor_stale` at worst; it must never
   look healthy.

ChillGuard holds no personal data and no secrets. Its readings, incidents and
limits are not confidential in themselves; the risk is integrity, not
disclosure.

## Trust boundary

Myrmic's security page is explicit about what the preview provides:

- **The network segment is the trust boundary.** With mutual TLS off, which
  is the default, any runtime or CLI that can reach the others on the segment
  joins the swarm with no join step and no authentication, and messages carry
  no proof of their sender.
- **Mutual TLS narrows the boundary** to holders of a certificate issued
  under a root the deployment controls. It is configured on the Zenoh
  transport link (`enable_mtls`, `verify_name_on_connect`, endpoints under
  the `tls/` scheme) and is off by default. Issuing certificates is the
  deployment's job; the preview ships no CA.
- **Cells are isolated.** A cell is a WebAssembly module with no WASI: no
  filesystem, sockets, processes or host clock, only the host calls the
  runtime links in (messaging, timers, the data layer), with execution
  metered by fuel.

There is **no per-command authorization** in Myrmic 0.7.0 and none in
ChillGuard. A cell sees the sender of a command or event as an `Sri`: nil for
anything sent with the Myrmic CLI, a cell's identity for cell-to-cell
traffic, with no cryptographic backing in either case. Every control
operation below is therefore available to **whoever can reach the swarm**,
and limiting access means limiting who can reach the swarm. How to do that is
in [limiting access to control commands](#limiting-access-to-control-commands).

Observed consequences of a shared segment, from the notes (sections 10 and
13): another party on the network sent commands to, and deleted, a cell of
this project because it used the same name; a second test run fed its
readings into this one's asset cell because it used the same refrigerator id.

## Entry points

Everything a cell accepts, from docs/decisions/0004-wire-contract.md, with
the gate each one has beyond [message validation](#what-every-inbound-message-goes-through)
and what someone on the segment could do with it.

| Message | Kind | Handled by | Gate in the cell | If abused by a party on the segment |
|---|---|---|---|---|
| `reading_observed` | event | asset | `risk::check_reading`: same refrigerator id, finite value, later than the last accepted reading | **A forged reading is accepted.** In-range readings mask an excursion; out-of-range ones raise false alerts; a timestamp far in the future blocks every real reading until the clock catches up (policy 3 in 0003). The gap at the top of [known gaps](#known-gaps). |
| `reading_accepted` | event | monitor | same refrigerator id as the monitor's | As above, one hop later. |
| `status_changed` | event | asset | refuses a change older than the view | **A forged change moves the asset's status view**, for example to `normal`. The monitor's own state and incidents are untouched; `chillguard status` shows a `note` when the two disagree. |
| `alert_raised`, `incident_recovered` | event | nobody in the cells; `chillguard watch` reads them | validated on decode | A forged event shows up in a running `watch` only. |
| `get_status`, `get_incidents` | command | asset, monitor | `reply_to` must be a valid command name; `limit` within 1..=100 | Read access to status and history. The cell publishes its answer under the caller's `reply_to`; a name that another handler listens on receives a payload of the wrong shape, which that handler drops and logs. |
| `set_limits` | command | asset | `RefrigeratorConfig::validate` and the same refrigerator id; a change is logged and takes effect on the next reading | **Alarms can be silenced** by raising the limits or the critical duration, or made noisy. The change is persistent and survives restarts. |
| `acknowledge` | command | monitor | the incident must exist; the first acknowledgement stands | An incident is marked as seen by nobody. The incident itself is not changed. |
| `simulate_scenario` | command | sensor | **refused unless the cell was deployed with `dev_mode: true`**; the refusal is logged | With dev mode on: the simulated sensor is driven to any preset, which is what the demo does. |
| `status_snapshot`, `incident_list` | command | dashboard | the sender must be one of the cells named in the dashboard's init payload, by SRI; then validated on decode | The sender is not proven (see the trust boundary), so a party on the segment that forges it can put a false snapshot or incident list on the web page until the next real answer replaces it. Nothing in the other cells changes. |
| HTTP on the gateway's port | `GET` | the gateway | none: no login | Read access to the page, the status and the incidents, from wherever the port can be reached. With `dev_controls`, also `POST`: see [the gateway](#the-gateway). |
| init payload (`--init-file`) | deploy | each cell | unknown fields refused, content validated, see [config/README.md](../config/README.md) | Deploying is as powerful as it sounds: anyone who can deploy can replace a cell (docs/myrmic-notes.md, section 10). |

Timer ticks (`tick` in the sensor and monitor cells) are commands the runtime
sends with the cell's own identity. Nothing checks that; a `myrmic send
<cell> tick` from outside makes the sensor publish one extra reading or the
monitor evaluate the clock once. Neither does anything a reading or the next
tick would not.

## What every inbound message goes through

The checks are in `chillguard-protocol`, so the sender and the receiver
cannot disagree about them, and in `chillguard-core`, so they are unit-tested
on the host.

1. **Envelope.** Every message is `{"v": 1, "body": {...}}`.
   `Envelope::decode` reads `v` first and refuses any version but
   `SCHEMA_VERSION` as `UnsupportedVersion`, then decodes the body's shape
   (`Malformed`), then runs the body's `Payload::validate` (`Invalid`).
   Encoding runs the same validation, so a cell cannot put on the wire what
   no cell would accept. Pinned by `crates/chillguard-protocol/tests/malformed.rs`
   and `roundtrip.rs`, and the golden files under `tests/golden/`.
2. **Content.** Identifiers are not empty; temperatures are finite (NaN
   would compare in range against every limit); a status change changes the
   status; an alert's severity matches an open incident; a recovery agrees
   with its incident; limits pass core's validation; `reply_to` passes the
   SDK's command-name rules; an incident page size is bounded.
3. **Commands are decoded by the handler, not by the SDK.** A command whose
   handler fails is redelivered without bound (notes, section 10). Every
   command handler therefore takes the raw bytes, decodes them itself, and on
   any failure logs `<command> rejected reason=...` and returns `Ok`. A bad
   command is one log line, not a retry loop. A bad event is dropped by the
   runtime after one log line.
4. **Readings.** `risk::check_reading` rejects a reading for another
   refrigerator, a non-finite temperature, a duplicate timestamp and an
   out-of-order one. The asset and the monitor apply the same function;
   `asset_and_monitor_accept_the_same_readings` in
   `crates/chillguard-core/src/asset.rs` feeds one stream to both. Policies
   1 to 5 of docs/decisions/0003-risk-policies.md.
5. **Init payloads.** Each cell's `config.rs` derives `Deserialize` with
   `deny_unknown_fields` and validates the content before anything is
   stored: an empty refrigerator id, a zero interval, a non-positive or
   non-finite time scale, a time scale other than 1 without dev mode, and
   any configuration core would refuse all fail the deployment with the
   reason in `Cell init error:`.
6. **Bounded state.** The monitor keeps one open incident and the last
   reading; closed incidents go to a table one row each; `get_incidents`
   returns at most 100. The asset's optional time-series history is off by
   default because the store has no expiry.

## Development-only commands

`SimulateScenario` is the one development command. It is marked
`Payload::DEV_ONLY` in the protocol, and the sensor cell accepts it only when
its init payload says `"dev_mode": true`. The field defaults to `false`
(`SensorConfig` in `crates/cells/sensor-cell/src/config.rs`); a payload that
omits it gets a sensor that refuses the command with
`simulate_scenario rejected ... reason=dev_mode_off` and does nothing else.

The deployment profiles in `config/` follow that: `make deploy` uses the
`prod` profile, whose sensor payload has no `dev_mode`, and only
`make deploy PROFILE=dev` turns it on. A time scale other than real time
(`time_scale`) is also refused unless dev mode is on, so a production sensor
cannot be deployed with a compressed clock by mistake.

## The gateway

The web dashboard is served by a Myrmic gateway, and the gateway is a second
way in, with its own reach. What follows was observed on Myrmic 0.7.0
(docs/myrmic-notes.md, section 16).

- **It listens on every interface.** The gateway embedded in the runtime
  (`gateway:` in `config/runtime.dashboard.yml`) binds `[::]:<port>` and has
  no setting for the address, also when the swarm side of the same runtime
  listens on loopback only. Whoever can reach the host on that port can load
  the page. There is no login; the gateway's OIDC support
  (`myrmic gateway --routing`) belongs to the standalone gateway and was not
  exercised.
- **Without `dev_controls` the dashboard is read-only by construction.** The
  dashboard cell mounts static files and nothing else. The gateway then has
  no path that carries a message into the swarm (`GET /chillguard/api`
  answers 404 and a `POST` 405; the `dashboard` scenario of `make e2e`
  asserts the first). What is
  exposed is what the page shows: the refrigerator's id, status, last
  reading, limits and incidents. None of it is confidential in itself; a
  site that disagrees should not start a gateway.
- **With `dev_controls: true` the gateway accepts messages over HTTP, and
  not only for the dashboard.** The page needs the gateway's message API to
  send `simulate_scenario`. That API is not limited to the cell that mounts
  it: with it mounted, an HTTP client sent `acknowledge` to the risk
  monitor and `get_status` to the asset cell, and published an event of its
  choosing. So `dev_controls` gives everyone who can reach the port
  everything the table under [entry points](#entry-points) lists, including
  `set_limits` and forged readings, without the Myrmic CLI. It is for a
  development machine or a closed demo network. `dev_controls` defaults to
  `false`, the `prod` profile leaves it out, and only
  `make deploy PROFILE=dev` turns it on; that profile's sensor accepts
  `simulate_scenario` anyway, which is the point of it.
- **The page trusts no value it shows as markup.** Identifiers and numbers
  from the swarm are written as text nodes; the page loads nothing from
  another origin and stores nothing.
- **A page that looks healthy when it is not** is the failure to avoid (item
  1 of [what is at stake](#what-is-at-stake)). The dashboard cell removes
  the stored answer of a cell that has left three refreshes unanswered, and
  the page shows `No answer` for a missing one. The page also compares each
  status snapshot's `as_of` with the gateway's clock, from the response's
  `Date` header and not from the phone's own clock, and refuses to present
  an old one as current. `chillguard status` remains the reference.

**The playground** (`chillguard start`) is the `dev` profile on a runtime
of its own: `dev_controls` on, gateway on every interface. What that
exposes is the playground itself, a simulated refrigerator held in memory
by a runtime that has multicast off and listens for other nodes on loopback
only; it cannot be used to reach another deployment. `start` says so when it
prints the address. Stop it when you are done, and do not run it on a
network where a stranger pressing "Power failure" would matter.

To serve the page without opening the port to a network: keep the host's
firewall closed for the port and reach it through an SSH tunnel
(`ssh -L 8080:127.0.0.1:8080 <host>`), or put a reverse proxy that
authenticates in front of it.

## Limiting access to control commands

`set_limits`, `acknowledge` and `simulate_scenario` change what the system
does or records, and `get_status` and `get_incidents` read it. None of them
can tell one caller from another. Per Myrmic's security page the controls
are at the transport, and they apply to all five alike. (The gateway is a
transport of its own; [the section above](#the-gateway) covers it.)

1. **Keep the runtime off any network you do not control.** For a single
   host, `config/runtime.local.yml` disables multicast discovery and listens
   on loopback only; the operator CLI reaches it with
   `--connect tcp/127.0.0.1:20123` (or `MYRMIC_CONNECT`). Only a process on
   that host can then send a command. The end-to-end harness runs on the
   same configuration with a port per run (`config/e2e/runtime.yml`), so
   the pattern is exercised on every `make e2e`; a runtime started from
   `runtime.local.yml` was checked to be reachable with `--connect` and
   absent from multicast discovery.
2. **Several hosts: a dedicated segment.** Myrmic's guidance is to treat the
   segment as trusted, so give the runtimes a network segment of their own
   and nothing else on it.
3. **Mutual TLS between nodes.** Narrows the boundary to certificate
   holders. It is configured under `zenoh:` in the runtime configuration on
   the transport link's TLS settings (the page names `enable_mtls`,
   `verify_name_on_connect`, a root certificate and per-node certificate
   chains, and `tls/` endpoints); the Myrmic repository's
   `swarm/tutorials/mTLS/` brings up a complete example with a local CA.
   **Not exercised in this project.** Two cautions from the notes: the
   runtime ignores an unknown key inside `zenoh:` without a message (section
   14), so check that mTLS took effect, for example that a CLI without a
   certificate cannot connect; and a forwarding node terminates the link and
   relays in the clear.
4. **Who can run `myrmic` on an allowed host.** The Myrmic CLI has no
   accounts. Anyone who can execute it on a host that reaches the swarm can
   send any command, deploy and delete cells, and export the database. Use
   the operating system for that: a dedicated user for the runtime, and
   restrict who may log in.
5. **Unique names.** Give every deployment its own `SRN_PREFIX` and
   `refrigerator_id` (docs/myrmic-notes.md, sections 10 and 13). This
   separates deployments that share a segment; it is not an access control.

Within the swarm the cells add what they can verify: a `set_limits` is
applied only if it names the asset's refrigerator and passes validation, and
every change is logged with the new values (`limits changed ...`); an
`acknowledge` is recorded only for an incident that exists and only once;
every refusal is logged with its reason. The log is the audit trail
(docs/operations.md, log fields).

## Data at rest and in logs

- Cell state and the incident table live in the runtime's database, under
  `$XDG_DATA_HOME/myrmic` (or `~/.local/share/myrmic`), unencrypted.
  `myrmic database export` reads any cell's scope from any node of the
  swarm. File permissions and the trust boundary above are the controls.
- The runtime's log file and the telemetry store carry readings, status
  changes, alerts and the refrigerator and incident identifiers, nothing
  else. The cells log no payload bytes and no configuration beyond the
  limits. Keep it that way: the runtime does not roll logs back with a
  failed handler, and the telemetry store keeps lines for the retention
  period on every node.
- The operator CLI prints what the cells answered. It stores nothing.

## Supply chain

- One toolchain, pinned in `rust-toolchain.toml`; CI installs that exact
  nightly and builds with `--locked` against the committed `Cargo.lock`.
- `cargo deny check` (`deny.toml`) runs in CI: dependencies must carry an
  allowed permissive licence, no crate may be yanked, no advisory from the
  RustSec database may be open, and only crates.io is an allowed source.
- The Myrmic CLI used in CI is the release `.deb`, verified against the
  SHA-256 published with the release before it is installed
  (`.github/workflows/ci.yml`).
- `unsafe_code` is denied across the workspace. The one exception is each
  cell's `handlers` module, where the SDK's handler attributes expand to
  `#[unsafe(no_mangle)]` exports; the module allows the lint and contains no
  hand-written `unsafe` (docs/decisions/0001-core-vs-cells.md).
- `unwrap`, `expect` and `panic!` are denied outside tests (`Cargo.toml`,
  `clippy.toml`), so a malformed input cannot take a cell down.

## Known gaps

In order of weight.

1. **Readings are not authenticated.** Anyone on the segment can publish a
   `reading_observed` for a known refrigerator id and it is accepted. Mutual
   TLS keeps strangers off the segment; it does not tell the asset cell
   which member of the swarm sent an event. A future asset cell could pin
   the sensor it listens to by comparing `md.sender` with the SRI derived
   from the sensor's name (an event from a cell carries that cell's SRI; one
   from the CLI carries nil, docs/myrmic-notes.md section 5). Not built.
2. **The status view can be forged.** A `status_changed` from anyone moves
   the asset's view (docs/decisions/0005-state-ownership.md, problem 3). The
   monitor's state is the truth and `chillguard status` says when they
   disagree, but `get_status` on its own cannot.
3. **`set_limits` has no second factor.** A party that can reach the swarm
   can raise the limits and silence the alarm. The change is logged; nothing
   stops it. A deploy-time lock on the limits (refuse `set_limits` unless
   the init payload allows it) would be a small change and is not built.
4. **Mutual TLS is documented, not tested here.** See point 3 of
   [limiting access](#limiting-access-to-control-commands).
5. **No rate limiting.** A flood of commands or events is handled one at a
   time per cell (one handler runs at a time), so a flood delays readings
   behind it. The runtime's fuel metering bounds each handler, not the
   queue.
6. **The dashboard has no login**, and its development controls open the
   swarm over HTTP. See [the gateway](#the-gateway): view-only deployments
   expose readings and incidents to whoever reaches the port; `dev_controls`
   exposes every command.
7. **A clock step is a blind spot** (policy 3 in 0003): one reading with a
   far-future timestamp, forged or from a broken sensor, makes every later
   real reading `out_of_order` until real time catches up. The only signal
   is the stream of `reading rejected` lines.

## Reporting

Report a suspected vulnerability privately to the repository owner rather
than in a public issue, and include the Myrmic version (`myrmic --version`)
and the ChillGuard commit. There is no bug-bounty programme.
