# 0001: Pure core, thin cells

Status: accepted (2026-10-09)

## Context

ChillGuard's value is a small set of rules: when a reading is out of range, how
long an excursion has lasted, when a warning becomes critical, what counts as a
recovery, when a sensor is stale. These rules must be correct and must be tested
for scenarios that are awkward to stage on a live runtime: long excursions,
gaps in readings, restarts, duplicate deliveries.

The Myrmic runtime shapes how code inside a cell behaves (docs/myrmic-notes.md):

- A cell is a `no_std` WebAssembly module. It only runs inside a runtime, so
  testing a rule that lives in a handler means build, deploy, send, read logs.
- A command whose handler fails is delivered again, without a retry bound. The
  same logical work can reach a handler twice.
- Time comes from a host call (`myrmic_sdk::now()`), which does not exist in a
  host test.
- The SDK is a 0.x developer preview. Its API and guarantees will change.

## Decision

Split the code into three layers with one-way dependencies.

```
cells (sensor, fridge-asset, risk-monitor)
  ├── chillguard-core       business rules
  └── chillguard-protocol   wire types + encoding
```

**`chillguard-core` holds every business rule and nothing else.** It has no
Myrmic dependency, performs no I/O, and never reads a clock: time is always a
parameter. Its functions take the current state and an input and return the
new state and what to emit. It is `no_std` with `alloc` so that cells can link
it, and it is tested natively with `cargo test`.

**`chillguard-protocol` holds the payload types that cross a cell boundary,**
with their wire encoding. It contains data shapes, no rules. It depends on
`myrmic-sdk` for one reason: a payload type gets its encoding by implementing
the SDK's `Encoder`/`Decoder` traits (`#[derive(myrmic_sdk::Message)]`), and
Rust only allows that in the crate that defines the type.

**Cells are thin adapters.** A handler does four things: decode the message,
call core, persist the result, emit what core returned. A cell owns the Myrmic
specifics: handler attributes, state handles, timers, the clock, logging.

Each piece of state has exactly one cell that writes it.

## Consequences

- Rules are tested in milliseconds on the host, including time-dependent and
  duplicate-delivery scenarios, by passing timestamps in.
- Idempotency is decided in core, where it can be tested, instead of in each
  handler.
- A Myrmic SDK upgrade touches the cells and the protocol derives. Core does
  not change.
- Core and protocol must stay `no_std`-compatible, and so must every
  dependency they take. The Wasm build of the cells is the check for this.
- Cells need the SDK's handler macros, which generate `#[unsafe(no_mangle)]`
  exports. The workspace denies `unsafe_code`, so each cell keeps its handlers
  in a private `handlers` module that allows it. Code in that module is not
  covered by the lint; keeping handlers thin keeps that surface small.
- Cell behaviour that core cannot see (wiring, encoding, persistence, timers)
  is only covered by the end-to-end test against a real runtime.

## Open point

Whether protocol types reuse core's domain newtypes or stay raw and are
converted at the cell boundary is not decided. Today the protocol crate does
not depend on core. Decide when the first payload types are written.
