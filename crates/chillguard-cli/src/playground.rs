//! The playground: `start` and `stop`.
//!
//! `start` brings up everything a person needs to try ChillGuard on one
//! machine, and nothing that outlives it: a Myrmic runtime of its own with
//! an in-memory database (`--tmp`), off the network (no multicast, loopback
//! only), with the gateway that serves the web dashboard embedded; then the
//! four cells with the development payloads of `config/*.dev.json`, which
//! are compiled into this program so that the playground is exactly
//! `make deploy PROFILE=dev`.
//!
//! The runtime runs in the background until `stop`. What a later `start`
//! or `stop` needs to know about it (its two ports) is kept in a small file
//! under [`Settings::home`].

use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::cli::{DEFAULT_DASHBOARD_PORT, DEFAULT_PLAYGROUND, DEFAULT_SWARM_PORT, Settings, Start};
use crate::error::Error;
use crate::myrmic::Myrmic;

/// Where the dashboard cell mounts the page (`MOUNT` in its `handlers.rs`).
const MOUNT: &str = "/chillguard/";
/// The file the dashboard cell serves once the asset cell has answered it:
/// the sign that the whole chain works, not only the page.
const FIRST_ANSWER: &str = "/chillguard/status/0.json";

/// How long the runtime, and then the dashboard, may take to come up.
const READY_WITHIN: Duration = Duration::from_secs(30);
/// How long a stopped runtime may take to exit.
const GONE_WITHIN: Duration = Duration::from_secs(10);
/// The pause between two looks at something that is being waited for.
const POLL_EVERY: Duration = Duration::from_millis(300);
/// The timeout of one `myrmic network --once` while waiting for the runtime.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// A deployment takes about a second; the operator's `--timeout` is meant
/// for queries and may be shorter than a slow machine needs here.
const DEPLOY_TIMEOUT: Duration = Duration::from_secs(30);
/// The timeout of one request to the dashboard.
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);

/// A cell of the playground: its name, its binary, and its init payload.
struct Cell {
    name: &'static str,
    wasm: &'static str,
    init: &'static str,
}

/// The cells in the order they are deployed. The dashboard comes last: it
/// asks the others. The names carry no prefix, which is what the dashboard's
/// payload expects and what `chillguard status` looks for by default.
const CELLS: [Cell; 4] = [
    Cell {
        name: "sensor-cell",
        wasm: "sensor_cell.wasm",
        init: include_str!("../../../config/sensor-cell.dev.json"),
    },
    Cell {
        name: "fridge-asset-cell",
        wasm: "fridge_asset_cell.wasm",
        init: include_str!("../../../config/fridge-asset-cell.dev.json"),
    },
    Cell {
        name: "risk-monitor-cell",
        wasm: "risk_monitor_cell.wasm",
        init: include_str!("../../../config/risk-monitor-cell.dev.json"),
    },
    Cell {
        name: "dashboard-cell",
        wasm: "dashboard_cell.wasm",
        init: include_str!("../../../config/dashboard-cell.dev.json"),
    },
];

/// A running playground, as `start` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playground {
    /// The name of its runtime.
    pub runtime: String,
    /// The address of the dashboard.
    pub url: String,
    /// The runtime's endpoint, for `--connect` and `MYRMIC_CONNECT`.
    pub connect: String,
    /// Whether this `start` found it running instead of starting it.
    pub already_running: bool,
}

/// The two ports of a playground.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ports {
    /// The gateway's port: where the dashboard is served.
    pub dashboard: u16,
    /// The runtime's loopback port: where the Myrmic CLI reaches it.
    pub swarm: u16,
}

/// The configuration the playground's runtime is started with: off the
/// network, with the gateway embedded. The same shape as
/// `config/runtime.dashboard.yml`.
#[must_use]
pub fn runtime_config(ports: Ports) -> String {
    format!(
        "# Written by `chillguard start`, and written again by the next one.\n\
         telemetry:\n\
         \x20 logs:\n\
         \x20   format: COMPACT\n\
         \x20   env_filter: \"info\"\n\
         \n\
         zenoh:\n\
         \x20 scouting:\n\
         \x20   multicast:\n\
         \x20     enabled: false\n\
         \x20 listen:\n\
         \x20   endpoints: [\"tcp/127.0.0.1:{}\"]\n\
         \n\
         gateway:\n\
         \x20 port: {}\n",
        ports.swarm, ports.dashboard
    )
}

