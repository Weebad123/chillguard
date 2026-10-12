//! Argument parsing: what the command line accepts, and what it turns into.

// The project standard exempts tests from the unwrap/expect/panic ban
// (clippy.toml), but clippy only recognises `#[test]` functions, not the
// helpers of an integration test file.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::time::Duration;

use chillguard_cli::cli::{Cli, Command, Global, SetLimits, Settings, Start};
use chillguard_cli::run::merge_limits;
use chillguard_core::risk::{RefrigeratorConfig, RefrigeratorId};
use chillguard_protocol::commands::ScenarioPreset;
use clap::{CommandFactory, Parser};

fn parse(line: &str) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(line.split_whitespace())
}

fn command(line: &str) -> Command {
    parse(line)
        .unwrap_or_else(|error| panic!("`{line}` should parse: {error}"))
        .command
}

fn rejected(line: &str) -> String {
    match parse(line) {
        Ok(cli) => panic!("`{line}` should be rejected, parsed as {cli:?}"),
        Err(error) => error.to_string(),
    }
}

const NO_LIMITS: SetLimits = SetLimits {
    lower: None,
    no_lower: false,
    upper: None,
    critical_after: None,
    stale_after: None,
    hysteresis: None,
};

#[test]
fn clap_definition_is_consistent() {
    Cli::command().debug_assert();
}

#[test]
fn every_command_parses() {
    struct Case {
        line: &'static str,
        expected: Command,
    }
    let cases = [
        Case {
            line: "chillguard status",
            expected: Command::Status,
        },
        Case {
            line: "chillguard incidents",
            expected: Command::Incidents { limit: 10 },
        },
        Case {
            line: "chillguard incidents --limit 100",
            expected: Command::Incidents { limit: 100 },
        },
        Case {
            line: "chillguard ack fridge-1-1791599824157",
            expected: Command::Ack {
                incident_id: "fridge-1-1791599824157".to_string(),
            },
        },
        Case {
            line: "chillguard set-limits --upper 10",
            expected: Command::SetLimits(SetLimits {
                upper: Some(10.0),
                ..NO_LIMITS
            }),
        },
        Case {
            line: "chillguard set-limits --lower -18.5 --upper -15 --critical-after 30m --stale-after 90s --hysteresis 0.5",
            expected: Command::SetLimits(SetLimits {
                lower: Some(-18.5),
                no_lower: false,
                upper: Some(-15.0),
                critical_after: Some(Duration::from_mins(30)),
                stale_after: Some(Duration::from_secs(90)),
                hysteresis: Some(0.5),
            }),
        },
        Case {
            line: "chillguard set-limits --no-lower",
            expected: Command::SetLimits(SetLimits {
                no_lower: true,
                ..NO_LIMITS
            }),
        },
        Case {
            line: "chillguard set-limits --critical-after 1500ms",
            expected: Command::SetLimits(SetLimits {
                critical_after: Some(Duration::from_millis(1500)),
                ..NO_LIMITS
            }),
        },
        Case {
            line: "chillguard simulate power_failure_2h",
            expected: Command::Simulate {
                preset: ScenarioPreset::PowerFailure2h,
                seed: 0,
            },
        },
        Case {
            line: "chillguard simulate short-door-open --seed 42",
            expected: Command::Simulate {
                preset: ScenarioPreset::ShortDoorOpen,
                seed: 42,
            },
        },
        Case {
            line: "chillguard simulate normal_day",
            expected: Command::Simulate {
                preset: ScenarioPreset::NormalDay,
                seed: 0,
            },
        },
        Case {
            line: "chillguard watch",
            expected: Command::Watch {
                readings: false,
                duration: None,
            },
        },
        Case {
            line: "chillguard watch --readings --for 30s",
            expected: Command::Watch {
                readings: true,
                duration: Some(Duration::from_secs(30)),
            },
        },
    ];
    for case in cases {
        assert_eq!(command(case.line), case.expected, "{}", case.line);
    }
}

