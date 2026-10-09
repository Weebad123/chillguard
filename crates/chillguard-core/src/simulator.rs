//! Deterministic temperature simulator.
//!
//! A [`Simulator`] stands in for a physical sensor: it plays a [`Scenario`],
//! a list of `(phase, duration)` steps, and produces a
//! [`TemperatureReading`] whenever it is asked for one. It is deterministic.
//! Randomness comes from a generator seeded by the caller, and time is the
//! `now` passed to [`Simulator::next_reading`]; the simulator never reads a
//! clock. The same configuration, scenario, seed and sequence of `now`
//! values always give the same readings, on the host and in a cell.
//!
//! # Thermal model
//!
//! The cabinet is one thermal mass that exchanges heat with one target
//! temperature (Newton's law of cooling, a first-order model):
//!
//! ```text
//! dT/dt = (T_target - T) / tau
//! ```
//!
//! The simulator uses the exact solution of that equation over a step `dt`,
//! so the result does not depend on how often it is sampled:
//!
//! ```text
//! T(t + dt) = T_target + (T(t) - T_target) * exp(-dt / tau)
//! ```
//!
//! After one time constant `tau`, 63 % of the distance to the target is
//! covered; after three, 95 %. The phase selects the target and the time
//! constant:
//!
//! | Phase | `T_target` | `tau` |
//! |---|---|---|
//! | [`Phase::Normal`] | `setpoint_c` | `cooling_time_constant` |
//! | [`Phase::PowerFailure`] | `ambient_c` | `warming_time_constant` |
//! | [`Phase::Recovery`] | `setpoint_c` | `cooling_time_constant` |
//! | [`Phase::Manual`] | none: `T` is set to the given value and held | |
//!
//! `Normal` and `Recovery` follow the same equation. A cabinet at its
//! setpoint stays there, which makes `Normal` noise around the setpoint;
//! `Recovery` is the name a scenario gives to the cool-down after a failure.
//!
//! # Readings
//!
//! `T` is the cabinet temperature without noise. A reading is
//!
//! ```text
//! reading = T + noise,   noise uniform in [-noise_amplitude_c, +noise_amplitude_c)
//! ```
//!
//! The noise is measurement noise: it is drawn per reading and does not feed
//! back into `T`. A [`Phase::Manual`] reading is exactly the set temperature,
//! without noise.
//!
//! # Time
//!
//! The first call to [`Simulator::next_reading`] starts the scenario. Each
//! later call advances it by the time since the latest `now` seen so far. A
//! `now` that is not later than that advances nothing: the reading is stamped
//! with the `now` it was asked for and reports the unchanged cabinet
//! temperature. When the scenario runs out, its last phase continues for as
//! long as readings are requested; an empty scenario is `Normal` throughout.

use alloc::vec::Vec;
use core::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::risk::{ReadingSource, RefrigeratorId, TemperatureReading, Timestamp};

/// What the simulated refrigerator is doing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Running normally: the cabinet is held at the setpoint.
    Normal,
    /// No power: the cabinet warms toward the ambient temperature.
    PowerFailure,
    /// Power is back: the cabinet cools toward the setpoint.
    Recovery,
    /// The cabinet temperature is set to this value, in degrees Celsius, and
    /// held there. Readings carry no noise.
    Manual(f64),
}

/// One step of a [`Scenario`]: a phase and how long it lasts.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScenarioStep {
    /// What the refrigerator does during the step.
    pub phase: Phase,
    /// How long the step lasts. A `Manual` step of zero length sets the
    /// temperature and moves straight on.
    #[serde(with = "crate::duration_millis")]
    pub duration: Duration,
}

/// A script for the simulator: phases played one after the other.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Scenario {
    steps: Vec<ScenarioStep>,
}

impl Scenario {
    /// Temperature the sensor reads while the door stands open in
    /// [`Scenario::short_door_open`], in degrees Celsius.
    pub const DOOR_OPEN_C: f64 = 12.0;

