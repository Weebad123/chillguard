//! The command line: arguments and their parsing.

// The doc comments on the argument types are the `--help` text. Backticks
// would be printed as they are, so identifiers in them are left bare.
#![allow(clippy::doc_markdown)]

use std::path::PathBuf;
use std::time::Duration;

use chillguard_protocol::commands::{MAX_INCIDENT_LIMIT, ScenarioPreset};
use clap::{Args, Parser, Subcommand};

/// ChillGuard operator interface.
///
/// Talks to the ChillGuard cells of one refrigerator through the Myrmic CLI,
/// which must be installed and able to reach the runtime.
#[derive(Debug, Clone, PartialEq, Parser)]
#[command(name = "chillguard", version, propagate_version = true)]
pub struct Cli {
    /// Options shared by every command.
    #[command(flatten)]
    pub global: Global,
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// Options shared by every command.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct Global {
    /// Prefix of the cells' names, as given to the deploy script
    /// (`SRN_PREFIX`). The cells are `<prefix>sensor-cell`,
    /// `<prefix>fridge-asset-cell` and `<prefix>risk-monitor-cell`.
    /// [env: CHILLGUARD_PREFIX] [default: none]
    #[arg(long, global = true, value_name = "PREFIX")]
    pub prefix: Option<String>,

    /// Runtime endpoint to connect to instead of discovering one by
    /// multicast, e.g. tcp/127.0.0.1:20123. [env: MYRMIC_CONNECT]
    #[arg(long, global = true, value_name = "ENDPOINT")]
    pub connect: Option<String>,

    /// How long to wait for a cell to answer, e.g. 5s, 500ms, 1m.
    #[arg(long, global = true, value_name = "DURATION", default_value = "10s", value_parser = parse_duration)]
    pub timeout: Duration,

    /// Print JSON instead of text, for scripts.
    #[arg(long, global = true)]
    pub json: bool,

    /// The Myrmic CLI to run. [env: MYRMIC_BIN] [default: myrmic]
    #[arg(long, global = true, value_name = "PATH")]
    pub myrmic: Option<PathBuf>,
}

/// The commands.
#[derive(Debug, Clone, PartialEq, Subcommand)]
pub enum Command {
    /// Show the refrigerator's status, latest reading, limits and open incident.
    Status,
    /// List the most recent incidents, newest first.
    Incidents {
        /// How many incidents to show at most (1 to 100).
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u16).range(1..=i64::from(MAX_INCIDENT_LIMIT)))]
        limit: u16,
    },
    /// Record that a person has seen an incident.
    Ack {
        /// The incident's id, as shown by `incidents`.
        incident_id: String,
    },
    /// Change limits and durations. Settings that are not given keep their value.
    SetLimits(SetLimits),
    /// Restart the simulated sensor on a preset scenario (dev mode only).
    Simulate {
        /// One of: normal_day, power_failure_2h, short_door_open.
        #[arg(value_parser = parse_preset)]
        preset: ScenarioPreset,
        /// Seed for the simulator's noise.
        #[arg(long, default_value_t = 0)]
        seed: u64,
    },
    /// Follow status changes, alerts and recoveries as they happen.
    Watch {
        /// Also show every accepted reading.
        #[arg(long)]
        readings: bool,
        /// Stop after this long instead of running until interrupted.
        #[arg(long = "for", value_name = "DURATION", value_parser = parse_duration)]
        duration: Option<Duration>,
    },
}

/// The settings `set-limits` can change. At least one must be given.
#[derive(Debug, Clone, PartialEq, Args)]
#[group(required = true, multiple = true)]
pub struct SetLimits {
    /// Lowest in-range temperature, in °C.
    #[arg(
        long,
        value_name = "CELSIUS",
        allow_negative_numbers = true,
        conflicts_with = "no_lower"
    )]
    pub lower: Option<f64>,
    /// Remove the lower limit.
    #[arg(long)]
    pub no_lower: bool,
    /// Highest in-range temperature, in °C.
    #[arg(long, value_name = "CELSIUS", allow_negative_numbers = true)]
    pub upper: Option<f64>,
    /// How long out of range before an incident is critical, e.g. 30m.
    #[arg(long, value_name = "DURATION", value_parser = parse_duration)]
    pub critical_after: Option<Duration>,
    /// How long without a reading before the sensor counts as stale, e.g. 5m.
    #[arg(long, value_name = "DURATION", value_parser = parse_duration)]
    pub stale_after: Option<Duration>,
    /// How far inside the limits a reading must be to end an incident, in °C.
    #[arg(long, value_name = "CELSIUS", allow_negative_numbers = true)]
    pub hysteresis: Option<f64>,
}

/// Parses `30m`, `90s`, `1500ms`, `1h 30m`. Durations travel as whole
/// milliseconds, so anything finer is refused here, where the message can
/// name the argument.
fn parse_duration(text: &str) -> Result<Duration, String> {
    let duration = humantime::parse_duration(text).map_err(|error| error.to_string())?;
    if duration.subsec_nanos() % 1_000_000 != 0 {
        return Err("must be a whole number of milliseconds".to_string());
    }
    Ok(duration)
}

/// Parses a preset by its wire name; dashes are accepted for underscores.
fn parse_preset(text: &str) -> Result<ScenarioPreset, String> {
    let name = text.replace('-', "_");
    ScenarioPreset::ALL
        .into_iter()
        .find(|preset| preset.as_str() == name)
        .ok_or_else(|| {
            let names: Vec<&str> = ScenarioPreset::ALL.iter().map(|p| p.as_str()).collect();
            format!("unknown preset; expected one of: {}", names.join(", "))
        })
}

/// The global options with their fallbacks applied: what the rest of the
/// program works with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Prefix of the cells' names; empty for none.
    pub prefix: String,
    /// Runtime endpoint, if one was given.
    pub connect: Option<String>,
    /// How long to wait for an answer.
    pub timeout: Duration,
    /// Whether to print JSON.
    pub json: bool,
    /// The Myrmic CLI to run.
    pub myrmic: PathBuf,
}

impl Settings {
    /// Applies the fallbacks: an option given on the command line wins, then
    /// the environment variable, then the default. `env` looks a variable
    /// up; passing it in keeps this testable without touching the process
    /// environment.
    pub fn resolve(global: &Global, env: impl Fn(&str) -> Option<String>) -> Self {
        let non_empty = |name: &str| env(name).filter(|value| !value.is_empty());
        Self {
            prefix: global
                .prefix
                .clone()
                .or_else(|| non_empty("CHILLGUARD_PREFIX"))
                .unwrap_or_default(),
            connect: global
                .connect
                .clone()
                .or_else(|| non_empty("MYRMIC_CONNECT")),
            timeout: global.timeout,
            json: global.json,
            myrmic: global
                .myrmic
                .clone()
                .or_else(|| non_empty("MYRMIC_BIN").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("myrmic")),
        }
    }

    /// The sensor cell's name.
    #[must_use]
    pub fn sensor(&self) -> String {
        format!("{}sensor-cell", self.prefix)
    }

    /// The asset cell's name.
    #[must_use]
    pub fn asset(&self) -> String {
        format!("{}fridge-asset-cell", self.prefix)
    }

    /// The risk monitor cell's name.
    #[must_use]
    pub fn monitor(&self) -> String {
        format!("{}risk-monitor-cell", self.prefix)
    }
}
