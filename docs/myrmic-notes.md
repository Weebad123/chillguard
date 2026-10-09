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
| Other subcommands present | `flash`, `monitor`, `database`, `replicate`, `tags`, `gateway`, `cells classes/status/teardown`, `telemetry` | Help text read; not exercised. |

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
- `myrmic runtimes start <file.yml>` rejects unknown config keys
  (`unknown field 'bogus_section'`), so a config file that starts is a config file whose keys exist.
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
- Store the sensor's `TimerHandle` in `State`; re-arm timers in `#[init]`; seed state only if absent.
- Time-series publish payloads must stay under 100 bytes: short series names, few short fields.
- All gates run on the pinned nightly through the workspace-root `rust-toolchain.toml`
  (docs/decisions/0002-toolchain-and-workspace.md).
- **Wasm build command for the quality gate:** `make build-cells`, which runs
  `myrmic build crates/cells/<cell>` for each cell.

## 10. Open questions / not yet verified

- `app_specs.yml` build/deploy (`classes`/`instances`) is documented but not exercised.
- Gateway (`myrmic gateway`, `gateway` routes/assets) for a dashboard is documented but not exercised.
- `find_measurement` was not exercised; buffer sizing for responses is the caller's problem.
- Behaviour of `#[evt]` across two different cells was not tested directly (only self-delivery
  and CLI publish). The tutorial shows cross-cell delivery working.