    /// Builds a scenario from `(phase, duration)` pairs.
    #[must_use]
    pub fn new(steps: impl IntoIterator<Item = (Phase, Duration)>) -> Self {
        Self {
            steps: steps
                .into_iter()
                .map(|(phase, duration)| ScenarioStep { phase, duration })
                .collect(),
        }
    }

    /// 24 hours of normal operation.
    #[must_use]
    pub fn normal_day() -> Self {
        Self::new([(Phase::Normal, Duration::from_hours(24))])
    }

    /// 30 minutes of normal operation, a power failure of 2 hours, 2 hours
    /// of recovery, and 30 minutes of normal operation: 5 hours in all.
    #[must_use]
    pub fn power_failure_2h() -> Self {
        Self::new([
            (Phase::Normal, Duration::from_mins(30)),
            (Phase::PowerFailure, Duration::from_hours(2)),
            (Phase::Recovery, Duration::from_hours(2)),
            (Phase::Normal, Duration::from_mins(30)),
        ])
    }

    /// 10 minutes of normal operation, 5 minutes with the door open (warm
    /// air at the sensor, [`Scenario::DOOR_OPEN_C`]), 1 hour of recovery,
    /// and 15 minutes of normal operation: 1 hour 30 minutes in all.
    #[must_use]
    pub fn short_door_open() -> Self {
        Self::new([
            (Phase::Normal, Duration::from_mins(10)),
            (Phase::Manual(Self::DOOR_OPEN_C), Duration::from_mins(5)),
            (Phase::Recovery, Duration::from_hours(1)),
            (Phase::Normal, Duration::from_mins(15)),
        ])
    }

    /// The steps, in the order they are played.
    #[must_use]
    pub fn steps(&self) -> &[ScenarioStep] {
        &self.steps
    }

    /// The sum of all step durations.
    #[must_use]
    pub fn total_duration(&self) -> Duration {
        self.steps.iter().fold(Duration::ZERO, |total, step| {
            total.saturating_add(step.duration)
        })
    }
}

/// Parameters of the thermal model and of the sensor noise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulatorConfig {
    /// The refrigerator the readings are stamped with.
    pub refrigerator_id: RefrigeratorId,
    /// Temperature the refrigerator regulates to, in degrees Celsius. The
    /// simulation starts here.
    pub setpoint_c: f64,
    /// Temperature of the room, in degrees Celsius.
    pub ambient_c: f64,
    /// `tau` while the cabinet warms toward ambient without power.
    #[serde(with = "crate::duration_millis")]
    pub warming_time_constant: Duration,
    /// `tau` while the cabinet cools toward the setpoint with power.
    #[serde(with = "crate::duration_millis")]
    pub cooling_time_constant: Duration,
    /// Largest distance between a reading and the cabinet temperature, in
    /// degrees Celsius. Zero turns the noise off.
    pub noise_amplitude_c: f64,
}

impl SimulatorConfig {
    /// Illustrative defaults for a small refrigerator: setpoint 4 °C, room
    /// at 22 °C, warming time constant 4 hours, cooling time constant
    /// 20 minutes, noise of ±0.2 °C.
    #[must_use]
    pub const fn new(refrigerator_id: RefrigeratorId) -> Self {
        Self {
            refrigerator_id,
            setpoint_c: 4.0,
            ambient_c: 22.0,
            warming_time_constant: Duration::from_hours(4),
            cooling_time_constant: Duration::from_mins(20),
            noise_amplitude_c: 0.2,
        }
    }

    /// Checks that the parameters describe a usable model.
    ///
    /// # Errors
    ///
    /// Returns the first fault found: a non-finite setpoint or ambient
    /// temperature, a zero time constant, or a noise amplitude that is
    /// negative or not finite.
    pub fn validate(&self) -> Result<(), SimulatorError> {
        if !self.setpoint_c.is_finite() || !self.ambient_c.is_finite() {
            return Err(SimulatorError::NonFiniteTemperature);
        }
        if self.warming_time_constant.is_zero() || self.cooling_time_constant.is_zero() {
            return Err(SimulatorError::ZeroTimeConstant);
        }
        if !self.noise_amplitude_c.is_finite() || self.noise_amplitude_c < 0.0 {
            return Err(SimulatorError::InvalidNoiseAmplitude);
        }
        Ok(())
    }
}