/// Starts the playground, or finds it running, and waits until its
/// dashboard shows the refrigerator. `progress` is told each step in words.
///
/// A `start` that fails after it started the runtime stops it again, so
/// that the next one begins from nothing.
///
/// # Errors
///
/// [`Error::Playground`] when the cells are not built, a port is taken, the
/// playground runs on other ports than were asked for, a cell cannot be
/// deployed or the dashboard does not come up; what [`Myrmic`] returns when
/// the Myrmic CLI itself fails.
pub fn start(
    args: &Start,
    settings: &Settings,
    progress: &mut dyn FnMut(&str),
) -> Result<Playground, Error> {
    let name = args.name.as_deref().unwrap_or(DEFAULT_PLAYGROUND);
    let cells = locate_cells(args.cells.as_deref())?;
    let dir = state_dir(settings, name);
    let local = myrmic(settings, None, settings.timeout);

    let already_running = local.runtime_is_running(name)?;
    let asked = Ports {
        dashboard: args.port.unwrap_or(DEFAULT_DASHBOARD_PORT),
        swarm: args.swarm_port.unwrap_or(DEFAULT_SWARM_PORT),
    };
    let ports = if already_running {
        let ports = read_ports(&dir).unwrap_or(asked);
        // Only what was asked for in so many words can be refused.
        let moved = args.port.is_some_and(|port| port != ports.dashboard)
            || args.swarm_port.is_some_and(|port| port != ports.swarm);
        if moved {
            return Err(Error::Playground(format!(
                "the playground is already running with its dashboard on port {} and its \
                 runtime on port {}; run `chillguard stop` first to move it",
                ports.dashboard, ports.swarm
            )));
        }
        progress(&format!("runtime {name} is already running"));
        ports
    } else {
        refuse_taken_ports(asked)?;
        let config = write_state(&dir, asked)?;
        progress(&format!("starting runtime {name}"));
        local.runtime_start(name, &config)?;
        asked
    };

    let playground = Playground {
        runtime: name.to_string(),
        url: format!("http://localhost:{}{MOUNT}", ports.dashboard),
        connect: format!("tcp/127.0.0.1:{}", ports.swarm),
        already_running,
    };
    let brought_up = bring_up(&playground, ports, &cells, settings, progress);
    if brought_up.is_err() && !already_running {
        // Best effort: the error that is returned is the one that matters.
        let _ = local.runtime_stop(name);
        let _ = fs::remove_dir_all(&dir);
    }
    brought_up.map(|()| playground)
}

/// Deploys the cells to a runtime that has been started, and waits for the
/// dashboard.
fn bring_up(
    playground: &Playground,
    ports: Ports,
    cells: &Path,
    settings: &Settings,
    progress: &mut dyn FnMut(&str),
) -> Result<(), Error> {
    let name = &playground.runtime;
    let connect = Some(playground.connect.clone());
    let probe = myrmic(settings, connect.clone(), PROBE_TIMEOUT);
    if !wait_for(READY_WITHIN, || probe.lists_node(name))? {
        return Err(Error::Playground(format!(
            "runtime {name} did not answer on {} within {}; `myrmic runtimes {name} logs` \
             says why",
            playground.connect,
            humantime::format_duration(READY_WITHIN)
        )));
    }

    let swarm = myrmic(settings, connect, settings.timeout.max(DEPLOY_TIMEOUT));
    for cell in &CELLS {
        // A cell that is already there is left as it is; one that went
        // missing (the computer slept and the runtime gave it up) is put back.
        let step = if playground.already_running {
            "making sure of"
        } else {
            "deploying"
        };
        progress(&format!("{step} {}", cell.name));
        swarm
            .deploy(&cells.join(cell.wasm), cell.name, &one_line(cell.init)?)
            .map_err(|error| {
                Error::Playground(format!(
                    "could not deploy {}: {error}. If the sources changed since the cells were \
                     built, run `make build-cells` and start again",
                    cell.name
                ))
            })?;
    }

    progress("waiting for the dashboard");
    for (path, what) in [
        (MOUNT, "is not served"),
        (FIRST_ANSWER, "shows no refrigerator"),
    ] {
        if !wait_for(READY_WITHIN, || {
            Ok(http_status(ports.dashboard, path) == Some(200))
        })? {
            return Err(Error::Playground(format!(
                "the dashboard {what} at {} after {}; `myrmic runtimes {name} logs` says why",
                playground.url,
                humantime::format_duration(READY_WITHIN)
            )));
        }
    }
    Ok(())
}

