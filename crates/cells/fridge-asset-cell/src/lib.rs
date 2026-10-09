//! Refrigerator asset cell.
//!
//! Thin Myrmic adapter that will own one refrigerator's current state and
//! configuration: the latest reading, the configured limits, and the
//! exposure-tracking state. It holds no business rules: those live in
//! `chillguard-core`.
//!
//! Stub: the cell only logs that it started.

#![no_std]

mod handlers;