/// Why a [`Simulator`] could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SimulatorError {
    /// The setpoint or the ambient temperature is NaN or infinite.
    #[error("setpoint and ambient temperature must be finite numbers")]
    NonFiniteTemperature,
    /// A time constant is zero.
    #[error("time constants must be longer than zero")]
    ZeroTimeConstant,
    /// The noise amplitude is negative, NaN, or infinite.
    #[error("noise amplitude must be a finite number that is zero or positive")]
    InvalidNoiseAmplitude,
    /// A `Manual` step of the scenario sets a temperature that is NaN or
    /// infinite.
    #[error("the manual temperature of scenario step {step} must be a finite number")]
    NonFiniteManualTemperature {
        /// Index of the offending step, counting from zero.
        step: usize,
    },
}

/// A simulated temperature sensor playing a scenario.
///
/// The whole simulator is serde data, generator state included, so a caller
/// can persist it and carry on with the same sequence after a restart.
/// Durations are serialised as whole milliseconds
/// ([`crate::duration_millis`]), so the scenario's step durations and the
/// time constants must be whole milliseconds for the simulator to be saved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Simulator {
    config: SimulatorConfig,
    scenario: Scenario,
    rng: SplitMix64,
    /// Cabinet temperature without noise: `T` of the thermal model.
    temperature_c: f64,
    /// Index of the scenario step being played.
    step_index: usize,
    /// Time already spent in that step.
    #[serde(with = "crate::duration_millis")]
    elapsed_in_step: Duration,
    /// The latest `now` seen; `None` until the first reading.
    last_at: Option<Timestamp>,
}

impl Simulator {
    /// Builds a simulator at the start of `scenario`, with the cabinet at
    /// the setpoint and the noise generator seeded with `seed`.
    ///
    /// # Errors
    ///
    /// Returns a [`SimulatorError`] when `config` is invalid (see
    /// [`SimulatorConfig::validate`]) or a `Manual` step of `scenario` sets a
    /// temperature that is not finite.
    pub fn new(
        config: SimulatorConfig,
        scenario: Scenario,
        seed: u64,
    ) -> Result<Self, SimulatorError> {
        config.validate()?;
        for (step, entry) in scenario.steps.iter().enumerate() {
            if let Phase::Manual(temperature_c) = entry.phase
                && !temperature_c.is_finite()
            {
                return Err(SimulatorError::NonFiniteManualTemperature { step });
            }
        }

        let mut simulator = Self {
            temperature_c: config.setpoint_c,
            config,
            scenario,
            rng: SplitMix64(seed),
            step_index: 0,
            elapsed_in_step: Duration::ZERO,
            last_at: None,
        };
        simulator.enter_step();
        // Moves past any leading steps of zero length.
        simulator.advance(Duration::ZERO);
        Ok(simulator)
    }

    /// Advances the scenario to `now` and returns the reading a sensor would
    /// take at that moment.
    ///
    /// The first call starts the scenario. A `now` that is not later than the
    /// latest one seen advances nothing; the reading is still stamped `now`.
    pub fn next_reading(&mut self, now: Timestamp) -> TemperatureReading {
        if let Some(last_at) = self.last_at {
            self.advance(now.saturating_duration_since(last_at));
            self.last_at = Some(last_at.max(now));
        } else {
            self.last_at = Some(now);
        }

        let temperature_c = match self.phase() {
            Phase::Manual(_) => self.temperature_c,
            Phase::Normal | Phase::PowerFailure | Phase::Recovery => {
                self.temperature_c + self.config.noise_amplitude_c * self.rng.next_signed_unit()
            }
        };
        TemperatureReading {
            refrigerator_id: self.config.refrigerator_id.clone(),
            temperature_c,
            observed_at: now,
            source: ReadingSource::Simulated,
        }
    }