/// Stops the playground and forgets its files. Returns `false` when it was
/// not running.
///
/// # Errors
///
/// What [`Myrmic::runtime_stop`] returns, and [`Error::Playground`] when
/// the runtime was told to stop and is still there.
pub fn stop(name: &str, settings: &Settings) -> Result<bool, Error> {
    let local = myrmic(settings, None, settings.timeout);
    let was_running = local.runtime_stop(name)?;
    // The runtime takes a moment to exit; its ports are taken until then,
    // and a `start` straight after would be refused.
    if was_running
        && !wait_for(GONE_WITHIN, || {
            local.runtime_is_running(name).map(|running| !running)
        })?
    {
        return Err(Error::Playground(format!(
            "runtime {name} was told to stop and is still running after {}; \
             `myrmic runtimes {name} info` shows its process",
            humantime::format_duration(GONE_WITHIN)
        )));
    }
    // Nothing to remove is the state wanted.
    let _ = fs::remove_dir_all(state_dir(settings, name));
    Ok(was_running)
}

/// Opens `url` in the desktop's browser. Returns `false` when no opener
/// could be started; whether a browser then appeared is not known.
#[must_use]
pub fn open_browser(url: &str) -> bool {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let child = Command::new(opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            // Some openers stay until the browser closes: wait for it off
            // to the side, where it cannot hold this command up.
            thread::spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(_) => false,
    }
}

/// A payload file's JSON on one line, as a command-line argument wants it.
fn one_line(json: &str) -> Result<String, Error> {
    serde_json::from_str::<serde_json::Value>(json)
        .map(|value| value.to_string())
        .map_err(|error| Error::Playground(format!("internal: init payload: {error}")))
}

fn myrmic(settings: &Settings, connect: Option<String>, timeout: Duration) -> Myrmic {
    Myrmic {
        bin: settings.myrmic.clone(),
        connect,
        timeout,
    }
}

/// Where a playground's runtime configuration and ports are kept.
fn state_dir(settings: &Settings, name: &str) -> PathBuf {
    settings.home.join("playground").join(name)
}

/// Writes the runtime configuration and the ports; returns the path of the
/// configuration.
fn write_state(dir: &Path, ports: Ports) -> Result<PathBuf, Error> {
    let cannot_write = |path: &Path, error: std::io::Error| {
        Error::Playground(format!("cannot write {}: {error}", path.display()))
    };
    fs::create_dir_all(dir).map_err(|error| cannot_write(dir, error))?;
    let config = dir.join("runtime.yml");
    fs::write(&config, runtime_config(ports)).map_err(|error| cannot_write(&config, error))?;
    let state = dir.join("ports.json");
    let json = serde_json::to_string(&ports)
        .map_err(|error| Error::Playground(format!("internal: ports: {error}")))?;
    fs::write(&state, json).map_err(|error| cannot_write(&state, error))?;
    Ok(config)
}

fn read_ports(dir: &Path) -> Option<Ports> {
    serde_json::from_str(&fs::read_to_string(dir.join("ports.json")).ok()?).ok()
}

/// The directory that holds every cell's binary: the one given, or the
/// first of the usual places that has them all.
fn locate_cells(given: Option<&Path>) -> Result<PathBuf, Error> {
    const BUILT: &str = "wasm32-unknown-unknown/release";
    let candidates: Vec<PathBuf> = if let Some(dir) = given {
        vec![dir.to_path_buf()]
    } else {
        // Beside this program when it runs from the workspace's target
        // directory (target/debug/chillguard), else under the current
        // directory.
        let beside = std::env::current_exe().ok().and_then(|exe| {
            exe.parent()
                .and_then(Path::parent)
                .map(|target| target.join(BUILT))
        });
        beside
            .into_iter()
            .chain([Path::new("target").join(BUILT)])
            .collect()
    };
    let missing = |dir: &Path| -> Vec<&'static str> {
        CELLS
            .iter()
            .map(|cell| cell.wasm)
            .filter(|wasm| !dir.join(wasm).is_file())
            .collect()
    };
    if let Some(found) = candidates.iter().find(|dir| missing(dir).is_empty()) {
        return Ok(found.clone());
    }
    let looked: Vec<String> = candidates
        .iter()
        .map(|dir| format!("{} (missing {})", dir.display(), missing(dir).join(", ")))
        .collect();
    Err(Error::Playground(format!(
        "the built cells were not found in {}; run `make build-cells` in the ChillGuard \
         repository, or name their directory with --cells",
        looked.join(" or ")
    )))
}

