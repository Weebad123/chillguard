# 0002: One pinned nightly toolchain, cells as workspace members

Status: accepted (2026-10-09)

## Context

Myrmic 0.7.0 builds cells with `nightly-2026-08-07` for the
`wasm32-unknown-unknown` target, and `myrmic new` writes a
`rust-toolchain.toml` with that channel into every cell crate. Measured on
this machine (docs/myrmic-notes.md):

- `myrmic build` obeys a toolchain file above the crate. A file that pins
  stable makes the Wasm build fail; a file that pins the nightly, or no file,
  works.
- `myrmic build <workspace dir>` treats every member as a cell and fails on
  `chillguard-core`.
- Cell crates build correctly as workspace members with inherited
  dependencies and lints.

## Decision

- One `rust-toolchain.toml` at the workspace root pins `nightly-2026-08-07`
  with `rust-src`, `rustfmt`, `clippy`, and the Wasm target. The per-cell
  files that `myrmic new` generates are removed. Every gate, including the
  native tests of `chillguard-core`, runs on that toolchain.
- The cells are members of the workspace, so they share
  `[workspace.dependencies]`, the lint table, one `Cargo.lock`, and one
  `target/` directory.
- Cells are built one crate at a time: `make build-cells` runs
  `myrmic build crates/cells/<cell>` for each.

## Consequences

- One compiler for everything: what is linted and tested is what is shipped.
- The pure core is compiled with a nightly. It must not use `#![feature]`
  attributes; nothing enforces that except review.
- A Myrmic upgrade that moves the pinned nightly is a one-line change here,
  and may surface new clippy lints across the workspace.
- A cell added with `myrmic new` needs three follow-ups: delete its
  `rust-toolchain.toml` and `.gitignore`, switch its manifest to the
  workspace-inherited form, and add it to `CELLS` in the Makefile and in
  `scripts/cells.sh`.
