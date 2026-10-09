//! Temperature sensor cell.
//!
//! Thin Myrmic adapter that will produce timestamped temperature readings at
//! a configurable interval, simulated at first. It holds no business rules:
//! those live in `chillguard-core`.
//!
//! Stub: the cell only logs that it started.

#![no_std]

mod handlers;