    /// The phase being played.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.scenario
            .steps
            .get(self.step_index)
            .map_or(Phase::Normal, |step| step.phase)
    }

    /// The cabinet temperature without sensor noise, in degrees Celsius.
    #[must_use]
    pub const fn temperature_c(&self) -> f64 {
        self.temperature_c
    }

    /// Plays `dt` of the scenario, crossing as many step boundaries as it
    /// spans. A step covers its start and excludes its end.
    fn advance(&mut self, mut dt: Duration) {
        loop {
            let Some(step) = self.scenario.steps.get(self.step_index) else {
                self.relax(Phase::Normal, dt);
                return;
            };
            let phase = step.phase;
            let remaining = step.duration.saturating_sub(self.elapsed_in_step);
            let is_last = self.step_index + 1 == self.scenario.steps.len();
            if is_last || dt < remaining {
                self.relax(phase, dt);
                self.elapsed_in_step = self.elapsed_in_step.saturating_add(dt);
                return;
            }
            self.relax(phase, remaining);
            dt = dt.saturating_sub(remaining);
            self.step_index += 1;
            self.elapsed_in_step = Duration::ZERO;
            self.enter_step();
        }
    }

    /// Applies what happens the moment a step begins.
    fn enter_step(&mut self) {
        if let Phase::Manual(temperature_c) = self.phase() {
            self.temperature_c = temperature_c;
        }
    }

    /// Moves the cabinet temperature toward the phase's target for `dt`:
    /// `T = T_target + (T - T_target) * exp(-dt / tau)`.
    fn relax(&mut self, phase: Phase, dt: Duration) {
        // Skipped rather than computed, so that no time means no change,
        // to the last bit.
        if dt.is_zero() {
            return;
        }
        let (target_c, tau) = match phase {
            Phase::Normal | Phase::Recovery => {
                (self.config.setpoint_c, self.config.cooling_time_constant)
            }
            Phase::PowerFailure => (self.config.ambient_c, self.config.warming_time_constant),
            Phase::Manual(_) => return,
        };
        let decay = libm::exp(-dt.as_secs_f64() / tau.as_secs_f64());
        self.temperature_c = target_c + (self.temperature_c - target_c) * decay;
    }
}

/// The `SplitMix64` generator (Steele, Lea, Flood 2014).
///
/// Written out here, not taken from a crate, so that the sequence for a seed
/// can never change with a dependency upgrade, and so that the generator's
/// whole state is one serialisable integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value uniform in `[-1, 1)`, built from the top 32 bits.
    fn next_signed_unit(&mut self) -> f64 {
        const TWO_POW_31: f64 = 2_147_483_648.0;
        let top = (self.next_u64() >> 32) as u32;
        f64::from(top) / TWO_POW_31 - 1.0
    }
}

#[cfg(test)]
mod tests {
    // Where temperatures are compared exactly, the test is about values that
    // must be reproduced bit for bit or passed through unchanged.
    #![allow(clippy::float_cmp)]

    use alloc::vec;
    use alloc::vec::Vec;

    use super::*;

    const T0_MS: u64 = 1_791_555_000_000;
    const FRIDGE: &str = "fridge-1";

    fn config() -> SimulatorConfig {
        SimulatorConfig::new(RefrigeratorId::new(FRIDGE))
    }

    fn at_secs(seconds: u64) -> Timestamp {
        Timestamp::from_millis(T0_MS + seconds * 1_000)
    }

    fn at(minute: u64) -> Timestamp {
        at_secs(minute * 60)
    }

    fn simulator(scenario: Scenario, seed: u64) -> Simulator {
        Simulator::new(config(), scenario, seed).expect("config and scenario are valid")
    }

    /// One reading a minute for minutes `0..=minutes`, with the cabinet
    /// temperature behind each reading.
    fn sample(simulator: &mut Simulator, minutes: u64) -> Vec<(TemperatureReading, f64)> {
        (0..=minutes)
            .map(|minute| {
                let reading = simulator.next_reading(at(minute));
                (reading, simulator.temperature_c())
            })
            .collect()
    }

