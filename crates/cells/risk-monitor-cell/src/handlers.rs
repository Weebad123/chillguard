//! Myrmic handler exports for the cell.
//!
//! Only functions carrying an SDK handler attribute belong here; keep them to
//! decode, call `chillguard-core`, persist, emit.

// The SDK's handler attributes expand to `#[unsafe(no_mangle)] extern "C"`
// exports, which the workspace-wide `unsafe_code = "deny"` rejects. The allow
// is confined to this module; nothing here may contain hand-written `unsafe`.
#![allow(unsafe_code)]

use myrmic_sdk::Metadata;

/// Runs once per incarnation, before any other handler.
#[myrmic_sdk::init]
fn init(md: Metadata) -> myrmic_sdk::Result {
    myrmic_sdk::info!("risk-monitor-cell started (id={})", md.id)?;
    Ok(())
}
