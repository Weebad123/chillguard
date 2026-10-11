//! Myrmic handler exports for the cell.
//!
//! Only functions carrying an SDK handler attribute belong here; keep them to
//! decode, call `chillguard-core`, persist, emit.

// The SDK's handler attributes expand to `#[unsafe(no_mangle)] extern "C"`
// exports, which the workspace-wide `unsafe_code = "deny"` rejects. The allow
// is confined to this module; nothing here may contain hand-written `unsafe`.
#![allow(unsafe_code)]

use core::time::Duration;

use chillguard_core::risk::Timestamp;
use chillguard_core::simulator::{Simulator, SimulatorConfig};
use chillguard_protocol::Envelope;
use chillguard_protocol::commands::{ScenarioPreset, SimulateScenario};
use chillguard_protocol::events::ReadingObserved;
use myrmic_sdk::db::state::State;
use myrmic_sdk::{Bytes, Callback, Metadata, Result};

use crate::config::SensorConfig;

/// The event readings are published under. A test pins it to the protocol.
const READING_OBSERVED: &str = "reading_observed";

/// The init payload, kept so that later handlers can read it.
const CONFIG: State<SensorConfig> = State::new_const("config");
/// The simulator between two ticks: scenario position, cabinet temperature,
/// noise generator.
const SIMULATOR: State<Simulator> = State::new_const("simulator");

fn new_simulator(config: &SensorConfig, preset: ScenarioPreset, seed: u64) -> Result<Simulator> {
    let thermal = SimulatorConfig {
        time_scale: config.time_scale,
        ..SimulatorConfig::new(config.refrigerator_id.clone())
    };
    Simulator::new(thermal, preset.scenario(), seed).map_err(|_| "invalid simulator configuration")
}

/// Runs once per incarnation, before any other handler: on deploy and again
/// on every restart.
#[myrmic_sdk::init]
fn init(md: Metadata, config: SensorConfig) -> Result {
    config.validate()?;
    // State survives a restart. Carry on with the stored simulator when the
    // configuration is the one it was built for; start afresh when the
    // configuration changed or the stored state cannot be read.
    let resumes = matches!(CONFIG.load(), Ok(Some(stored)) if stored == config)
        && matches!(SIMULATOR.load(), Ok(Some(_)));
    if !resumes {
        SIMULATOR.save(&new_simulator(
            &config,
            ScenarioPreset::NormalDay,
            config.seed,
        )?)?;
        CONFIG.save(&config)?;
    }
    // Timers die with the incarnation (observed; docs/myrmic-notes.md), so
    // the interval is armed on every init and nothing is left over to cancel.
    // The handle is only good for cancelling, and this timer runs for as long
    // as the cell does.
    let period = Duration::from_millis(config.interval_ms);
    let _ = myrmic_sdk::interval(Callback::of::<tick>(), period).build()?;
    myrmic_sdk::info!(
        "sensor-cell started id={} refrigerator_id={} interval_ms={} dev_mode={} time_scale={} \
         resumed={resumes}",
        md.id,
        config.refrigerator_id,
        config.interval_ms,
        config.dev_mode,
        config.time_scale,
    )?;
    Ok(())
}

/// The periodic handler: one reading per tick.
#[myrmic_sdk::cmd]
fn tick(_md: Metadata) -> Result {
    let now = u64::try_from(myrmic_sdk::now()?.as_millis()).map_err(|_| "clock out of range")?;
    let mut simulator = SIMULATOR.load()?.ok_or("simulator state is missing")?;
    let reading = simulator.next_reading(Timestamp::from_millis(now));
    SIMULATOR.save(&simulator)?;

    let event = Envelope::new(ReadingObserved { reading });
    myrmic_sdk::publish(READING_OBSERVED, &event)?;
    // Logged last: the line stands for a reading that was saved and published.
    let reading = &event.body.reading;
    myrmic_sdk::info!(
        "reading refrigerator_id={} temperature_c={:.3} observed_at={} phase={:?}",
        reading.refrigerator_id,
        reading.temperature_c,
        reading.observed_at.as_millis(),
        simulator.phase(),
    )?;
    Ok(())
}

/// Development command: restart the simulator on a preset scenario.
///
/// Takes the raw payload and decodes it here. A command that returns an error
/// is redelivered every few seconds without end, so one that can never
/// succeed (malformed, another schema version, dev mode off) is logged and
/// dropped with `Ok`.
#[myrmic_sdk::cmd]
fn simulate_scenario(_md: Metadata, payload: Bytes) -> Result {
    let SimulateScenario { preset, seed } = match Envelope::decode(&payload) {
        Ok(envelope) => envelope.body,
        Err(error) => {
            myrmic_sdk::warn!("simulate_scenario rejected reason=\"{error}\"")?;
            return Ok(());
        }
    };
    let config = CONFIG.load()?.ok_or("configuration is missing")?;
    if !config.dev_mode {
        myrmic_sdk::warn!(
            "simulate_scenario rejected refrigerator_id={} preset={preset} reason=dev_mode_off",
            config.refrigerator_id
        )?;
        return Ok(());
    }
    SIMULATOR.save(&new_simulator(&config, preset, seed)?)?;
    myrmic_sdk::info!(
        "scenario started refrigerator_id={} preset={preset} seed={seed}",
        config.refrigerator_id
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chillguard_protocol::{Kind, Payload};
    use myrmic_sdk::Handler;

    use super::*;

    /// The names in this file are literals because the handler attributes
    /// need them to be; this ties them to the protocol.
    #[test]
    fn wire_names_match_the_protocol() {
        assert_eq!(ReadingObserved::KIND, Kind::Event(READING_OBSERVED));
        assert_eq!(
            SimulateScenario::KIND,
            Kind::Command(<simulate_scenario as Handler>::NAME)
        );
    }
}