#[test]
fn a_duration_with_several_units_is_one_argument() {
    let cli =
        Cli::try_parse_from(["chillguard", "set-limits", "--critical-after", "1h 30m"]).unwrap();
    assert_eq!(
        cli.command,
        Command::SetLimits(SetLimits {
            critical_after: Some(Duration::from_mins(90)),
            ..NO_LIMITS
        })
    );
}

#[test]
fn what_cannot_be_meant_is_rejected() {
    struct Case {
        line: &'static str,
        says: &'static str,
    }
    let cases = [
        Case {
            line: "chillguard",
            says: "Usage",
        },
        Case {
            line: "chillguard explode",
            says: "unrecognized subcommand",
        },
        Case {
            line: "chillguard incidents --limit 0",
            says: "0 is not in 1..=100",
        },
        Case {
            line: "chillguard incidents --limit 101",
            says: "101 is not in 1..=100",
        },
        Case {
            line: "chillguard incidents --limit many",
            says: "invalid value 'many'",
        },
        Case {
            line: "chillguard ack",
            says: "<INCIDENT_ID>",
        },
        Case {
            line: "chillguard set-limits",
            says: "required arguments were not provided",
        },
        Case {
            line: "chillguard set-limits --lower 2 --no-lower",
            says: "cannot be used with",
        },
        Case {
            line: "chillguard set-limits --upper warm",
            says: "invalid value 'warm'",
        },
        Case {
            line: "chillguard set-limits --critical-after 30",
            says: "time unit needed",
        },
        Case {
            line: "chillguard set-limits --stale-after 1500us",
            says: "whole number of milliseconds",
        },
        Case {
            line: "chillguard simulate",
            says: "<PRESET>",
        },
        Case {
            line: "chillguard simulate earthquake",
            says: "expected one of: normal_day, power_failure_2h, short_door_open",
        },
        Case {
            line: "chillguard simulate normal_day --seed -1",
            says: "unexpected argument '-1'",
        },
        Case {
            line: "chillguard watch --for soon",
            says: "invalid value 'soon'",
        },
        Case {
            line: "chillguard status --timeout fast",
            says: "invalid value 'fast'",
        },
    ];
    for case in cases {
        let message = rejected(case.line);
        assert!(
            message.contains(case.says),
            "`{}` was rejected with:\n{message}\nexpected it to mention: {}",
            case.line,
            case.says
        );
    }
}

#[test]
fn start_and_stop_parse() {
    struct Case {
        line: &'static str,
        expected: Command,
    }
    let cases = [
        Case {
            line: "chillguard start",
            expected: Command::Start(Start::default()),
        },
        Case {
            line: "chillguard start --port 9000 --no-open --cells ./built --swarm-port 21000 --name demo-2",
            expected: Command::Start(Start {
                port: Some(9000),
                no_open: true,
                cells: Some(PathBuf::from("./built")),
                swarm_port: Some(21000),
                name: Some("demo-2".to_string()),
            }),
        },
        Case {
            line: "chillguard stop",
            expected: Command::Stop {
                name: "chillguard-play".to_string(),
            },
        },
        Case {
            line: "chillguard stop --name demo-2",
            expected: Command::Stop {
                name: "demo-2".to_string(),
            },
        },
    ];
    for case in cases {
        assert_eq!(command(case.line), case.expected, "{}", case.line);
    }
}

