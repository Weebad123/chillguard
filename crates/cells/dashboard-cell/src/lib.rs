//! Dashboard cell.
//!
//! Thin Myrmic adapter between the refrigerator cells and a browser. It
//! puts one web page into its gateway asset store and mounts it at
//! `/chillguard`; a Myrmic gateway serves it over HTTP. On a timer it asks
//! each configured asset cell for its status snapshot and each risk monitor
//! for its incidents, and stores the answers, as they arrived, as JSON
//! files beside the page. The page polls those files.
//!
//! The cell owns no truth and decides nothing about a refrigerator: the
//! files are a read model, and deleting the cell loses nothing. It never
//! sends a command that changes anything. What it does decide is what the
//! page may go on showing: the answer of a cell that has stopped answering
//! is removed, so that the page says so instead of looking current.
//!
//! With `dev_controls` on it also mounts the gateway's message API, which
//! is what lets the page send `simulate_scenario` to a sensor cell; see
//! `docs/security.md` for what else that opens. The design and its costs
//! are in `docs/decisions/0006-web-dashboard.md`.

#![no_std]

mod config;
mod handlers;
mod routes;