    fn minutes_of(scenario: &Scenario) -> u64 {
        scenario.total_duration().as_secs() / 60
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not within {tolerance} of {expected}"
        );
    }

    #[test]
    fn same_seed_always_gives_the_same_output() {
        for scenario in [
            Scenario::normal_day(),
            Scenario::power_failure_2h(),
            Scenario::short_door_open(),
        ] {
            let minutes = minutes_of(&scenario);
            let first = sample(&mut simulator(scenario.clone(), 42), minutes);
            let second = sample(&mut simulator(scenario.clone(), 42), minutes);
            assert_eq!(first, second, "same seed, same readings, bit for bit");

            let other = sample(&mut simulator(scenario, 43), minutes);
            assert_ne!(first, other, "another seed gives other noise");
            let cabinet = |run: &[(TemperatureReading, f64)]| -> Vec<f64> {
                run.iter()
                    .map(|(_, temperature_c)| *temperature_c)
                    .collect()
            };
            assert_eq!(
                cabinet(&first),
                cabinet(&other),
                "the seed only drives the noise, never the cabinet temperature"
            );
        }
    }

    #[test]
    fn output_for_a_seed_is_pinned() {
        // Reference outputs of SplitMix64. If these change, every stored
        // seed means something else.
        let mut rng = SplitMix64(0);
        assert_eq!(
            [rng.next_u64(), rng.next_u64(), rng.next_u64()],
            [
                0xE220_A839_7B1D_CDAF,
                0x6E78_9E6A_A1B9_65F4,
                0x06C4_5D18_8009_454F
            ]
        );
        let mut rng = SplitMix64(1_234_567);
        assert_eq!(
            [rng.next_u64(), rng.next_u64(), rng.next_u64()],
            [
                6_457_827_717_110_365_317,
                3_203_168_211_198_807_973,
                9_817_491_932_198_370_423
            ]
        );
    }

    #[test]
    fn restart_from_serialized_state_continues_the_same_sequence() {
        let scenario = Scenario::power_failure_2h();
        let minutes = minutes_of(&scenario);
        let uninterrupted = sample(&mut simulator(scenario.clone(), 7), minutes);

        // Save and reload the simulator after every reading.
        let mut restarted = simulator(scenario, 7);
        for (minute, expected) in (0..=minutes).zip(&uninterrupted) {
            let reading = restarted.next_reading(at(minute));
            assert_eq!(reading, expected.0, "minute {minute}");
            let bytes = postcard::to_allocvec(&restarted).expect("serialises");
            restarted = postcard::from_bytes(&bytes).expect("deserialises");
        }
    }

    #[test]
    fn normal_is_noise_around_the_setpoint() {
        let mut simulator = simulator(Scenario::normal_day(), 1);
        let readings = sample(&mut simulator, 24 * 60);

        let amplitude = config().noise_amplitude_c;
        let mut lowest = f64::INFINITY;
        let mut highest = f64::NEG_INFINITY;
        for (reading, cabinet_c) in &readings {
            assert_eq!(*cabinet_c, 4.0, "the cabinet sits on the setpoint");
            assert!((reading.temperature_c - 4.0).abs() <= amplitude);
            lowest = lowest.min(reading.temperature_c);
            highest = highest.max(reading.temperature_c);
        }
        // The noise uses its range and is not stuck on one side.
        assert!(lowest < 4.0 - 0.9 * amplitude, "lowest {lowest}");
        assert!(highest > 4.0 + 0.9 * amplitude, "highest {highest}");
    }

    #[test]
    fn power_failure_warms_monotonically_within_noise_bounds() {
        let scenario = Scenario::new([(Phase::PowerFailure, Duration::from_hours(4))]);
        let mut simulator = simulator(scenario, 3);
        let readings = sample(&mut simulator, 4 * 60);
        let amplitude = config().noise_amplitude_c;

        for pair in readings.windows(2) {
            let ((before, cabinet_before), (after, cabinet_after)) = (&pair[0], &pair[1]);
            assert!(
                cabinet_after > cabinet_before,
                "the cabinet warms at every step: {cabinet_before} then {cabinet_after}"
            );
            assert!(
                after.temperature_c > before.temperature_c - 2.0 * amplitude,
                "a reading falls by no more than the noise allows"
            );
        }
        for (reading, cabinet_c) in &readings {
            assert!((reading.temperature_c - cabinet_c).abs() <= amplitude);
            assert!(*cabinet_c < 22.0, "never past ambient");
        }

        // Closed form: starts at the setpoint, and after one time constant
        // (4 h) has covered 1 - 1/e of the 18 °C to ambient: 22 - 18/e.
        assert_eq!(readings[0].1, 4.0);
        assert_close(readings[4 * 60].1, 15.378_170_058_914, 1e-9);
        // And after 2 h: 22 - 18 * exp(-0.5).
        assert_close(readings[2 * 60].1, 11.082_448_125_173, 1e-9);
    }

    #[test]
    fn recovery_converges_toward_the_setpoint() {
        // Start warm, then cool for ten time constants.
        let scenario = Scenario::new([
            (Phase::Manual(15.0), Duration::ZERO),
            (Phase::Recovery, Duration::from_mins(200)),
        ]);
        let mut simulator = simulator(scenario, 5);
        assert_eq!(simulator.temperature_c(), 15.0);
        let readings = sample(&mut simulator, 200);
        let amplitude = config().noise_amplitude_c;

        for pair in readings.windows(2) {
            let (before, after) = (pair[0].1 - 4.0, pair[1].1 - 4.0);
            assert!(after > 0.0, "the cabinet never undershoots the setpoint");
            assert!(after < before, "every step ends closer to the setpoint");
        }
        // One time constant (20 min) leaves 1/e of the 11 °C excess.
        assert_close(readings[20].1, 4.0 + 11.0 / core::f64::consts::E, 1e-9);
        // Ten time constants leave less than a thousandth of a degree.
        let (last_reading, last_cabinet) = &readings[200];
        assert!(last_cabinet - 4.0 < 1e-3);
        assert!((last_reading.temperature_c - 4.0).abs() <= amplitude + 1e-3);
    }

    #[test]
    fn manual_holds_the_set_temperature_without_noise() {
        let scenario = Scenario::new([
            (Phase::Normal, Duration::from_mins(2)),
            (Phase::Manual(-18.5), Duration::from_mins(3)),
            (Phase::Recovery, Duration::from_mins(5)),
        ]);
        let mut simulator = simulator(scenario, 9);
        let readings = sample(&mut simulator, 6);

        // The step starts at minute 2 and excludes its end at minute 5.
        for (reading, cabinet_c) in &readings[2..5] {
            assert_eq!(reading.temperature_c, -18.5);
            assert_eq!(*cabinet_c, -18.5);
        }
        // The next phase carries on from the manual temperature: recovery
        // begins at minute 5 and has run for one minute at minute 6.
        assert_eq!(readings[5].1, -18.5);
        let expected = 4.0 + (-18.5 - 4.0) * libm::exp(-60.0 / 1200.0);
        assert_close(readings[6].1, expected, 1e-9);
    }

    #[test]
    fn cabinet_temperature_does_not_depend_on_the_sampling_interval() {
        let scenario = Scenario::power_failure_2h();
        let end = scenario.total_duration().as_secs();

        // Compared mid-failure (minute 147) and at the end (minute 300).
        for checkpoint in [147 * 60, end] {
            let mut every_second = simulator(scenario.clone(), 1);
            for second in 0..=checkpoint {
                every_second.next_reading(at_secs(second));
            }

            // Every 7 minutes: the phase changes fall inside a step.
            let mut every_7_minutes = simulator(scenario.clone(), 2);
            for second in (0..checkpoint).step_by(7 * 60) {
                every_7_minutes.next_reading(at_secs(second));
            }
            every_7_minutes.next_reading(at_secs(checkpoint));

            // One step across every phase change at once.
            let mut two_calls = simulator(scenario.clone(), 3);
            two_calls.next_reading(at_secs(0));
            two_calls.next_reading(at_secs(checkpoint));

            let reference = every_second.temperature_c();
            assert_close(every_7_minutes.temperature_c(), reference, 1e-9);
            assert_close(two_calls.temperature_c(), reference, 1e-9);
            assert_eq!(every_7_minutes.phase(), every_second.phase());
            assert_eq!(two_calls.phase(), every_second.phase());
        }
    }

    #[test]
    fn phases_follow_the_scenario() {
        let mut simulator = simulator(Scenario::power_failure_2h(), 1);
        let expected = [
            (0, Phase::Normal),
            (29, Phase::Normal),
            (30, Phase::PowerFailure),
            (149, Phase::PowerFailure),
            (150, Phase::Recovery),
            (269, Phase::Recovery),
            (270, Phase::Normal),
            (300, Phase::Normal),
            // Past the end, the last phase continues.
            (10_000, Phase::Normal),
        ];
        for (minute, phase) in expected {
            simulator.next_reading(at(minute));
            assert_eq!(simulator.phase(), phase, "minute {minute}");
        }
    }

    #[test]
    fn last_phase_continues_after_the_scenario_ends() {
        let scenario = Scenario::new([(Phase::PowerFailure, Duration::from_mins(1))]);
        let mut simulator = simulator(scenario, 1);
        simulator.next_reading(at(0));
        // A week later the cabinet has reached the room temperature.
        simulator.next_reading(at(7 * 24 * 60));
        assert_eq!(simulator.phase(), Phase::PowerFailure);
        assert_close(simulator.temperature_c(), 22.0, 1e-9);
    }

    #[test]
    fn empty_scenario_is_normal_throughout() {
        let mut simulator = simulator(Scenario::default(), 1);
        let readings = sample(&mut simulator, 60);
        assert_eq!(simulator.phase(), Phase::Normal);
        assert!(readings.iter().all(|(_, cabinet_c)| *cabinet_c == 4.0));
    }

    #[test]
    fn time_that_does_not_move_forward_advances_nothing() {
        let scenario = Scenario::new([(Phase::PowerFailure, Duration::from_hours(4))]);
        let mut simulator = simulator(scenario.clone(), 1);
        simulator.next_reading(at(0));
        simulator.next_reading(at(60));
        let at_60 = simulator.temperature_c();

        // The clock steps back: the reading says so, the cabinet is unchanged.
        let reading = simulator.next_reading(at(30));
        assert_eq!(reading.observed_at, at(30));
        assert_eq!(simulator.temperature_c(), at_60);

        // Coming back to minute 60 does not replay the 30 minutes.
        simulator.next_reading(at(60));
        assert_eq!(simulator.temperature_c(), at_60);

        // From there on, time counts once.
        simulator.next_reading(at(90));
        let mut reference = Simulator::new(config(), scenario, 1).expect("valid");
        reference.next_reading(at(0));
        reference.next_reading(at(90));
        assert_close(simulator.temperature_c(), reference.temperature_c(), 1e-9);
    }

    #[test]
    fn repeated_readings_at_one_instant_do_not_drift() {
        // 0.1 does not survive `22 + (0.1 - 22)` in floating point, so a
        // step of zero length must not go through the model's arithmetic.
        let scenario = Scenario::new([
            (Phase::Manual(0.1), Duration::ZERO),
            (Phase::PowerFailure, Duration::from_hours(1)),
        ]);
        let mut simulator = simulator(scenario, 1);
        for _ in 0..5 {
            simulator.next_reading(at(0));
            assert_eq!(simulator.temperature_c(), 0.1);
        }
    }

    #[test]
    fn readings_carry_identity_time_and_source() {
        let mut simulator = simulator(Scenario::normal_day(), 1);
        let reading = simulator.next_reading(at(17));
        assert_eq!(reading.refrigerator_id, RefrigeratorId::new(FRIDGE));
        assert_eq!(reading.observed_at, at(17));
        assert_eq!(reading.source, ReadingSource::Simulated);
    }

    #[test]
    fn noise_can_be_turned_off() {
        let config = SimulatorConfig {
            noise_amplitude_c: 0.0,
            ..config()
        };
        let mut simulator = Simulator::new(config, Scenario::power_failure_2h(), 1).expect("valid");
        for minute in 0..=300 {
            let reading = simulator.next_reading(at(minute));
            assert_eq!(reading.temperature_c, simulator.temperature_c());
        }
    }

    #[test]
    fn presets_have_the_documented_shape() {
        struct Case {
            name: &'static str,
            scenario: Scenario,
            steps: Vec<(Phase, Duration)>,
            total: Duration,
        }
        let cases = [
            Case {
                name: "normal_day",
                scenario: Scenario::normal_day(),
                steps: vec![(Phase::Normal, Duration::from_hours(24))],
                total: Duration::from_hours(24),
            },
            Case {
                name: "power_failure_2h",
                scenario: Scenario::power_failure_2h(),
                steps: vec![
                    (Phase::Normal, Duration::from_mins(30)),
                    (Phase::PowerFailure, Duration::from_hours(2)),
                    (Phase::Recovery, Duration::from_hours(2)),
                    (Phase::Normal, Duration::from_mins(30)),
                ],
                total: Duration::from_hours(5),
            },
            Case {
                name: "short_door_open",
                scenario: Scenario::short_door_open(),
                steps: vec![
                    (Phase::Normal, Duration::from_mins(10)),
                    (Phase::Manual(12.0), Duration::from_mins(5)),
                    (Phase::Recovery, Duration::from_hours(1)),
                    (Phase::Normal, Duration::from_mins(15)),
                ],
                total: Duration::from_mins(90),
            },
        ];
        for case in cases {
            let steps: Vec<(Phase, Duration)> = case
                .scenario
                .steps()
                .iter()
                .map(|step| (step.phase, step.duration))
                .collect();
            assert_eq!(steps, case.steps, "{}", case.name);
            assert_eq!(case.scenario.total_duration(), case.total, "{}", case.name);
            assert_eq!(case.scenario, Scenario::new(case.steps), "{}", case.name);
        }
    }

    #[test]
    fn invalid_parameters_are_rejected() {
        struct Case {
            name: &'static str,
            config: SimulatorConfig,
            scenario: Scenario,
            error: SimulatorError,
        }
        let cases = [
            Case {
                name: "NaN setpoint",
                config: SimulatorConfig {
                    setpoint_c: f64::NAN,
                    ..config()
                },
                scenario: Scenario::normal_day(),
                error: SimulatorError::NonFiniteTemperature,
            },
            Case {
                name: "infinite ambient",
                config: SimulatorConfig {
                    ambient_c: f64::INFINITY,
                    ..config()
                },
                scenario: Scenario::normal_day(),
                error: SimulatorError::NonFiniteTemperature,
            },
            Case {
                name: "zero warming time constant",
                config: SimulatorConfig {
                    warming_time_constant: Duration::ZERO,
                    ..config()
                },
                scenario: Scenario::normal_day(),
                error: SimulatorError::ZeroTimeConstant,
            },
            Case {
                name: "zero cooling time constant",
                config: SimulatorConfig {
                    cooling_time_constant: Duration::ZERO,
                    ..config()
                },
                scenario: Scenario::normal_day(),
                error: SimulatorError::ZeroTimeConstant,
            },
            Case {
                name: "negative noise",
                config: SimulatorConfig {
                    noise_amplitude_c: -0.1,
                    ..config()
                },
                scenario: Scenario::normal_day(),
                error: SimulatorError::InvalidNoiseAmplitude,
            },
            Case {
                name: "NaN noise",
                config: SimulatorConfig {
                    noise_amplitude_c: f64::NAN,
                    ..config()
                },
                scenario: Scenario::normal_day(),
                error: SimulatorError::InvalidNoiseAmplitude,
            },
            Case {
                name: "NaN manual temperature",
                config: config(),
                scenario: Scenario::new([
                    (Phase::Normal, Duration::from_mins(1)),
                    (Phase::Manual(f64::NAN), Duration::from_mins(1)),
                ]),
                error: SimulatorError::NonFiniteManualTemperature { step: 1 },
            },
        ];
        for case in cases {
            assert_eq!(
                Simulator::new(case.config, case.scenario, 0),
                Err(case.error),
                "{}",
                case.name
            );
        }
    }
}