#[test]
fn start_and_stop_reject_ports_and_names_they_cannot_use() {
    struct Case {
        line: &'static str,
        says: &'static str,
    }
    let cases = [
        Case {
            line: "chillguard start --port 80",
            says: "80 is not in 1024..=65535",
        },
        Case {
            line: "chillguard start --port 70000",
            says: "70000 is not in 1024..=65535",
        },
        Case {
            line: "chillguard start --swarm-port http",
            says: "invalid value 'http'",
        },
        Case {
            line: "chillguard start --name ../elsewhere",
            says: "lower-case letters, digits",
        },
        Case {
            line: "chillguard stop --name Play",
            says: "lower-case letters, digits",
        },
        Case {
            line: "chillguard stop --name",
            says: "a value is required",
        },
    ];
    for case in cases {
        let message = rejected(case.line);
        assert!(
            message.contains(case.says),
            "`{}` was rejected with:\n{message}\nexpected it to mention: {}",
            case.line,
            case.says
        );
    }
}

#[test]
fn global_options_work_before_and_after_the_command() {
    let expected = Global {
        prefix: Some("site-a-".to_string()),
        connect: Some("tcp/127.0.0.1:20123".to_string()),
        timeout: Duration::from_millis(500),
        json: true,
        myrmic: Some(PathBuf::from("/opt/myrmic")),
    };
    for line in [
        "chillguard --prefix site-a- --connect tcp/127.0.0.1:20123 --timeout 500ms --json --myrmic /opt/myrmic status",
        "chillguard status --prefix site-a- --connect tcp/127.0.0.1:20123 --timeout 500ms --json --myrmic /opt/myrmic",
        "chillguard --json status --timeout 500ms --prefix site-a- --myrmic /opt/myrmic --connect tcp/127.0.0.1:20123",
    ] {
        let cli = parse(line).unwrap();
        assert_eq!(cli.global, expected, "{line}");
        assert_eq!(cli.command, Command::Status, "{line}");
    }
}

#[test]
fn global_options_have_defaults() {
    let cli = parse("chillguard status").unwrap();
    assert_eq!(
        cli.global,
        Global {
            prefix: None,
            connect: None,
            timeout: Duration::from_secs(10),
            json: false,
            myrmic: None,
        }
    );
}

#[test]
fn settings_prefer_the_command_line_then_the_environment_then_defaults() {
    let env = |name: &str| match name {
        "CHILLGUARD_PREFIX" => Some("env-".to_string()),
        "MYRMIC_CONNECT" => Some("tcp/10.0.0.1:1".to_string()),
        "MYRMIC_BIN" => Some("/env/myrmic".to_string()),
        _ => None,
    };
    let none = |_: &str| None;
    let empty = |_: &str| Some(String::new());

    // Nothing given anywhere.
    let bare = parse("chillguard status").unwrap().global;
    let settings = Settings::resolve(&bare, none);
    assert_eq!(settings.prefix, "");
    assert_eq!(settings.connect, None);
    assert_eq!(settings.myrmic, PathBuf::from("myrmic"));
    assert_eq!(settings.timeout, Duration::from_secs(10));
    assert!(!settings.json);
    assert_eq!(settings.home, std::env::temp_dir().join("chillguard"));
    // An empty variable is no setting.
    assert_eq!(Settings::resolve(&bare, empty), settings);

    // The environment fills what the command line left out.
    let settings = Settings::resolve(&bare, env);
    assert_eq!(settings.prefix, "env-");
    assert_eq!(settings.connect.as_deref(), Some("tcp/10.0.0.1:1"));
    assert_eq!(settings.myrmic, PathBuf::from("/env/myrmic"));

    // The command line wins.
    let flags =
        parse("chillguard --prefix cli- --connect tcp/127.0.0.1:2 --myrmic ./m --json status")
            .unwrap()
            .global;
    let settings = Settings::resolve(&flags, env);
    assert_eq!(settings.prefix, "cli-");
    assert_eq!(settings.connect.as_deref(), Some("tcp/127.0.0.1:2"));
    assert_eq!(settings.myrmic, PathBuf::from("./m"));
    assert!(settings.json);
}

