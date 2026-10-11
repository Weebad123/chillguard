//! Temperature sensor cell.
//!
//! Thin Myrmic adapter around `chillguard_core::simulator`: on a timer it
//! reads the clock, asks the simulator for a reading, and publishes it as a
//! `reading_observed` event. It holds no business rules and no thermal
//! model: those live in `chillguard-core`.
//!
//! The cell is configured once, at deploy time, through its init payload
//! (`config::SensorConfig`). With `dev_mode` on it also accepts the
//! `simulate_scenario` command.

#![no_std]

mod config;
mod handlers;