/// Refuses ports something else is listening on. The runtime would start
/// regardless and the dashboard would silently be somebody else's page.
fn refuse_taken_ports(ports: Ports) -> Result<(), Error> {
    // The gateway listens on every interface, IPv4 and IPv6; the runtime on
    // IPv4 loopback.
    let dashboard_taken = in_use(SocketAddr::from((Ipv4Addr::UNSPECIFIED, ports.dashboard)))
        || in_use(SocketAddr::from((Ipv6Addr::UNSPECIFIED, ports.dashboard)));
    if dashboard_taken {
        return Err(Error::Playground(format!(
            "port {} is in use on this machine; choose another for the dashboard with --port",
            ports.dashboard
        )));
    }
    if ports.swarm == ports.dashboard
        || in_use(SocketAddr::from((Ipv4Addr::LOCALHOST, ports.swarm)))
    {
        return Err(Error::Playground(format!(
            "port {} is in use on this machine; choose another for the runtime with --swarm-port",
            ports.swarm
        )));
    }
    Ok(())
}

/// Whether something listens on `address`. A bind that fails for another
/// reason does not say so, and is left for the runtime to report.
fn in_use(address: SocketAddr) -> bool {
    matches!(TcpListener::bind(address), Err(error) if error.kind() == ErrorKind::AddrInUse)
}

/// The status of `GET <path>` on this machine's `port`, or `None` when
/// nothing answered in time.
fn http_status(port: u16, path: &str) -> Option<u16> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, HTTP_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(HTTP_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(HTTP_TIMEOUT)).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    status_of(&line)
}

/// The status code of an HTTP status line: `HTTP/1.1 200 OK` gives 200.
fn status_of(line: &str) -> Option<u16> {
    let mut words = line.split_whitespace();
    words
        .next()
        .filter(|version| version.starts_with("HTTP/"))?;
    words.next()?.parse().ok()
}

