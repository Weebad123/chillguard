# Myrmic verified notes

Facts about the Myrmic SDK and CLI that have been checked against the installed
version on this machine, the official docs, or the SDK source. Every entry names
its source. Anything not listed here is unverified: check before using it.

Discovery spike run on 2026-10-09. Throwaway cell kept at `/tmp/myrmic-spike/spike-cell`
(tmpfs, not part of the repo).

Legend: **[local]** = observed on this machine. **[docs]** = official docs page.
**[src]** = SDK source in `~/.cargo/registry/src/index.crates.io-*/`.

---

## 1. Installed versions

| Component | Version | Where | Source |
|---|---|---|---|
| `myrmic` CLI | 0.7.0 (29a7dd91) | `/usr/bin/myrmic`, Debian package `myrmic-cli` (maintainer Peeriot GmbH) | [local] `myrmic --version`, `dpkg -s myrmic-cli` |
| `myrmic-sdk` crate | 0.7.0 | crates.io | [local] `cargo info myrmic-sdk` |
| `myrmic-sdk-macros` crate | 0.7.0 | crates.io, pulled in by `myrmic-sdk` | [local] |
| `myrmic-common` crate | 0.7.0 | crates.io, shared wire types | [local] |
| Cell toolchain | `nightly-2026-08-07`, components `rust-src`, target `wasm32-unknown-unknown` | pinned by the `rust-toolchain.toml` that `myrmic new` writes | [local] scaffold output |
| Host toolchain | stable `rustc 1.98.1` | default | [local] |
| Build platforms | `linux`, `riscv32imac` (alias `esp32_c6`) | `myrmic platforms` | [local] |

The crate `myrmic` (0.1.0) on crates.io is unrelated; do not depend on it.

Docs index: https://book.myrmic.dev/llms.txt. Doc pages are HTML only (no `.md` variant).
SDK repo: https://github.com/peeriot/myrmic. API reference: https://docs.rs/myrmic-sdk/0.7.0.

## 2. Toolchain gotchas observed

- **[local]** `rustup toolchain install nightly-2026-08-07 --profile minimal` installs rustc but
  not cargo; `myrmic build` then fails with
  `the 'cargo' binary ... is not applicable to the 'nightly-2026-08-07' toolchain`, and
  `rustup component add cargo` falsely reports "up to date". Fix that worked: uninstall and
  reinstall with `--profile default`:
  ```sh
  rustup toolchain uninstall nightly-2026-08-07
  rustup toolchain install nightly-2026-08-07 --profile default --component rust-src --target wasm32-unknown-unknown
  ```
- **[docs]** Normally the first `myrmic build` auto-installs the toolchain via `rust-toolchain.toml`.
  Source: https://book.myrmic.dev/docs/quickstart/installation
- **[local]** A cell crate also compiles on the native host with **stable**:
  `cargo +stable check`, `cargo +stable clippy --all-targets -- -D warnings`, `cargo +stable fmt --check`,
  and `cargo test` (0 tests) all pass for the spike cell. The Wasm target does **not** build on
  stable (`can't find crate for core`; the SDK also gates `feature(alloc_error_handler)` on wasm32).
  The Wasm build must go through `myrmic build`.
- **[local]** `myrmic build` obeys a `rust-toolchain.toml` above the crate. With the file pinning
  `stable` the build fails (`the -Z flag is only accepted on the nightly channel`); with no file,
  or with the file pinning `nightly-2026-08-07`, it works. A workspace-root toolchain file must
  therefore pin that nightly, which also makes it the toolchain for the host gates.
- **[docs]** A C toolchain (`build-essential`) is needed because build scripts compile natively.
  Source: installation page above.

## 3. CLI commands verified

All `[local]` unless noted. Global flags: `-v/-vv`, `--timeout <dur>` (e.g. `8s`), `--connect <endpoint>`.

| Purpose | Command | Notes |
|---|---|---|
| Scaffold a cell | `myrmic new <path> [--name <crate>]` | Writes `Cargo.toml`, `.gitignore`, `rust-toolchain.toml`, `src/lib.rs` (counter demo). |
| **Build a cell to Wasm** | `myrmic build [<crate dir or Cargo.toml>]` | Release build with `cargo +nightly-2026-08-07`. Artifact: `<crate>/target/wasm32-unknown-unknown/release/<crate_name_with_underscores>.wasm`. ~65 s cold, ~1.5 s warm. |
| Build several platforms | `myrmic build --platform linux,riscv32imac` | [docs] https://book.myrmic.dev/docs/reference/myrmic-cli/build |
| Build a workspace | `myrmic build <workspace dir>` | [docs] compiles all member crates to Wasm. **[local]** It treats **every** member as a cell: on the ChillGuard workspace it fails on `chillguard-core` with `#[panic_handler] function required`. Build each cell crate instead. |
| Pick a cargo target | `myrmic build --target lib|<bin name>` | [docs] auto-selects sole bin, else sole lib. |
| Start a runtime | `myrmic runtimes start --name <n> [--tmp] [--detached] [--tag t]` | `--tmp` = in-memory DB. Alias `run`. Writes `~/.local/share/myrmic/runtimes/<n>.yaml` and `~/.local/share/myrmic/<id>/{db,logs}` even with `--tmp` (logs only). PID file in `$XDG_RUNTIME_DIR/myrmic/<n>.pid`. |
| List runtimes | `myrmic runtimes list` | |
| Runtime logs | `myrmic runtimes <n> logs` | Cell `info!` lines appear as `INFO wasm_log: ... sri=<sri>`. |
| Stop a runtime | `myrmic runtimes delete <n>` | Sends SIGTERM. |
| Nodes on network | `myrmic network --once` | |
| Deploy | `myrmic deploy <crate dir \| .wasm \| app_specs.yml> [--name <srn>] [--init <json>] [--init-file <f>] [--tag t] [--policy never\|on-error\|always]` | A `.wasm` deploys without building and defaults its SRN to the file name **including `.wasm`**, so pass `--name`. From a crate dir it builds first. |
| List cells | `myrmic cells --once` | Columns: cell, sri, kind, runtime, age, policy, class, srn. |
| Send a command | `myrmic send <srn\|sri> <cmd> [json payload] [--raw]` | Sender SRI seen by the cell is nil. No payload = empty buffer. |
| Publish an event | `myrmic publish <event> [json payload] [--raw]` | `myrmic publish set_limit 2.5` decoded into an `f64` handler param. |
| Subscribe to events | `myrmic subscribe [names...]` | Prints `event=<name> sender=<sri> payload=N bytes` then the JSON. Runs until Ctrl-C. |
| Remove a cell | `myrmic delete <srn> --cell` (or `--app`, `--branch`, `--children`) | Required before redeploying a changed cell: redeploying a class with live instances is refused. [docs] https://book.myrmic.dev/docs/tutorials/smart-greenhouse/the-mock-sensor |
| Other subcommands present | `flash`, `monitor`, `database`, `replicate`, `tags`, `gateway`, `cells classes/status/teardown`, `telemetry` | Help text read; not exercised here. `database`, `telemetry` and `gateway` since: sections 11, 14 and 16. |

Reference pages: https://book.myrmic.dev/docs/reference/myrmic-cli/new,
.../myrmic-cli/build, .../myrmic-cli/deploy, .../myrmic-cli/runtimes/start.

## 4. Cell crate shape

