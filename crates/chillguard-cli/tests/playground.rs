//! `start` and `stop` against a stand-in for the Myrmic CLI and a stand-in
//! for the gateway: a shell script that speaks `runtimes`, `network` and
//! `deploy` the way myrmic 0.7.0 does (messages captured from it), and a
//! listener in this process that answers every request with 200 once the
//! script has "started the runtime". What is tested is the order of the
//! steps and what each one is given, without a runtime. The real thing is
//! the `playground` scenario of `scripts/e2e.sh`.

#![cfg(unix)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chillguard_cli::Error;
use chillguard_cli::cli::{Settings, Start};
use chillguard_cli::playground::{self, Playground};

/// `runtimes start` leaves a file `running` behind and `runtimes delete`
/// removes it; `info` and `network` answer by it. `deploy` remembers the
/// names it was given and refuses a second one as the runtime does. A file
/// `refuse-<name>` makes that cell's deployment fail in init.
const SCRIPT: &str = r#"#!/usr/bin/env bash
dir="$(dirname "$0")"
echo "$*" >> "$dir/calls"
while [[ "${1:-}" == --timeout || "${1:-}" == --connect ]]; do shift 2; done
case "${1:-}" in
runtimes)
    case "${2:-}" in
    start)
        touch "$dir/running"
        echo "INFO  Starting runtime \"$5\"(ff30cc5e8f3644f9a91803a957ddcb4c)" >&2
        ;;
    delete)
        if [[ ! -f "$dir/running" ]]; then
            echo "ERROR no runtime \"$3\" found in /run/user/1000/myrmic" >&2
            exit 1
        fi
        rm "$dir/running"
        echo "INFO  sent SIGTERM to runtime \"$3\" (pid 4242)" >&2
        ;;
    *)
        if [[ ! -f "$dir/running" ]]; then
            echo "ERROR no runtime \"$2\" recorded (never started on this machine?)" >&2
            exit 1
        fi
        echo "runtime   $2 (ff30cc5e8f3644f9a91803a957ddcb4c)"
        echo "status    running (pid 4242)"
        ;;
    esac
    ;;
network)
    if [[ ! -f "$dir/running" ]]; then
        echo "ERROR no myrmic nodes discovered within 2s (extend with --timeout)" >&2
        exit 1
    fi
    echo "Discovered 1 node(s)"
    echo
    echo "  #  name             kind   id          tags"
    echo "  0  $(cat "$dir/name")  linux  [f]f30cc5e  linux"
    ;;
deploy)
    name="$4"
    echo "INFO  deploying cell (srn = $name, sri = f24a3588-be24-5377-ad08-2ce5fd1dc5c9)" >&2
    if [[ -f "$dir/refuse-$name" ]]; then
        echo "ERROR unable to deploy cell" >&2
        echo "Caused by: deployment failed:" >&2
        echo "  cell 'f24a3588' failed on runtime ff30: Cell init error: failed to deserialize json" >&2
        exit 1
    fi
    if grep -qx "$name" "$dir/deployed" 2>/dev/null; then
        echo "ERROR unable to deploy cell" >&2
        echo "Caused by: application '$name' is already deployed" >&2
        exit 1
    fi
    echo "$name" >> "$dir/deployed"
    echo "INFO  deployed cell (srn = $name)" >&2
    ;;
*)
    echo "ERROR unknown command" >&2
    exit 2
    ;;
esac
"#;

const WASMS: [&str; 4] = [
    "sensor_cell.wasm",
    "fridge_asset_cell.wasm",
    "risk_monitor_cell.wasm",
    "dashboard_cell.wasm",
];

/// A directory holding the script, its files, the "built" cells and the
/// playground's home; removed when dropped. Dropping also stops the gateway.
struct Fake {
    dir: PathBuf,
    name: String,
    dashboard_port: u16,
    swarm_port: u16,
    stop_gateway: Arc<AtomicBool>,
    gateway: Option<JoinHandle<()>>,
}

fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .unwrap()
        .port()
}

