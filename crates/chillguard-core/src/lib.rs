//! ChillGuard domain logic.
//!
//! This crate holds every business rule: temperature limits, excursion
//! tracking, alert escalation, and recovery. It is pure: it has no Myrmic
//! dependency, performs no I/O, and never reads a clock. Time always comes in
//! as a parameter, so every rule can be tested deterministically on the host.
//!
//! It also holds the temperature [`simulator`] that stands in for a physical
//! sensor. The simulator obeys the same constraints: seeded randomness, time
//! as a parameter.
//!
//! The crate is `no_std` (with `alloc`) because the cells that call it are
//! `no_std` WebAssembly modules; see `docs/decisions/0001-core-vs-cells.md`.
//!
//! ChillGuard records observations. Nothing in this crate may claim that food
//! or medicine is safe.

#![no_std]

extern crate alloc;

pub mod duration_millis;
pub mod risk;
pub mod simulator;
