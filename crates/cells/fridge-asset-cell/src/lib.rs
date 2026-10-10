//! Refrigerator asset cell.
//!
//! Thin Myrmic adapter around `chillguard_core::asset`. It owns one
//! refrigerator's configuration and its latest accepted reading, and keeps
//! both in persistent state. It hears every `reading_observed`, keeps the
//! ones core accepts, and publishes each as `reading_accepted` together with
//! the configuration in force, for the risk monitor to evaluate.
//!
//! The status it reports is a view, mirrored from `status_changed` events;
//! the risk monitor decides the status. See
//! `docs/decisions/0005-state-ownership.md`.
//!
//! The cell holds no rules: which reading counts, when a configuration is
//! valid and how the status view moves are decided in `chillguard-core`.

#![no_std]

mod config;
mod handlers;
