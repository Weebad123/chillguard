//! Risk monitor cell.
//!
//! Thin Myrmic adapter around `chillguard_core::risk`. It holds one
//! refrigerator's [`MonitorState`](chillguard_core::risk::MonitorState) in
//! persistent state, runs `risk::evaluate` for every reading the asset cell
//! accepts and for every tick of its own timer, publishes what core returns
//! as `status_changed`, `alert_raised` and `incident_recovered`, and keeps
//! the incidents in a table for `get_incidents` and `acknowledge`.
//!
//! It holds no rules: when a status changes, when an alert is raised and
//! when a sensor is stale are decided in `chillguard-core`. See
//! `docs/decisions/0005-state-ownership.md`.

#![no_std]

mod config;
mod handlers;
