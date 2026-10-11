//! The client against a stand-in for the Myrmic CLI: a shell script that
//! speaks `myrmic send` and `myrmic subscribe` the way the real one does
//! (formats captured from myrmic 0.7.0), so that starting the subscriber,
//! waiting for it, reading its events and stopping it are tested without a
//! runtime.

#![cfg(unix)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chillguard_cli::Error;
use chillguard_cli::cli::Settings;
use chillguard_cli::client::Client;
use chillguard_core::risk::{IncidentId, Status};
use chillguard_protocol::commands::Acknowledge;
use chillguard_protocol::{Envelope, Payload};
use common::{config, incident_list, snapshot};

/// `send` appends the canned answer for the command, as an event named by
/// the payload's `reply_to`, to a file; `subscribe` announces itself and
/// follows that file. A target called `missing-...` is not deployed.
const SCRIPT: &str = r#"#!/usr/bin/env bash
dir="$(dirname "$0")"
echo "$*" >> "$dir/calls"
while [[ "${1:-}" == --* ]]; do shift 2; done
case "${1:-}" in
subscribe)
    echo "INFO  subscribed to: ${2:-} (ctrl-c to stop)" >&2
    touch "$dir/events"
    exec tail -n +1 -f "$dir/events"
    ;;
send)
    target="$2" command="$3" payload="${4:-}"
    echo "INFO  trace ID = 2cbcfce77134afe3860840909e206c94" >&2
    if [[ "$target" == missing-* ]]; then
        echo "ERROR command failed: cell 2d0238fd-bb8a-56bd-a20f-05404767fad4 has no placement" >&2
        exit 1
    fi
    reply="$(printf '%s' "$payload" | sed -n 's/.*"reply_to":"\([^"]*\)".*/\1/p')"
    if [[ -n "$reply" && -f "$dir/$command.json" ]]; then
        {
            echo
            echo "[2026-10-10T09:19:51.954Z] event=$reply sender=6033b371-07fe-5fc5-9f03-9a18a386682b payload=330 bytes"
            cat "$dir/$command.json"
        } >> "$dir/events"
    fi
    echo "INFO  successfully sent command" >&2
    ;;
*)
    echo "ERROR unknown command" >&2
    exit 2
    ;;
esac
"#;

/// A directory holding the script and its files; removed when dropped.
struct Fake {
    dir: PathBuf,
}

impl Fake {
    fn new(test: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("chillguard-cli-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("myrmic");
        fs::write(&script, SCRIPT).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }

    /// The answer `send <command>` will produce, pretty-printed over several
    /// lines as the real subscriber prints it.
    fn answer<T: Payload>(&self, command: &str, body: T) {
        let bytes = Envelope::new(body).encode().unwrap();
        self.answer_raw(command, &serde_json::from_slice(&bytes).unwrap());
    }

    fn answer_raw(&self, command: &str, value: &serde_json::Value) {
        let text = serde_json::to_string_pretty(value).unwrap();
        fs::write(self.dir.join(format!("{command}.json")), text + "\n").unwrap();
    }

    fn client(&self, timeout: Duration) -> Client {
        Client::new(&Settings {
            prefix: String::new(),
            connect: Some("tcp/127.0.0.1:20123".to_string()),
            timeout,
            json: false,
            myrmic: self.dir.join("myrmic"),
        })
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.dir.join("calls")).unwrap_or_default()
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_query_gets_its_answer_from_the_event() {
    let fake = Fake::new("query");
    let expected = snapshot(Status::Warning, Some((44, 9.5)), config());
    fake.answer("get_status", expected.clone());
    fake.answer("get_incidents", incident_list());
    let client = fake.client(Duration::from_secs(5));

    assert_eq!(client.status("fridge-asset-cell").unwrap(), expected);
    // A second request starts a second subscriber, which also sees the
    // first request's event in the stream and must skip it.
    assert_eq!(
        client.incidents("risk-monitor-cell", 20).unwrap(),
        incident_list()
    );

    let calls = fake.calls();
    assert!(
        calls.contains("--timeout 5000ms --connect tcp/127.0.0.1:20123 subscribe cg_reply_get_status_fridge_asset_cell"),
        "{calls}"
    );
    assert!(
        calls.contains("--connect tcp/127.0.0.1:20123 send fridge-asset-cell get_status {\"v\":1,\"body\":{\"reply_to\":\"cg_reply_get_status_fridge_asset_cell\"}}"),
        "{calls}"
    );
    assert!(
        calls.contains("send risk-monitor-cell get_incidents {\"v\":1,\"body\":{\"limit\":20,\"reply_to\":\"cg_reply_get_incidents_risk_monitor_cell\"}}"),
        "{calls}"
    );
}

#[test]
fn a_command_is_one_send_with_the_envelope() {
    let fake = Fake::new("command");
    let client = fake.client(Duration::from_secs(5));
    client
        .command(
            "risk-monitor-cell",
            Acknowledge {
                incident_id: IncidentId::new("fridge-1-1791598200000"),
            },
        )
        .unwrap();
    assert_eq!(
        fake.calls().trim(),
        "--timeout 5000ms --connect tcp/127.0.0.1:20123 send risk-monitor-cell acknowledge \
         {\"v\":1,\"body\":{\"incident_id\":\"fridge-1-1791598200000\"}}"
    );
}

#[test]
fn a_cell_that_is_not_deployed_is_named() {
    let fake = Fake::new("missing");
    let client = fake.client(Duration::from_secs(5));
    let error = client.status("missing-fridge-asset-cell").unwrap_err();
    assert!(
        matches!(&error, Error::NotDeployed { target, .. } if target == "missing-fridge-asset-cell"),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("`missing-fridge-asset-cell` is not deployed"),
        "{message}"
    );
    assert!(message.contains("has no placement"), "{message}");
}

#[test]
fn silence_is_reported_after_the_timeout() {
    let fake = Fake::new("silence");
    // No canned answer: the cell accepts the command and never replies.
    let client = fake.client(Duration::from_millis(1200));
    let started = Instant::now();
    let error = client.status("fridge-asset-cell").unwrap_err();
    let waited = started.elapsed();
    assert!(
        matches!(&error, Error::NoAnswer { target, command, .. }
            if target == "fridge-asset-cell" && *command == "get_status"),
        "{error:?}"
    );
    assert!(waited >= Duration::from_millis(1200), "{waited:?}");
    assert!(waited < Duration::from_secs(4), "{waited:?}");
    assert!(
        error
            .to_string()
            .contains("no answer from `fridge-asset-cell` to `get_status` within 1s 200ms")
    );
}

#[test]
fn an_answer_from_another_version_is_refused() {
    let fake = Fake::new("version");
    let bytes = Envelope::new(snapshot(Status::Normal, None, config()))
        .encode()
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["v"] = serde_json::json!(2);
    fake.answer_raw("get_status", &value);
    let client = fake.client(Duration::from_secs(5));
    let error = client.status("fridge-asset-cell").unwrap_err();
    assert!(matches!(error, Error::BadAnswer { .. }), "{error:?}");
    assert!(
        error.to_string().contains("unsupported schema version 2"),
        "{error}"
    );
}

#[test]
fn a_missing_myrmic_is_explained() {
    let fake = Fake::new("nobinary");
    fs::remove_file(fake.path().join("myrmic")).unwrap();
    let client = fake.client(Duration::from_secs(1));
    let error = client.status("fridge-asset-cell").unwrap_err();
    assert!(matches!(error, Error::Spawn { .. }), "{error:?}");
    assert!(
        error.to_string().contains("is the Myrmic CLI installed?"),
        "{error}"
    );
}

#[test]
fn stopped_subscribers_do_not_linger() {
    let fake = Fake::new("linger");
    fake.answer("get_status", snapshot(Status::Normal, None, config()));
    let client = fake.client(Duration::from_secs(5));
    for _ in 0..3 {
        client.status("fridge-asset-cell").unwrap();
    }
    // Every subscriber `exec`ed `tail -f` on this test's own events file.
    let needle = fake.path().join("events").display().to_string();
    let lingering = fs::read_dir("/proc")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read(entry.path().join("cmdline")).ok())
        .filter(|cmdline| String::from_utf8_lossy(cmdline).contains(&needle))
        .count();
    assert_eq!(lingering, 0, "subscriber processes were left running");
}