**[local]** Exactly what `myrmic new` 0.7.0 generates:

```toml
# Cargo.toml
[package]
name = "spike-cell"
version = "0.1.0"
edition = "2024"
publish = false

[package.metadata.myrmic]
heap_size = 32_768

[dependencies]
myrmic-sdk = "0.7.0"
serde = { version = "1", default-features = false, features = ["alloc", "derive"] }
```

```toml
# rust-toolchain.toml
[toolchain]
channel = "nightly-2026-08-07"
components = ["rust-src"]
targets = ["wasm32-unknown-unknown"]
```

- The crate is a `#![no_std]` **library** (`src/lib.rs`), no `main`, no allocator or panic
  handler boilerplate (the SDK's default `cell-init` feature emits them on wasm32). [src] `lib.rs`
- `[package.metadata.myrmic]` keys: `heap_size` (default 32768), `stack_size` (default 32768),
  `initial_memory` (131072), `max_memory` (131072); baked into the binary at build time.
  [docs] https://book.myrmic.dev/docs/reference/configuration/cell-and-application-configuration
- Multiple cells can live in one crate as `[[bin]]` targets selected with `--target`, or one
  crate per cell in a workspace with a shared types crate. [docs] https://book.myrmic.dev/docs/guides/cells
- SDK default features (`alloc`, `db`, `cells`, `cell-init`) are "the only configuration that
  builds today". [src] `lib.rs` feature table.

## 5. SDK API (crate `myrmic_sdk` 0.7.0)

### Handlers and macros [src] `myrmic-sdk-macros/src/lib.rs`, [docs] reference pages

| Attribute | Export | Payload rules |
|---|---|---|
| `#[myrmic_sdk::init]` | `init_cell`, runs once per incarnation (first deploy **and** every restart/respawn) | `fn(md: Metadata)` or `fn(md: Metadata, cfg: T)` where `T: Decoder`; deploy payload via `--init`. |
| `#[myrmic_sdk::cmd]` / `#[cmd(name = "x")]` | `command_<name>` | `fn(md: Metadata)` (rejects any payload), `fn(md, p: T)`, or `fn(md, p: Option<T>)` (empty buffer = `None`). Generates a marker type usable as `Callback::of::<fn_name>()`. |
| `#[myrmic_sdk::evt]` / `#[evt(name = "x")]` | `event_<name>` | Same shape as `cmd`. **Declaring the handler is the subscription.** No marker type. |
| `#[myrmic_sdk::monitor]` | fixed `on_cell_lost` | `fn(md, lost: monitor::CellLost)`; at most one per cell. |
| `#[derive(myrmic_sdk::Message)]` + optional `#[codec(myrmic_sdk::Json)]`/`#[codec(myrmic_sdk::Postcard)]` | derives `Encoder`+`Decoder` | JSON is the default. Also derive `serde::{Serialize, Deserialize}`. |

- **[local]** The macro-crate doc comment on `#[evt]` says "the standalone macros do not yet
  register the subscription - a cell must call `myrmic_sdk::subscribe_to_event`". That note is
  stale: no such function exists in the SDK, and a plain `#[evt] fn reading(...)` received
  events (from itself and from `myrmic publish`) with no extra call.
- All handlers return `myrmic_sdk::Result<T = ()>` = `core::result::Result<T, &'static str>`.
  Host-call failures are `ApiError` (enum) and convert into `&'static str` so `?` works. [src] `error.rs`
- Command and event names: ASCII letters, digits, underscores only; non-empty; no whitespace.
  [src] `myrmic-common/src/cells/names.rs`

### Metadata and identity [src] `metadata.rs`, `myrmic-common/src/cells/ids.rs`

- `Metadata { id: Sri, sender: Sri }`. **[local]** `sender` is nil for `init` of a root deploy and
  for anything from the CLI; it is the cell's **own** SRI for timer ticks. [docs] it is the
  gateway session's SRI for gateway calls, and the spawner for a spawned child's init.
- `Sri` (UUIDv5 derived from the SRN path): `Sri::NIL`, `is_nil()`, `of_path("app/cell")`,
  `from_target(&str)` (UUID or SRN path), `child("name")`, `as_uuid()`, `from_uuid()`,
  `Display`, `FromStr` (strict UUID). An SRI cannot be turned back into an SRN.
  [docs] https://book.myrmic.dev/docs/reference/myrmic-sdk/cell-model/identity-and-metadata
- SRN segments must match `^[a-z0-9][a-z0-9._-]*$`. [src] `Srn::parse` doc comment.

### Messaging [src] `messaging.rs`, `messages.rs`

- `myrmic_sdk::publish(name, &value) -> Result` and `myrmic_sdk::send(sri, name, &value) -> Result`,
  both fire-and-forget; success means the runtime accepted it. `name` is anything
  `TryInto<Event>`/`TryInto<Command>` (a `&str` works).
- Payload-less: `publish("x", &myrmic_sdk::Void)`.
- `Callback<T>`: `Callback::of::<handler>()` (type-checked) or `Callback::to("name")?`;
  `cb.invoke(md.sender, &reply)` sends the reply as a command back to the caller. Use
  `Option<Callback<T>>` if the CLI may call the handler, and check `!md.sender.is_nil()` before
  invoking. [docs] https://book.myrmic.dev/docs/reference/myrmic-sdk/messaging/callbacks
- Scalars `String, bool, char, f32, f64` and all integers encode as bare JSON values; `Bytes`
  (`Vec<u8>`) passes through raw; `JsonValue` = `serde_json::Value`. [src] `codec.rs`
- Re-exported alloc items: `myrmic_sdk::{String, Vec, vec!, format!, Bytes}`.

### Persistent state [src] `host_functions/db/state.rs`

- `const X: State<T> = State::new_const("key")` (private scope) or `State::new_const_in(key, Scope)`;
  runtime key: `State::<T>::new(&key)` / `new_in`. Default codec is **Postcard** (`State<T, C>`).
- Methods: `load() -> Result<Option<T>>`, `save(&T)`, `load_from(key)`, `save_to(key, &T)`,
  `modify(|&mut T|) -> Result<Option<T>>`, `guard() -> Option<Guard>` (saves on drop, best-effort;
  call `guard.save()?` to observe errors). With `T: Default`: `upsert()`, `upsert_with(|&mut T|) -> Result<T>`,
  `guard_or_default()`.
- `T` needs `serde::Serialize + DeserializeOwned`. Structs work (tutorial grow-bed); `TimerHandle`
  is serde-able and is stored this way.
- Limits [docs]: reads go through an 8 KiB buffer; writes have no limit, so a value can become
  unreadable. https://book.myrmic.dev/docs/reference/myrmic-sdk/state-and-storage/persistent-state

### Other stores [src] `db/tree.rs`, `db/table.rs`, `db/ts.rs`, `db/mod.rs`

- `Kv<V>`: `Kv::new("prefix")`, keys stored as `"prefix/key"`; `get`, `put`, `delete`, `keys(sub)`,
  `for_each(sub, f)` (skips decode errors), `iter(sub)`, `list(sub)`. 8 KiB per value, 16 KiB key list.
- `Table<V, K = String>`: `Table::new("name")`; `get`, `insert` (host id), `insert_with(key, v)`,
  `delete`, `count`, `list`, `list_rev`, `iter`, `keys`, `to_map`. Keys: `String`, `Vec<u8>`, `Sri`.
- Time series: `publish_measurement(Scope, Measurement { name, tags: Vec<(String,String)>, fields: Vec<(String, FieldValue)>, ts: Option<u64> }) -> ApiResult`
  and `find_measurement(scope, name, limit, start, end, order, &mut req_buf, &mut resp_buf) -> ApiResult<FindResponse { samples: Vec<Sample> }>`.
  `FieldValue::{I64,U64,F64,String,Boolean}`, `TsOrderBy::{TimestampAsc, TimestampDesc(default)}`,
  `Timestamp = u64` (ms). **Publish request must fit 100 bytes total** (scope+name+tags+fields);
  no delete, no expiry. **[local]** a measurement with name `temp` and one `F64` field succeeded.
  [docs] https://book.myrmic.dev/docs/reference/myrmic-sdk/state-and-storage/time-series-store
- `Scope`: `Scope::private()` (default), `Scope::private_in(Some("schema"))`,
  `Scope::public("namespace")`, `Scope::public_in(ns, Some(db), Some(schema))`. Private scope is
  derived from the cell's SRI; public scopes are shared and unprotected.
  [docs] https://book.myrmic.dev/docs/reference/myrmic-sdk/state-and-storage/storage-scopes
- `InMemory<T>` (`static X: InMemory<Option<T>> = InMemory::empty()`): `with(|&mut T|)`,
  `try_borrow`, `try_borrow_mut`, `upsert_with`; lives in Wasm memory, lost on restart.

### Timers and time [src] `host_functions/cell/timers.rs`, `host_functions/time.rs`

- `myrmic_sdk::interval(Callback::of::<handler>(), period).build()? -> TimerHandle`;
  `interval_at(cb, first_delay, period)`; builder options `.count(n)`, `.fixed_delay()`.
  `myrmic_sdk::delay(cb, dur).build()?`. `handle.cancel()?` consumes the handle.
  Scheduled handler must be a `#[cmd]` taking only `Metadata`.
- **Dropping a `TimerHandle` does not cancel the timer**; store it in `State` to cancel later.
- Limits [docs]: 5 scheduled invocations per cell; ms resolution; queue of 10 pending ticks.
  Timers die with the incarnation: re-arm them in `#[init]`.
- **[local]** `interval(.., 1s)` fired about every 1–1.5 s; `cancel()` stopped it.
- `myrmic_sdk::now() -> ApiResult<Duration>` since UNIX epoch from the swarm-synchronised clock
  (**[local]** `as_millis()` gave `1791555716895` on 2026-10-09). `uptime()` monotonic per node.
  `wait(dur)` blocks the whole cell. `now()` can return `ApiError::NotReady` on embedded nodes.
  [docs] https://book.myrmic.dev/docs/reference/myrmic-sdk/scheduling-and-time/clock-and-wait

### Logging [src] `host_functions/logging.rs`

- `myrmic_sdk::{trace!,debug!,info!,warn!,error!}(fmt, args) -> ApiResult<()>`, plus `*_str(&str)`
  and `log(&str, LogLevel)`. Logs are **not** rolled back with a failed handler. Keep secrets out.

### Runtime facts, spawning, bridges (read, not exercised)

- `runtime_tags() -> ApiResult<Vec<String>>`, `runtime_id() -> ApiResult<String>`. [src] `runtime.rs`
- `const CHILD: ClassHandle = myrmic_sdk::declare!("class")`; `CHILD.new().name(..).payload(&cfg).tag(..).grace(..).deadline(..).spawn()?`.
  Linux runtime only. [docs] https://book.myrmic.dev/docs/reference/myrmic-sdk/cell-lifecycle/cell-classes-and-spawning
- `myrmic_sdk::import!("bridge.yml")` generates typed HTTP/MQTT bridge clients. [src] macros `import`.

## 6. Runtime behaviour that shapes cell design [docs]

Sources: https://book.myrmic.dev/docs/reference/myrmic-sdk/error-model,
https://book.myrmic.dev/docs/guarantees, https://book.myrmic.dev/docs/reference/myrmic-sdk/messaging/events

- One handler runs at a time per cell; commands, events, callbacks and timer ticks form one
  ordered stream. Long work blocks everything behind it.
- A handler's storage writes and outbound messages commit atomically when it returns `Ok`.
  On `Err`/trap they roll back; spawns, timers, logs and hardware writes do **not** roll back.
- After a failure: a **command is redelivered** (unbounded retries, so handlers must be
  idempotent); an **event is dropped**; a scheduled tick is lost but the schedule continues;
  init failure fails the deployment.
- Events are only delivered to cells deployed before the publish; a payload that fails to decode
  is skipped and logged, the handler never runs.
- Private DB state survives restarts and redeploys under the same SRI; `init` re-runs over the
  old data, so seed idempotently (e.g. `if X.load()?.is_none()`).
- State is held as one copy by default; losing the node loses it unless replication is configured.
- Redeploying a class with running instances is refused: `myrmic delete <srn> --cell` first.
- `--policy on-error|always` restarts root cells (max 5 per 60 s, 1 s apart by default).

## 7. End-to-end smoke test performed [local]

Throwaway cell with: `#[init]` arming a 1 s `interval`; `#[cmd] tick` reading `now()`, updating a
`State<struct>` via `upsert_with`, writing a time-series sample and publishing a struct event
`reading`; `#[evt] reading(md, Reading)`; `#[evt] set_limit(md, f64)`; `#[cmd] stop` cancelling
the timer. Sequence that worked:

```sh
myrmic build                                  # -> target/wasm32-unknown-unknown/release/spike_cell.wasm
myrmic runtimes start --name spike-rt --tmp --detached
myrmic deploy ./target/wasm32-unknown-unknown/release/spike_cell.wasm --name spike-cell
myrmic subscribe reading                      # JSON payloads every ~1 s, sender = cell SRI
myrmic publish set_limit 2.5                  # evt handler logged limit=2.5, sender nil
myrmic send spike-cell stop                   # ticks stopped
myrmic runtimes spike-rt logs | grep wasm_log
myrmic delete spike-cell --cell
myrmic runtimes delete spike-rt
```

Wire format seen on `subscribe`: `{"observed_at_ms":1791555717928,"refrigerator_id":"fridge-1","temperature_c":5.0}`
(81 bytes, JSON, keys sorted).

## 8. Workspace findings (2026-10-09, ChillGuard scaffold) [local]

- Cell crates work as members of a Cargo workspace with inherited settings
  (`version.workspace = true`, `myrmic-sdk = { workspace = true }`, `[lints] workspace = true`).
  `myrmic build crates/cells/<cell>` then writes to the **workspace** target directory:
  `target/wasm32-unknown-unknown/release/<crate_name_with_underscores>.wasm`. First build of the
  workspace took about 70 s; a no-change rebuild under 1 s per cell.
- The handler macros expand to `#[unsafe(no_mangle)] pub extern "C" fn` exports, so
  `unsafe_code = "deny"` rejects every handler, and `missing_docs` flags the undocumented export
  when the handler sits in a public scope. Keeping handlers in a private module that carries
  `#![allow(unsafe_code)]` satisfies both, and the exports still reach the runtime (init ran).
  The generated `.unwrap()` calls do not trip `clippy::unwrap_used`.
- CLI exit codes: `myrmic` exits 1 on a failed deploy, a failed delete (`nothing named 'x' is
  deployed`), an unparsable runtime config, or no node discovered. `myrmic cells --once <srn>` exits
  0 even when the cell is not registered (prints `Cell <srn> is not registered`).
- Redeploying a name that is live fails with `application '<srn>' is already deployed`.
- `myrmic runtimes start <file.yml>` rejects an unknown **top-level** key
  (`unknown field 'bogus_section'`). Inside `execution` and `telemetry` an unknown key is ignored
  without a message (section 14), so a file that starts proves little about nested keys.
  `config/runtime.dev.yml` (`execution.tags`, `telemetry.logs.format`, `telemetry.logs.env_filter`) starts.
- Placement tags work as documented: a runtime started with `--tag t` accepts `deploy --tag t`; a
  deploy requiring a tag no runtime carries fails with `deployment infeasible ... missing required tags`.
  Tags from the config file and from `--tag` are merged.
- Setting `XDG_DATA_HOME` moves the runtime's identity file and logs out of `~/.local/share/myrmic`;
  `myrmic runtimes <name> logs` finds them as long as the same variable is set.
- A freshly started runtime shows up in `myrmic network --once` within a second but with an
  empty name at first; wait until its name is listed before deploying. After
  `myrmic runtimes delete` the process takes a moment to exit.

## 9. Implications for ChillGuard (derived from the facts above)

- Cells are `no_std` libraries built by `myrmic build <crate dir>`; the pure
  `chillguard-core` and `chillguard-protocol` crates must therefore be `no_std` + `alloc`
  compatible (with `serde` `default-features = false, features = ["alloc", "derive"]`) to be
  usable from cells. Core logic takes time as a `u64` ms parameter; cells obtain it via `now()`.
- Protocol payload structs derive `serde::{Serialize, Deserialize}` and `myrmic_sdk::Message`
  (JSON) in `chillguard-protocol` itself. The derive implements the SDK's `Encoder`/`Decoder`
  traits, and Rust's orphan rule only allows that in the crate that defines the type, so the
  protocol crate depends on `myrmic-sdk`. Field renames break wire compatibility silently.
- Every handler must be idempotent (commands are redelivered on error). Excursion tracking must
  tolerate seeing the same reading twice.
- Re-arm timers in `#[init]`; seed state only if absent. Do not keep a `TimerHandle` across
  incarnations (section 10).
- Time-series publish payloads must stay under 100 bytes: short series names, few short fields.
- All gates run on the pinned nightly through the workspace-root `rust-toolchain.toml`
  (docs/decisions/0002-toolchain-and-workspace.md).
- **Wasm build command for the quality gate:** `make build-cells`, which runs
  `myrmic build crates/cells/<cell>` for each cell.

## 10. Findings from the sensor cell (2026-10-09) [local]

All observed with `sensor-cell` on a `--tmp` runtime.

**Periodic handlers and time**

- `myrmic_sdk::interval(Callback::of::<tick>(), period).build()?` in `#[init]` works as
  documented. The first tick fires at once (2 ms after init returned), then every period
  (1.001 s measured for 1 s). A tick's log lines carry `message_type="timer_tick" identifier="tick"`.
- `TimerHandle` is `#[must_use]`; dropping it needs an explicit `let _ =`.
- Timers do die with the incarnation: after delete and redeploy, cancelling the handle stored by
  the previous incarnation fails with `timer 0 not found`, and the host logs that at ERROR.
  Timer ids start again at 0, so a handle kept from an earlier incarnation can name a *new*
  timer. Do not keep handles across incarnations.
- A tick that returns `Err` is logged (`WARN ... error in timer tick 'tick': <msg>`) and the
  schedule continues.
- `now()?.as_millis()` feeds `Timestamp::from_millis` directly. `libm::exp` and `{:.3}` float
  formatting work in the Wasm cell (binary: 157 KB).

**Init payload**

- `myrmic deploy <wasm> --init '<json>'` and `--init-file <file>` both reach
  `#[init] fn(md, cfg: T)`. With no payload the deploy fails
  (`Cell init error: no payload was sent; ...`). An `Err` from init fails the deploy with
  `Cell init error: <the cell's message>`; a payload that does not decode gives
  `Cell init error: failed to deserialize json`.
- Private state survives `myrmic delete <srn> --cell` followed by a deploy under the same SRN,
  on a `--tmp` runtime too, for as long as the runtime runs. A redeployed sensor cell found its
  stored simulator and carried on.

**Commands that fail are retried, whatever the reason**

- A command whose handler returns `Err` is redelivered every 5 s. It was still being retried
  when observation stopped; nothing suggests a bound.
- This includes a payload that fails to decode into the handler's parameter type. The handler
  never runs, the runtime logs `errored out while serving command`, and retries. One malformed
  `myrmic send` is a permanent retry loop. (Section 6 says a payload that fails to decode is
  skipped; that is the documented behaviour for *events*.)
- Pending commands outlive the cell: after delete and redeploy under the same SRN, the new
  cell received the two commands that had been failing against the old one.
- Consequence: a `#[cmd]` handler takes `myrmic_sdk::Bytes`, decodes the payload itself, and
  returns `Ok` after logging anything that can never succeed. `myrmic send <srn> <cmd>` with no
  payload arrives as an empty buffer.

**A shared network is a shared namespace**

- Runtimes on one machine or LAN find each other by multicast, and `runtimes start` has no
  documented switch to turn that off. During this session another party on the network used
  the SRN `sensor-cell`. Because the SRI and the private storage scope are derived from the
  SRN, their commands reached this cell (`myrmic send sensor-cell ...`), their
  `myrmic delete sensor-cell --cell` removed it, and for about 3 s this cell's `State::load()`
  returned `None` for values it had saved. With a unique SRN none of this recurred.
- Give every cell an SRN that is unique on the network (as `scripts/e2e.sh` does with
  `SRN_PREFIX`) and deploy with a tag only your runtime carries.
- `--connect <endpoint>` makes a CLI command talk to one runtime. The endpoint is in the
  runtime's log: `Zenoh can be reached at: tcp/<ip>:<port>`. A runtime can also be configured
  to be reachable only that way (section 13).

**Reading logs and events**

- `myrmic runtimes <name> logs | grep wasm_log` shows cell log lines, each followed by host
  fields (`sri=`, `uptime_*`, `zid=`, and for handler invocations `message_type=`, `identifier=`).
- `myrmic telemetry logs` prints nothing until a retention period is set:
  `myrmic telemetry set-db-retention 30m` ("on all connected nodes"; add `--connect` to limit
  it). After that it prints `INFO [<time>] | trace_id = <id> | <cell message>`.
- `myrmic subscribe` pretty-prints payloads with keys **sorted**. That is the CLI's display,
  not the wire: the bytes are in declaration order (`{"v":1,"body":{...}}`), as the golden
  files in `chillguard-protocol` pin. The same holds for the sample in section 7.
- About 30 s after a deploy the runtime logs
  `escalating uncovered offload of CELLS/<sri>/p to a replica` at INFO. It is routine.

## 11. Findings from the asset cell, and runtime restarts (2026-10-09) [local]

Observed with `sensor-cell` and `fridge-asset-cell` on a runtime with a **persistent** database
(started without `--tmp`, `XDG_DATA_HOME` pointing at a private directory).

**Restarting a runtime: exact behaviour**

| Step | What happened |
|---|---|
| `myrmic runtimes <name> info` while running | Prints the runtime id, `status running (pid N)`, the data and db folders, and the db size. |
| `myrmic runtimes delete <name>` | Sends SIGTERM; the process is gone within a second. `runtimes list` then says `no local runtimes`, but `runtimes <name> info` says `status stopped`: the identity file, the database and the log stay on disk. |
| `myrmic runtimes start <config> --name <same name>` (no `--tmp`) | Same runtime id as before, same database (`fjall::db: Recovering database at ...`). The log file is appended to. |
| Cells after the start | **Gone.** The runtime logs `WARN ... supervision::startup: startup sweep: releasing cells of a previous incarnation count=2`, and `myrmic cells --once` prints `No cells registered`. Nothing respawns them. |
| The same with `deploy --policy always` | No difference: the cells are released at startup. The restart policy does not cover a runtime restart. |
| The same after `kill -9 <pid>` instead of SIGTERM | No difference, apart from LSM-tree recovery lines at startup. |
| `myrmic deploy <wasm> --name <same SRN> --init ...` afterwards | `#[init]` runs with the payload of *this* deploy. `State::load()` returns what was saved before the stop. |

What survived, checked through the asset cell's `get_status`:

- the configuration as last changed by `set_limits` (not the one in the init payload);
- the latest accepted reading, including one accepted 46 ms before `kill -9`;
- the status view;
- the sensor cell's simulator (`resumed=true`).

What did not: the cells themselves, their timers, and any reading the sensor would have taken
while the runtime was down.

Consequences: after a runtime restart the cells must be deployed again (`make deploy`). An
`#[init]` must treat stored state as the truth and its payload as a default for a first
deployment, or a restart silently undoes every change made since.

**Events between cells** (closes an open question)

- `publish("reading_observed", ..)` in the sensor cell reached `#[evt] fn reading_observed` in
  the asset cell about 3 ms later. The asset's own `publish("reading_accepted", ..)` showed up
  in `myrmic subscribe reading_accepted` with `sender=<asset sri>`.
- `myrmic publish <event> '<json>'` reaches `#[evt]` handlers in cells.
- An event whose payload fails to decode, or whose handler returns `Err`, is logged once
  (`ERROR <the cell's message>` and `WARN ... error handling event '<name>': <message>`) and is
  not delivered again. Three bad events gave exactly three errors.

**Commands: two more cases**

- A command the target cell has no handler for is dropped:
  `WARN ... does not serve cmd '<name>'; discarding it`. It is not retried.
- A command to an SRN that is not deployed fails at the sender. The CLI prints
  `ERROR command failed: cell <sri> has no placement`.
- So the three outcomes of a command are: no placement (sender fails), no handler (dropped),
  handler error (redelivered every 5 s without bound, section 10).
- A command from the CLI has a nil `md.sender`: a query handler has nobody to reply to and
  can only log its answer.

**Time-series store**

- `publish_measurement(Scope::private(), Measurement { name: "temp", tags: [], fields:
  [("c", F64)], ts: Some(<ms>) })` works from an `#[evt]` handler, once a second, with no
  failure in several minutes.
- Read back by exporting the cell's scope:
  `myrmic database export CELLS/<sri>/p --target <path>`. The file held one `:ts temp` entry
  per sample (126 after two minutes) and one `:kv asset` entry per `State::save`.
- `myrmic database status` lists the scopes a node holds: `CELLS/<sri>/p` per cell, plus
  the runtime's own (`sorg/...`, `sys/replication/p`).
- Size, from `runtimes <name> info`: 36 KiB for an empty database, 808 KiB after about five
  minutes of these two cells at one reading a second with the history on. There is no
  expiry and no delete for time series, so the history is opt-in per deployment.

**A shared machine**

- A runtime's data directory under a fixed name in `/tmp` was deleted by another party while
  the runtime was running. Keep `XDG_DATA_HOME` for a runtime somewhere nobody else will name,
  for example under the worktree's ignored `target/` directory.

## 12. Findings from the risk monitor cell (2026-10-10) [local]

Observed with all three cells on one persistent runtime, playing the simulator's
`short_door_open` scenario with limits tightened by `set_limits` (upper 10 °C, critical after
60 s, hysteresis 0.5 °C).

**Timers within one incarnation**

- `interval(..).build()?` can be called again later in the same incarnation, from an `#[evt]`
  handler, and the previous handle's `cancel()?` succeeds. The monitor keeps the handle in a
  `static InMemory<Option<(TimerHandle, Duration)>>` and re-arms when `stale_after` changes:
  `tick armed period_ms=150000`, then after a `set_limits` `tick armed period_ms=15000`, with
  no error from the cancel.
- Re-arming from `#[init]` after a redeploy works the same way; the `InMemory` slot is empty
  then, so there is nothing to cancel.
- A tick handler that finds nothing to do and returns `Ok` costs nothing visible.

**Table store (`myrmic_sdk::db::table::Table<V, K = String>`)**

- `Table::new("incidents")`, `insert_with(&key, &v)` (replaces), `get(&key)`, `list_rev()`,
  `count()` all work from `#[evt]` and `#[cmd]` handlers. Keys of the form
  `<refrigerator>-<ms>` list in time order.
- `count()` from `#[init]` works too (`incidents=1` after a redeploy).
- Rows written in a handler that also publishes events are visible to the next handler
  invocation; nothing suggested a write and its events can come apart.

**Persist, then emit: what a restart does**

- The monitor was deleted and redeployed while an incident was critical. Init logged
  `resumed=true status=Critical incident_id=... incidents=1 config_known=true`; no
  `alert_raised` was published again, the next readings were evaluated against the restored
  state, and the recovery 10 minutes later closed the same incident id. Two `ALERT` lines in
  the whole run, one per severity.
- Both `State::save` and `Table::insert_with` from the same handler were present after the
  redeploy.

**Stale detection end to end**

- With `stale_after` 30 s the tick period is 15 s. After `myrmic delete <sensor> --cell` the
  monitor logged `sensor stale` at the first tick more than 30 s after the last reading
  (43 s after the delete), the asset's view followed within 5 ms, and redeploying the sensor
  gave `sensor resumed` on its first reading.

**Latency of the chain**

- sensor `reading_observed` → asset `reading accepted` → monitor evaluated → asset
  `status view changed`: 3 ms end to end on this machine (02:37:04.157 reading,
  02:37:04.162 alert, 02:37:04.165 view).

**A stale Wasm artifact, cause unknown**

- At the start of this session `target/wasm32-unknown-unknown/release/fridge_asset_cell.wasm`
  was dated 20:38 of the previous day, older than the asset cell's sources, although
  `make ci` had reported it built at 20:51 and `make e2e` had deployed it. Deploying it failed
  in init with `failed to deserialize json`. A controlled test afterwards (touch a source,
  build one cell, check all three artifacts) showed `myrmic build` behaving correctly: each
  build touches its own artifact only and rebuilds on a change. The cause was not found.
- Mitigation: `scripts/deploy.sh` now refuses a `.wasm` that is older than any `.rs` file
  under `crates/`. `touch crates/cells/*/src/lib.rs && make build-cells` forces a rebuild.

**Clock**

- `now()` and the host's log timestamps agree to the millisecond: a reading with
  `observed_at=1791599824157` (02:37:04.157 UTC) was logged by the host at 02:37:04.157704.

## 13. End-to-end harness (2026-10-10) [local]

`scripts/e2e.sh` runs four scenarios (six since sections 16 and 17) against a throwaway runtime with the clock compressed
(`config/e2e/`: simulator `time_scale` 60, `critical_after` 5 s, `stale_after` 10 s). What
it relies on, all observed:

- **A persistent runtime, not `--tmp`.** The `restart` scenario stops the runtime and starts it
  again under the same name; with `--tmp` the database is gone with the process. A temporary
  `XDG_DATA_HOME` keeps the database out of everyone's way and is removed at the end.
- **One SRN prefix per scenario.** State lives under the SRI, which is derived from the SRN, so
  `e2e-<pid>-<scenario>-<cell>` gives every scenario empty state without any reset command.
- **Queries go through the operator CLI.** The harness first read answers back from the cells'
  logs (`status snapshot={json}`), because a `myrmic send` has a nil sender. Since the cells
  answer such a caller with an event (section 14), it calls `chillguard --json status` and
  `chillguard --json incidents` instead, and uses `simulate`, `set-limits` and `ack` as well, so
  every run tests the operator CLI against a real runtime.
- **Runtime restart timings.** `runtimes delete` then `runtimes start` and three deploys took
  about 10 s end to end on this machine; the cells' first readings after a restart were 9 s
  apart from the last ones before it. To test the data-gap policy the harness keeps the runtime
  down for `stale_after + 5 s`.
- **Stale detection timing.** With `stale_after` 10 s (tick every 5 s) the asset reported
  `sensor_stale` 13 s after `delete <sensor> --cell`, in three runs.
- **Scenario durations at 60x:** `short_door_open` reaches warning 11 s after the command,
  critical 5 s later, normal about 16 s after that; `power_failure_2h` reaches warning about
  92 s after the command. The whole suite takes about 5 minutes.
- **One product bug found.** The risk monitor wrote an incident's table row only when an alert
  or recovery fired. `get_incidents` therefore reported an open incident as it was at its last
  alert: exposure frozen at 5014 ms and `has_data_gap=false` while core's own state had moved on
  (seen in `restart`, where the row was expected to show the outage). Fixed by asking core which
  record is due after each transition (`MonitorState::changed_incident`), with a regression test
  in core (`changed_incident_is_due_after_every_reading_that_touches_it`) and a property test.
- The harness's own mistakes while it was written: boolean spelling (`true`, not `True`),
  shell arithmetic (`1`, not `true`), an outage shorter than `stale_after`, and log counts
  spanning the whole runtime log instead of the scenario (fixed by filtering on the line's
  timestamp, which sorts as text).
- The staleness guard in `scripts/deploy.sh` (section 12) earned its keep: after a change to
  core only the monitor had been rebuilt, and the guard refused the sensor and asset binaries.
  `scripts/e2e.sh` now runs the same check before starting a runtime, and a failed deploy is
  reported with its message instead of as "cell did not start".
- The guard also gave a false positive, which explains part of section 12's "stale artifact":
  `myrmic build` finished in 0.2 s and left the monitor's `.wasm` untouched after a change to
  `src/risk/tests.rs`. That is correct: a `#[cfg(test)]` module is not part of a release build,
  so cargo has nothing to do, while "any `.rs` newer than the binary" says otherwise. The guard
  now leaves out `tests/` directories and `tests.rs` files. The binary of 2026-10-09 20:38 that
  failed in init was a different case: its sources were real code changes, and that remains
  unexplained.
- Teardown with no runtime running costs 30 s per `myrmic delete` (discovery timeout); the
  harness now skips the cell deletes unless it started a runtime, and uses a 5 s timeout.
- **Events cross runtimes, so a shared `refrigerator_id` crosses test runs.** While this
  harness ran, another session on this machine ran the same harness, with its own runtime, tag,
  SRN prefix and data directory, and the same `fridge-e2e`. Its sensor's `reading_observed`
  events reached this run's asset cell, which accepted those 4 °C readings between its own 12 °C
  ones: 28 warnings and recoveries in one scenario, no critical, stale noticed late. Tags and SRN
  prefixes isolate placement and state, not events. The harness now writes a refrigerator_id
  unique to the run into copies of `config/e2e/`, so another run's readings are rejected as
  another refrigerator's.
- **`State::load()` returned `None` for state the cell had saved**, for about 4 s starting
  17 s after the asset cell's deploy: `error handling event 'reading_observed': asset state is
  missing`, `get_status` failed the same way and was retried. Another session's runtime was on
  the network at the time. Two runtimes of my own on the network, with the cells on one of
  them, did **not** reproduce it in 60 s (61 readings accepted, no error). Same symptom as
  section 10's 3 s of `None`, which was blamed on another party's delete; the cause is still
  not pinned down, only the correlation with other nodes being present.
- **The runtime configuration carries the whole Zenoh config under `zenoh:`** (the unknown-field
  error lists the keys: `id`, `mode`, `connect`, `listen`, `scouting`, `routing`, `transport`,
  ...). Top-level keys are `execution`, `telemetry`, `zenoh`; there is no `network` or `swarm`.
- **A runtime can be kept off the network.** With this in the runtime configuration

  ```yaml
  zenoh:
    scouting:
      multicast:
        enabled: false
    listen:
      endpoints: ["tcp/127.0.0.1:<port>"]
  ```

  `myrmic network --once` (multicast) does not list the runtime, while
  `myrmic --connect tcp/127.0.0.1:<port> network --once` does; `deploy`, `send`, `cells` and
  `delete` all work with `--connect`, and `runtimes <name> logs` needs nothing (it reads files).
  The runtime logs `Zenoh can be reached at: tcp/127.0.0.1:<port>`. `scripts/e2e.sh` runs on
  such a runtime (`config/e2e/runtime.yml`, port `20000 + pid % 10000`), so other runtimes on
  the machine neither see it nor receive its events.
- A harness run whose output was piped into `head` was killed by SIGPIPE before its EXIT trap
  ran and left its runtime behind (without cells). The harness now traps PIPE, INT and TERM so
  cleanup always runs, and writes cleanup's own messages to stderr.
- The same holds for production: two deployments that name the same refrigerator and can see
  each other on the network will feed each other's readings.

## 14. Operator CLI and observability (2026-10-10) [local]

**A client outside the swarm: the Myrmic CLI as transport**

- `chillguard` (crates/chillguard-cli) runs `myrmic send` and `myrmic subscribe` as child
  processes. Of the three documented ways in for an external client, this is the one already
  verified here; the gateway needs cells to mount routes and a gateway node, and bridges are for
  calling out. [docs] https://book.myrmic.dev/docs/reference/myrmic-sdk/external-systems/gateway-routes
- **Answers to a caller without an identity travel as events.** A cell cannot reply to a command
  from `myrmic send` (nil sender), so the asset and monitor cells `publish(reply_to, &answer)`
  in that case and `Callback::to(reply_to)?.invoke(sender, ..)` otherwise. The client subscribes
  to the name, sends the query, reads the event. Measured: subscriber ready 89 ms after start,
  answer 135 ms after the send, `chillguard status` (two queries) 0.44 s in all.
- `myrmic subscribe <name>` as a child process: events go to **stdout** (an empty line, the
  header `[<time>] event=<name> sender=<sri> payload=<n> bytes`, the payload pretty-printed);
  `INFO  subscribed to: <name> (ctrl-c to stop)` goes to **stderr** and is the signal that the
  subscription is in place. Stdout is flushed per event, also into a pipe.
- `myrmic send`: `INFO  trace ID = <id>` and `INFO  successfully sent command` on stderr, exit 0.
  To a name that is not deployed: `ERROR command failed: cell <sri> has no placement`, exit 1.
  With no runtime reachable, `subscribe` prints
  `ERROR no myrmic nodes discovered within <timeout> (extend with --timeout)` and exits 1.
- `--timeout` takes plain milliseconds (`10000ms`).
- A `myrmic subscribe` whose parent was killed keeps running and shows up in
  `myrmic network status` as a node of kind `cli` (`myrmic subscribe @ user@host`). Kill the
  process itself, not a shell wrapper around it.

**One metric series per event name**

- `myrmic telemetry metrics` lists `cell_events_sent` once per event name
  (`event=<name>`). A reply name made up per request (`cg_reply_<pid>_<clock>_<n>`) added a
  series per query: 32 after a few minutes of manual use. The client now derives the name from
  the query and the cell asked (`cg_reply_get_status_<cell>`), which gave exactly two series.
  Any name a cell publishes under should come from a small fixed set.

**Observability commands, as run** (guide: https://book.myrmic.dev/docs/guides/observability)

- `myrmic network status` and `myrmic cells status` exist and print what `network --once` and
  `cells --once` print. `network status` also lists connected CLI clients.
- `telemetry.db_retention: "10m"` in the runtime configuration turns the telemetry store on at
  start: `myrmic telemetry logs` returned the runtime's own start-up lines with no
  `set-db-retention` having been run.
- `myrmic telemetry logs --trace-id <id>` with the id `myrmic send` printed returned exactly the
  cell's line for that command.
- `myrmic telemetry metrics` printed 46 metric names about 90 s after the retention was set,
  coloured even into a pipe (strip with `sed -E 's/\x1b\[[0-9;]*m//g'`). Per-cell metrics carry
  `sri=`; `cell_commands_processed` carries `cmd=`, `cell_events_processed` carries `event=`.
- `myrmic telemetry traces --trace-id <id>`: six spans, 1.8 KB, for one `get_status`. All traces
  after two minutes of three cells at one reading a second: 784 KB.
- `myrmic telemetry debug --timeout 4s` streamed `EVENT=<name> payload=<json>` lines and the
  cells' log lines live, with no retention set for it.
- Not tried: `set-filter`, `-v` on `telemetry logs`, export to an OpenTelemetry collector.
- **Nested unknown configuration keys are silently ignored.** `execution:\n  bogus: 1` and
  `telemetry:\n  bogus: 1` start without a message; only an unknown top-level key is an error.
  The observability tutorial says the same. This corrects section 8.

**Logs**

- All three cells now write the same keys for the same things (`refrigerator_id`,
  `incident_id`, `status`, `previous_status`, `severity`, `temperature_c`, `observed_at`, `at`,
  `reason`), with status and severity in their wire spelling. The table is in
  docs/operations.md.
- The cells no longer write query answers into the log; the end-to-end harness asks through
  `chillguard --json` instead of scraping `status snapshot=` lines.

## 15. Open questions / not yet verified

- `app_specs.yml` build/deploy (`classes`/`instances`) is documented but not exercised.
- `find_measurement` was not exercised; buffer sizing for responses is the caller's problem.
- Whether anything can make cells come back by themselves after a runtime restart
  (an `app_specs.yml` deployment, perhaps) is not known. `--policy always` does not.
- A standalone `myrmic gateway` on a multicast network, the gateway's `--routing` file with OIDC,
  `--over-https`, and the WebSocket path (`.ws()`) are documented and were not exercised
  (section 16 covers the embedded gateway, static files and the HTTP message API).
- Whether the database's on-disk growth from repeated `State::save` is given back by compaction
  was not measured over a long enough run (section 16 has the short-run numbers).

Closed since: the gateway for a dashboard, and the reply path of a query from a **cell**
(`Callback::to(name)?.invoke(md.sender, &reply)`), both in section 16.

## 16. Gateway and the dashboard cell (2026-10-12)

Read first: the gateway pages of the book
(https://book.myrmic.dev/docs/reference/myrmic-cli/gateway,
.../reference/myrmic-sdk/external-systems/gateway-routes, .../gateway-assets,
.../reference/myrmic-sdk/state-and-storage/blob-store,
.../reference/configuration/runtime-configuration) and the two dashboard tutorials
(.../tutorials/smart-greenhouse/the-dashboard for a page that polls a file a cell rewrites,
.../tutorials/scaling/the-dashboard for a page that sends commands). Then run with
`dashboard-cell` beside the three other cells on a loopback-only runtime.

**SDK surface used** [src] `host_functions/gateway.rs`, `db/store.rs`

- `gateway::assets(md.id) -> BlobStore`, the cell's own asset scope
  (`gw/gateway-assets/<sri>`); `put(path, bytes)`, `get`, `size_of`, `list`, `delete`, `rename`.
- `gateway::mount("/chillguard").index("/index.html").fallback(Fallback::None).api("/api").bind()`;
  `.ws(path)`, `.assets()`, `.scope(..)` exist too. Nothing is served until `bind()`. Its error
  is a `GatewayError`, which converts with `.map_err(<&'static str>::from)`.
- `gateway::unmount(path)`. Not needed: a cell's routes go with the cell.
- [docs] Mounting and unmounting join the handler's transaction. A path belongs to the cell that
  mounted it first; that cell may mount it again, another cell's attempt fails. A cell that uses
  the gateway does not load on an embedded node.

**Which gateway** [local]

- **Embedded works.** `gateway:\n  port: <p>` in the runtime configuration starts a gateway
  inside the runtime (`swarm_gateway: gateway listening on [::]:<p>`), and it served the cell's
  mount at once. The runtime logs `gateway routing table updated: N route(s)` when a cell mounts
  or goes.
- **It listens on every interface**, also in a runtime whose swarm side listens on loopback only
  (`ss -ltn`: `*:<p>`). The section has one key, `port`; there is no bind address.
- **A standalone `myrmic gateway --connect tcp/127.0.0.1:<port>` did not reach a runtime that has
  multicast off.** It started and listened, never showed up in `myrmic network status`, logged
  `route reconcile failed: unable to list gateway routes: ... no connected databases` every
  15 s, and answered every path with `404 myrmic gateway: no application is mounted at this
  path`. Same with the flag before the subcommand. Not tried: a standalone gateway on a
  multicast network, which is what the tutorials run.

**What the gateway answers** [local], cell mounted at `/chillguard` with `Fallback::None`

| Request | Answer |
|---|---|
| `GET /chillguard/`, `/chillguard/index.html` | `200`, `content-type: text/html` |
| `GET /chillguard` | `307`, `location: /chillguard/` |
| `GET /chillguard/config.json` | `200`, `application/json` (the type follows the extension) |
| `GET /chillguard/<no such file>` | `404 not found` (with the default `Fallback::Spa` an extensionless path gets the index instead [docs]) |
| `GET /` or any path outside a mount | `404 myrmic gateway: no application is mounted at this path` |
| any of them after `myrmic delete <cell> --cell` | `404`: route and files go with the cell, at once |

Responses carry `content-length` and `date` and no caching headers. [docs] Writing to a path
replaces what the gateway serves; the route does not need registering again.

**The HTTP message API** (`.api("/api")`) [local]

- `GET /chillguard/api` is a `text/event-stream`. Its first message is
  `data: {"type":"ready","session":"<uuid>"}`. The session lives while the stream is open and is
  listed by `myrmic cells` as a row without a name.
- `POST /chillguard/api` with the header `X-Gateway-Session: <uuid>` and the body
  `{"type":"command","sri":"<srn or sri>","name":"<command>","payload":"<base64 of the payload>"}`
  answers `202` with no body. The cell sees `md.sender` = the session.
  `{"type":"event","name":..,"payload":..}` publishes an event, also `202`.
- Without the header: `401 missing or malformed X-Gateway-Session header`. To a name that is not
  deployed: `502 {"type":"error","message":"cell '<sri>' has no placement"}`.
- A command a cell sends to the session arrives on the stream as
  `data: {"type":"command","sri":"<the cell>","name":"<name>","payload":"<base64>"}`. The asset
  cell's answer to a `get_status` posted this way came back like that.
- Without `.api()` in the mount, `GET /chillguard/api` is a `404` and a `POST` a `405`.
- **The API path is not limited to the cell that mounts it.** Through the dashboard cell's mount
  an HTTP client sent `acknowledge` to the risk monitor, `get_status` to the asset cell and
  `simulate_scenario` to the sensor, and published an event under a name of its choice. One
  cell mounting `.api()` opens every command of every cell, and every event, to whoever can
  reach the gateway's port. The dashboard cell mounts it only with `dev_controls`
  (docs/security.md).

**Queries between cells** (closes an open question) [local]

- The dashboard cell sends `get_status` and `get_incidents` with `myrmic_sdk::send`; the asset
  and the monitor see its SRI as `md.sender` and answer with
  `Callback::to(reply_to)?.invoke(md.sender, &answer)` (`... via=command` in their logs). The
  answer arrives as a command of that name; the asset logged its first one 14 ms after the
  dashboard's init.
- **A command from a cell to a cell that is not deployed is kept, not refused.** `send` returns
  `Ok` and nothing is logged (the CLI, by contrast, fails with `has no placement`). When the
  asset cell was deployed again after 24 s, the twelve `get_status` the dashboard had sent in
  the meantime were all delivered within 60 ms of its init. A poller that asks on every tick
  queues one command per tick for as long as its target is away; the dashboard cell asks a cell
  again only once it has answered (`routes.rs`), which left exactly one.

**The blob store keeps everything** [docs] + [local]

- [docs] "Nothing ever deletes stored content. Once its last path is gone it cannot be reached
  any more, and it stays on the node." Deleting or replacing a path changes only the name.
- [local] It does not store equal content once, either. A handler that wrote one of three fixed
  strings to the same path every 2 s left 2, then 9, then 15 copies of each in
  `myrmic database export gw/gateway-assets/<sri>` over 80 s. Five deployments of the dashboard
  cell left five copies of its page there.
- [local] Cost of the tutorials' pattern (rewrite a JSON file on every update), from the size of
  that export: a 328-byte status snapshot adds about 0.8 KiB, for good. Writing both answers
  every 2 s added about 80 KiB a minute; writing the snapshot alone, 24 KiB a minute.
- [local] On disk the short-run cost is ten times that. `du` of the runtime's `db` folder over
  240 s each, e2e payloads at one reading a second: 3.5 kB/s with the three cells alone,
  7.4 kB/s with the dashboard cell writing one snapshot every 2 s, so about 7.7 kB per refresh.
  A refresh is four commands (two queries, two answers) and one file; most of the difference is
  presumably journal that compaction gives back, but no compaction was seen in these minutes
  (the folder only grew, from 2.5 MiB to 17 MiB in 35 minutes of experiments).
- Consequences for the dashboard cell: it writes an answer only when it says something new (the
  incident list changes rarely; the status snapshot with every reading, and its `as_of` is the
  page's sign of life), the refresh period is part of the init payload, and the cost is stated
  in config/README.md. The alternative without storage is the message API, with what that opens.

**Smaller facts** [local]

- A 15.6 KB page baked in with `include_bytes!` builds and runs with
  `heap_size = 65_536`, `initial_memory = max_memory = 262_144` (the tutorial's values); the
  binary is 228 KB. `BlobStore::put` hands the bytes to the host by reference.
- `static X: InMemory<Vec<T>> = InMemory::new(Vec::new())` holds a cell's working state across
  handlers of one incarnation; `with` hands out `&mut`.
- The page was checked in headless Chromium at 390 px and 1100 px wide (no horizontal overflow,
  no console errors) against the live gateway, including a click on a scenario button.

## 17. The playground: `chillguard start` (2026-10-12) [local]

What `crates/chillguard-cli/src/playground.rs` relies on, each observed.

- `myrmic runtimes start <config.yml> --name <n> --tmp --detached` takes a configuration file
  and `--tmp` together: the file's `zenoh` and `gateway` sections apply and the database is in
  memory (`runtimes <n> info` then says `db ... (missing)`). The command returns once the
  daemon is forked and does not hold a pipe it was given.
- `myrmic runtimes <n> info`: exit 0 with a line `status    running (pid N)` or
  `status    stopped`; exit 1 with `ERROR no runtime "<n>" recorded (never started on this
  machine?)` for a name it has never seen. `runtimes start` under a name that is running:
  exit 1, `ERROR runtime "<n>" already running (pid N)`. `runtimes delete <n>` when nothing of
  that name runs: exit 1, `ERROR no runtime "<n>" found in <pid directory>`.
- The pid directory is `$XDG_RUNTIME_DIR/myrmic`, shared by every runtime of the user whatever
  `XDG_DATA_HOME` says: `myrmic runtimes list` showed another session's test runtime.
- `myrmic deploy <wasm> --name <srn> --init '<json>'` takes the payload inline. A failed
  deployment puts its reason on `Caused by:` lines after `ERROR unable to deploy cell`
  (`application '<srn>' is already deployed`, or `... Cell init error: <the cell's message>`).
- Four deployments took 4 to 5 s, a whole `chillguard start` 4 s to 12 s.
- **A gateway port that is taken is not an error.** A second runtime configured with the
  gateway port of a running one started, reported `status running`, and logged nothing about
  the gateway. Check the port before starting.
- **There is no gateway bind address.** The binary's configuration type for the section has
  one element (`struct GatewayConf with 1 element`, from `strings`), which is `port`; the
  embedded gateway listens on `*:<port>` (section 16).
- **A suspended machine loses the cells and keeps the runtime.** The machine this ran on was
  frozen for an hour with a playground up. On resume the runtime logged
  `sorg_orchestration::supervision: hygiene: cleaning up after dead cell(s) cells=4`, then
  `fencing: killing cell sri=... why=SelfSuperseded` for each cell, and the gateway
  `ignoring route for absent cell ... (mount /chillguard)`. `myrmic cells --once`: `No cells
  registered`; `runtimes <n> info`: still `running`. Deploying the cells again worked and they
  found their state (`resumed=true`), in memory as it was. Nothing brought them back by
  itself. Presumably the cells' leases ran out during the jump of the clock; a production
  host that suspends would need the same redeploy (`make deploy`).
- A stale cell binary shows up as `Cell init error: failed to deserialize json` when its
  payload shape has changed since it was built; `scripts/deploy.sh` guards against that and
  `chillguard start` only hints at it (`make start` rebuilds first).