#[test]
fn playground_files_go_under_the_xdg_data_directory() {
    let bare = parse("chillguard start").unwrap().global;
    let home_only = |name: &str| (name == "HOME").then(|| "/home/ada".to_string());
    assert_eq!(
        Settings::resolve(&bare, home_only).home,
        PathBuf::from("/home/ada/.local/share/chillguard")
    );
    let both = |name: &str| match name {
        "HOME" => Some("/home/ada".to_string()),
        "XDG_DATA_HOME" => Some("/data".to_string()),
        _ => None,
    };
    assert_eq!(
        Settings::resolve(&bare, both).home,
        PathBuf::from("/data/chillguard")
    );
}

#[test]
fn cell_names_follow_the_deploy_script() {
    let global = parse("chillguard --prefix e2e-123- status").unwrap().global;
    let settings = Settings::resolve(&global, |_| None);
    assert_eq!(settings.sensor(), "e2e-123-sensor-cell");
    assert_eq!(settings.asset(), "e2e-123-fridge-asset-cell");
    assert_eq!(settings.monitor(), "e2e-123-risk-monitor-cell");

    let settings = Settings::resolve(&parse("chillguard status").unwrap().global, |_| None);
    assert_eq!(settings.asset(), "fridge-asset-cell");
}

fn current() -> RefrigeratorConfig {
    RefrigeratorConfig {
        refrigerator_id: RefrigeratorId::new("fridge-1"),
        lower_limit_c: Some(2.0),
        upper_limit_c: 8.0,
        critical_after: Duration::from_mins(30),
        stale_after: Duration::from_mins(5),
        hysteresis_c: 0.5,
    }
}

fn limits(line: &str) -> SetLimits {
    match command(line) {
        Command::SetLimits(limits) => limits,
        other => panic!("not set-limits: {other:?}"),
    }
}

#[test]
fn set_limits_changes_only_what_was_given() {
    let merged = merge_limits(&current(), &limits("chillguard set-limits --upper 10")).unwrap();
    assert_eq!(
        merged,
        RefrigeratorConfig {
            upper_limit_c: 10.0,
            ..current()
        }
    );

    let merged = merge_limits(
        &current(),
        &limits(
            "chillguard set-limits --no-lower --critical-after 10m --stale-after 2m --hysteresis 0",
        ),
    )
    .unwrap();
    assert_eq!(
        merged,
        RefrigeratorConfig {
            lower_limit_c: None,
            critical_after: Duration::from_mins(10),
            stale_after: Duration::from_mins(2),
            hysteresis_c: 0.0,
            ..current()
        }
    );

    // Asking for what is already so changes nothing.
    let merged = merge_limits(
        &current(),
        &limits("chillguard set-limits --lower 2 --upper 8"),
    )
    .unwrap();
    assert_eq!(merged, current());
}

#[test]
fn set_limits_refuses_a_result_core_would_refuse() {
    struct Case {
        line: &'static str,
        says: &'static str,
    }
    let cases = [
        Case {
            line: "chillguard set-limits --lower 9",
            says: "lower limit 9 °C is above upper limit 8 °C",
        },
        Case {
            line: "chillguard set-limits --upper 1",
            says: "lower limit 2 °C is above upper limit 1 °C",
        },
        Case {
            line: "chillguard set-limits --upper nan",
            says: "temperature limits must be finite numbers",
        },
        Case {
            line: "chillguard set-limits --hysteresis -1",
            says: "hysteresis must be a finite number that is zero or positive",
        },
        Case {
            line: "chillguard set-limits --hysteresis 4",
            says: "leaves no temperature that counts as recovered",
        },
        Case {
            line: "chillguard set-limits --stale-after 0s",
            says: "stale_after must be longer than zero",
        },
    ];
    for case in cases {
        let error = merge_limits(&current(), &limits(case.line)).expect_err(case.line);
        let message = error.to_string();
        assert!(
            message.contains(case.says),
            "`{}` gave `{message}`, expected it to mention `{}`",
            case.line,
            case.says
        );
    }
}
