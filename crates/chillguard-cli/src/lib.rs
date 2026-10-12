//! ChillGuard operator interface.
//!
//! A native command-line client for the ChillGuard cells. It reaches them
//! the way the project's notes record as verified for a client outside the
//! swarm: by running the `myrmic` CLI (`send` and `subscribe`).
//!
//! - A command (`ack`, `set-limits`, `simulate`) is a `myrmic send`.
//! - A query (`status`, `incidents`) is a `myrmic send` carrying a one-off
//!   `reply_to` name. A caller that is not a cell has no identity to send a
//!   reply to, so the cells publish the answer as an event under that name,
//!   and this client reads it from a `myrmic subscribe` it started first.
//! - `watch` is a `myrmic subscribe` on the monitor's events.
//! - `start` and `stop` run a playground on this machine: a throwaway
//!   runtime with the cells and the web dashboard ([`playground`]).
//!
//! Everything that crosses the wire is an [`Envelope`](chillguard_protocol::Envelope)
//! from `chillguard-protocol`, validated in both directions.
//!
//! The modules separate what can be tested without a runtime from what
//! cannot: [`cli`] (arguments), [`wire`] (the subscriber's output format)
//! and [`render`] (what is printed) are pure; [`myrmic`] runs processes.

pub mod cli;
pub mod client;
pub mod error;
pub mod myrmic;
pub mod playground;
pub mod render;
pub mod run;
pub mod wire;

pub use error::Error;