impl Fake {
    fn new(test: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("chillguard-play-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("cells")).unwrap();
        let script = dir.join("myrmic");
        fs::write(&script, SCRIPT).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        for wasm in WASMS {
            fs::write(dir.join("cells").join(wasm), b"\0asm").unwrap();
        }
        let name = format!("play-{test}");
        fs::write(dir.join("name"), &name).unwrap();
        let mut fake = Self {
            dir,
            name,
            dashboard_port: free_port(),
            swarm_port: free_port(),
            stop_gateway: Arc::new(AtomicBool::new(false)),
            gateway: None,
        };
        fake.gateway = Some(fake.spawn_gateway());
        fake
    }

    /// Answers 200 to everything on the dashboard's port, from the moment
    /// the script's runtime "runs": the port must be free until then, as
    /// `start` checks.
    fn spawn_gateway(&self) -> JoinHandle<()> {
        let running = self.dir.join("running");
        let port = self.dashboard_port;
        let stop = Arc::clone(&self.stop_gateway);
        thread::spawn(move || {
            while !running.exists() {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
            listener.set_nonblocking(true).unwrap();
            while !stop.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}",
                );
            }
        })
    }

    fn settings(&self) -> Settings {
        Settings {
            prefix: String::new(),
            connect: None,
            timeout: Duration::from_secs(5),
            json: false,
            myrmic: self.dir.join("myrmic"),
            home: self.dir.join("home"),
        }
    }

    fn args(&self) -> Start {
        Start {
            port: Some(self.dashboard_port),
            no_open: true,
            cells: Some(self.dir.join("cells")),
            swarm_port: Some(self.swarm_port),
            name: Some(self.name.clone()),
        }
    }

    fn start(&self, args: &Start) -> Result<(Playground, Vec<String>), Error> {
        let mut steps = Vec::new();
        let playground = playground::start(args, &self.settings(), &mut |step| {
            steps.push(step.to_string());
        })?;
        Ok((playground, steps))
    }

    /// What the script was called with, one call per line, the timeout
    /// left out.
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.dir.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let mut words = line.split(' ');
                assert_eq!(words.next(), Some("--timeout"), "{line}");
                words.next();
                words.collect::<Vec<_>>().join(" ")
            })
            .collect()
    }

    fn state_dir(&self) -> PathBuf {
        self.dir.join("home/playground").join(&self.name)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop_gateway.store(true, Ordering::Relaxed);
        if let Some(gateway) = self.gateway.take() {
            let _ = gateway.join();
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn start_brings_up_a_runtime_the_cells_and_the_dashboard_in_that_order() {
    let fake = Fake::new("fresh");
    let (playground, steps) = fake.start(&fake.args()).unwrap();

    let connect = format!("tcp/127.0.0.1:{}", fake.swarm_port);
    assert_eq!(
        playground,
        Playground {
            runtime: fake.name.clone(),
            url: format!("http://localhost:{}/chillguard/", fake.dashboard_port),
            connect: connect.clone(),
            already_running: false,
        }
    );
    assert_eq!(
        steps,
        [
            format!("starting runtime {}", fake.name),
            "deploying sensor-cell".to_string(),
            "deploying fridge-asset-cell".to_string(),
            "deploying risk-monitor-cell".to_string(),
            "deploying dashboard-cell".to_string(),
            "waiting for the dashboard".to_string(),
        ]
    );

    let calls = fake.calls();
    let config = fake.state_dir().join("runtime.yml");
    assert_eq!(calls[0], format!("runtimes {} info", fake.name));
    // The runtime is local: in memory, in the background, and started
    // without `--connect`, which would point it at another runtime.
    assert_eq!(
        calls[1],
        format!(
            "runtimes start {} --name {} --tmp --detached",
            config.display(),
            fake.name
        )
    );
    assert_eq!(calls[2], format!("--connect {connect} network --once"));
    let built = fake.dir.join("cells");
    for (call, (wasm, name)) in calls[3..].iter().zip([
        ("sensor_cell.wasm", "sensor-cell"),
        ("fridge_asset_cell.wasm", "fridge-asset-cell"),
        ("risk_monitor_cell.wasm", "risk-monitor-cell"),
        ("dashboard_cell.wasm", "dashboard-cell"),
    ]) {
        let expected = format!(
            "--connect {connect} deploy {} --name {name} --init {{",
            built.join(wasm).display()
        );
        assert!(call.starts_with(&expected), "{call}\nexpected: {expected}");
    }
    assert_eq!(calls.len(), 7, "{calls:#?}");
    // The sensor is given the development payload: scenarios allowed.
    assert!(calls[3].contains(r#""dev_mode":true"#), "{}", calls[3]);
    assert!(calls[6].contains(r#""dev_controls":true"#), "{}", calls[6]);

    // The runtime was given a configuration with both ports.
    let written = fs::read_to_string(config).unwrap();
    assert!(written.contains(&format!("tcp/127.0.0.1:{}", fake.swarm_port)));
    assert!(written.contains(&format!("port: {}", fake.dashboard_port)));
}

#[test]
fn a_second_start_finds_the_playground_and_changes_nothing() {
    let fake = Fake::new("again");
    let (first, _) = fake.start(&fake.args()).unwrap();
    let before = fake.calls().len();

    // No ports given: they are read back from the first start.
    let bare = Start {
        port: None,
        swarm_port: None,
        ..fake.args()
    };
    let (second, steps) = fake.start(&bare).unwrap();
    assert_eq!(
        second,
        Playground {
            already_running: true,
            ..first
        }
    );
    assert_eq!(
        steps[0],
        format!("runtime {} is already running", fake.name)
    );
    assert_eq!(steps[1], "making sure of sensor-cell");
    let calls = fake.calls();
    assert!(
        !calls[before..]
            .iter()
            .any(|call| call.contains("runtimes start")),
        "{calls:#?}"
    );
    // Every cell was offered again and found there.
    assert_eq!(
        fs::read_to_string(fake.dir.join("deployed")).unwrap(),
        "sensor-cell\nfridge-asset-cell\nrisk-monitor-cell\ndashboard-cell\n"
    );

    // Naming one port it already has is no objection to the other.
    let same = Start {
        port: None,
        ..fake.args()
    };
    assert!(fake.start(&same).unwrap().0.already_running);

    // Asking for other ports while it runs is refused, not ignored.
    let moved = Start {
        port: Some(free_port()),
        ..fake.args()
    };
    let error = fake.start(&moved).unwrap_err().to_string();
    assert!(error.contains("already running"), "{error}");
    assert!(error.contains(&fake.dashboard_port.to_string()), "{error}");
    assert!(error.contains("chillguard stop"), "{error}");
}

#[test]
fn a_start_that_fails_stops_the_runtime_it_started() {
    let fake = Fake::new("refused");
    fs::write(fake.dir.join("refuse-risk-monitor-cell"), "").unwrap();
    let error = fake.start(&fake.args()).unwrap_err().to_string();
    assert!(
        error.contains("could not deploy risk-monitor-cell"),
        "{error}"
    );
    assert!(error.contains("failed to deserialize json"), "{error}");
    assert!(error.contains("make build-cells"), "{error}");

    assert_eq!(
        fake.calls().last().map(String::as_str),
        Some(format!("runtimes delete {}", fake.name).as_str())
    );
    assert!(!fake.dir.join("running").exists());
    assert!(!fake.state_dir().exists());
}

#[test]
fn start_without_the_cells_starts_nothing() {
    let fake = Fake::new("unbuilt");
    fs::remove_file(fake.dir.join("cells/dashboard_cell.wasm")).unwrap();
    let error = fake.start(&fake.args()).unwrap_err().to_string();
    assert!(error.contains("missing dashboard_cell.wasm"), "{error}");
    assert!(fake.calls().is_empty(), "{:#?}", fake.calls());
}

#[test]
fn start_refuses_a_port_that_is_taken() {
    let fake = Fake::new("taken");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let taken = listener.local_addr().unwrap().port();
    let args = Start {
        port: Some(taken),
        ..fake.args()
    };
    let error = fake.start(&args).unwrap_err().to_string();
    assert!(
        error.contains(&format!("port {taken} is in use")),
        "{error}"
    );
    assert!(error.contains("--port"), "{error}");
    assert_eq!(fake.calls(), [format!("runtimes {} info", fake.name)]);
}

#[test]
fn stop_stops_the_runtime_and_says_whether_there_was_one() {
    let fake = Fake::new("stop");
    fake.start(&fake.args()).unwrap();
    assert!(fake.state_dir().exists());

    assert!(playground::stop(&fake.name, &fake.settings()).unwrap());
    assert!(!fake.dir.join("running").exists());
    assert!(!fake.state_dir().exists());
    let calls = fake.calls();
    assert!(
        calls.contains(&format!("runtimes delete {}", fake.name)),
        "{calls:#?}"
    );

    // Stopping what is not running is not an error.
    assert!(!playground::stop(&fake.name, &fake.settings()).unwrap());
}

#[test]
fn a_missing_myrmic_is_reported_as_such() {
    let fake = Fake::new("no-myrmic");
    fs::remove_file(fake.dir.join("myrmic")).unwrap();
    let error = fake.start(&fake.args()).unwrap_err();
    assert!(matches!(error, Error::Spawn { .. }), "{error}");
    let error = playground::stop(&fake.name, &fake.settings()).unwrap_err();
    assert!(matches!(error, Error::Spawn { .. }), "{error}");
}