/// Looks every [`POLL_EVERY`] until `ready` says yes or `within` has passed.
/// Returns whether it said yes.
fn wait_for(
    within: Duration,
    mut ready: impl FnMut() -> Result<bool, Error>,
) -> Result<bool, Error> {
    let deadline = Instant::now() + within;
    loop {
        if ready()? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(POLL_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    const PORTS: Ports = Ports {
        dashboard: 8642,
        swarm: 20124,
    };

    #[test]
    fn runtime_configuration_is_off_the_network_with_a_gateway() {
        let config = runtime_config(PORTS);
        assert_eq!(
            config,
            "# Written by `chillguard start`, and written again by the next one.\n\
             telemetry:\n  logs:\n    format: COMPACT\n    env_filter: \"info\"\n\n\
             zenoh:\n  scouting:\n    multicast:\n      enabled: false\n  listen:\n    \
             endpoints: [\"tcp/127.0.0.1:20124\"]\n\n\
             gateway:\n  port: 8642\n"
        );
    }

    /// The payloads are the dev profile's files. This pins what `start`
    /// relies on in them: one refrigerator throughout, a sensor that takes
    /// scenarios, and a dashboard that names the cells as they are deployed
    /// here and offers its buttons.
    #[test]
    fn payloads_describe_one_refrigerator_with_the_controls_on() {
        let payload = |name: &str| -> Value {
            let cell = CELLS.iter().find(|cell| cell.name == name).unwrap();
            serde_json::from_str(cell.init).unwrap()
        };
        let sensor = payload("sensor-cell");
        let fridge = &sensor["refrigerator_id"];
        assert!(fridge.is_string());
        assert_eq!(sensor["dev_mode"], true);
        assert_eq!(
            &payload("fridge-asset-cell")["config"]["refrigerator_id"],
            fridge
        );
        assert_eq!(&payload("risk-monitor-cell")["refrigerator_id"], fridge);

        let dashboard = payload("dashboard-cell");
        assert_eq!(dashboard["dev_controls"], true);
        let shown = &dashboard["refrigerators"];
        assert_eq!(shown.as_array().map(Vec::len), Some(1));
        assert_eq!(shown[0]["sensor"], "sensor-cell");
        assert_eq!(shown[0]["asset"], "fridge-asset-cell");
        assert_eq!(shown[0]["monitor"], "risk-monitor-cell");
    }

    #[test]
    fn payloads_go_on_the_command_line_as_one_line_in_the_files_order() {
        assert_eq!(
            one_line("{\n  \"refrigerator_id\": \"fridge-1\",\n  \"interval_ms\": 1000\n}\n")
                .unwrap(),
            r#"{"refrigerator_id":"fridge-1","interval_ms":1000}"#
        );
        for cell in &CELLS {
            assert!(
                !one_line(cell.init).unwrap().contains('\n'),
                "{}",
                cell.name
            );
        }
        assert!(one_line("{").is_err());
    }

    #[test]
    fn the_dashboard_is_deployed_last() {
        assert_eq!(CELLS.last().map(|cell| cell.name), Some("dashboard-cell"));
        for cell in &CELLS {
            assert_eq!(cell.wasm, format!("{}.wasm", cell.name.replace('-', "_")));
        }
    }

    #[test]
    fn status_lines() {
        assert_eq!(status_of("HTTP/1.1 200 OK\r\n"), Some(200));
        assert_eq!(status_of("HTTP/1.1 404 Not Found\r\n"), Some(404));
        assert_eq!(status_of("HTTP/1.0 307 Temporary Redirect"), Some(307));
        assert_eq!(status_of(""), None);
        assert_eq!(status_of("SSH-2.0-OpenSSH_9.6"), None);
        assert_eq!(status_of("HTTP/1.1 OK"), None);
    }

    #[test]
    fn a_port_somebody_listens_on_is_refused_and_nothing_answers_on_a_free_one() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let taken = listener.local_addr().unwrap().port();
        assert!(in_use(SocketAddr::from((Ipv4Addr::LOCALHOST, taken))));
        let error = refuse_taken_ports(Ports {
            dashboard: taken,
            swarm: 20124,
        })
        .unwrap_err();
        assert!(error.to_string().contains("--port"), "{error}");
        drop(listener);
        assert!(!in_use(SocketAddr::from((Ipv4Addr::LOCALHOST, taken))));
        assert_eq!(http_status(taken, MOUNT), None);
    }

    #[test]
    fn the_two_ports_must_differ() {
        let free = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .and_then(|listener| listener.local_addr())
            .unwrap()
            .port();
        let error = refuse_taken_ports(Ports {
            dashboard: free,
            swarm: free,
        })
        .unwrap_err();
        assert!(error.to_string().contains("--swarm-port"), "{error}");
    }

    #[test]
    fn missing_cells_are_named_with_what_to_do() {
        let dir = std::env::temp_dir().join(format!("chillguard-cells-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("sensor_cell.wasm"), b"").unwrap();
        let error = locate_cells(Some(&dir)).unwrap_err().to_string();
        assert!(error.contains("missing fridge_asset_cell.wasm"), "{error}");
        assert!(!error.contains("missing sensor_cell.wasm"), "{error}");
        assert!(error.contains("make build-cells"), "{error}");

        for cell in &CELLS {
            fs::write(dir.join(cell.wasm), b"").unwrap();
        }
        assert_eq!(locate_cells(Some(&dir)).unwrap(), dir);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ports_are_kept_between_start_and_stop() {
        let dir = std::env::temp_dir().join(format!("chillguard-state-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(read_ports(&dir), None);
        let config = write_state(&dir, PORTS).unwrap();
        assert_eq!(fs::read_to_string(config).unwrap(), runtime_config(PORTS));
        assert_eq!(read_ports(&dir), Some(PORTS));
        fs::remove_dir_all(&dir).unwrap();
    }
}
