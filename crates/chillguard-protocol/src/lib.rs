//! Message, event, and command types shared by the ChillGuard cells.
//!
//! Every payload that crosses a cell boundary is defined here, together with
//! its wire encoding, so that the sender and the receiver cannot disagree on
//! a shape. The types carry data only; business rules live in
//! `chillguard-core`.
//!
//! This crate depends on `myrmic-sdk` because the SDK's `Encoder`/`Decoder`
//! traits must be implemented in the crate that defines a payload type; see
//! `docs/decisions/0001-core-vs-cells.md`.

#![no_std]
