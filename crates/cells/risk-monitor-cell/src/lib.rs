//! Risk monitor cell.
//!
//! Thin Myrmic adapter that will evaluate readings against a refrigerator's
//! limits and emit warning, critical, and recovery events. It holds no
//! business rules: those live in `chillguard-core`.
//!
//! Stub: the cell only logs that it started.

#![no_std]

mod handlers;
