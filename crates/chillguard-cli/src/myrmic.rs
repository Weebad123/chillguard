//! Running the Myrmic CLI: the one place where this client meets the swarm.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::Error;
use crate::wire::{EventParser, ReceivedEvent};

/// How much longer than the timeout a starting subscriber is waited for.
const READY_GRACE: Duration = Duration::from_secs(2);

/// How to run `myrmic`.
#[derive(Debug, Clone)]
pub struct Myrmic {
    /// The program.
    pub bin: PathBuf,
    /// The runtime endpoint for `--connect`, if any.
    pub connect: Option<String>,
    /// The `--timeout` every invocation gets.
    pub timeout: Duration,
}

/// What `myrmic deploy` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deployed {
    /// The cell was deployed.
    Now,
    /// A cell of that name was running already; nothing changed.
    Already,
}

impl Myrmic {
    fn command(&self) -> Command {
        let mut command = self.local();
        if let Some(endpoint) = &self.connect {
            command.arg("--connect").arg(endpoint);
        }
        command
    }

    /// A command about this machine's runtimes, which needs no swarm.
    fn local(&self) -> Command {
        let mut command = Command::new(&self.bin);
        command
            .arg("--timeout")
            .arg(format!("{}ms", self.timeout.as_millis().max(1)));
        command.stdin(Stdio::null());
        command
    }

    fn output(&self, mut command: Command) -> Result<Output, Error> {
        command.output().map_err(|source| self.spawn_error(source))
    }

    fn spawn_error(&self, source: std::io::Error) -> Error {
        Error::Spawn {
            bin: self.bin.display().to_string(),
            source,
        }
    }

    /// Sends a command to a cell: `myrmic send <target> <command> <payload>`.
    ///
    /// # Errors
    ///
    /// [`Error::Spawn`] when `myrmic` cannot be started, [`Error::Myrmic`]
    /// with its message when it reports a failure, for example that the
    /// target is not deployed.
    pub fn send(&self, target: &str, command: &str, payload: &[u8]) -> Result<(), Error> {
        let output = self
            .command()
            .arg("send")
            .arg(target)
            .arg(command)
            .arg(String::from_utf8_lossy(payload).as_ref())
            .output()
            .map_err(|source| self.spawn_error(source))?;
        if output.status.success() {
            return Ok(());
        }
        let detail = failure_message(&String::from_utf8_lossy(&output.stderr));
        if detail.contains("has no placement") {
            return Err(Error::NotDeployed {
                target: target.to_string(),
                detail,
            });
        }
        Err(Error::Myrmic(detail))
    }

    /// Whether a runtime called `name` is running on this machine:
    /// `myrmic runtimes <name> info`. A runtime that was never started, or
    /// was stopped, is not.
    ///
    /// # Errors
    ///
    /// [`Error::Spawn`] when `myrmic` cannot be started.
    pub fn runtime_is_running(&self, name: &str) -> Result<bool, Error> {
        let mut command = self.local();
        command.arg("runtimes").arg(name).arg("info");
        let output = self.output(command)?;
        Ok(output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line.starts_with("status") && line.contains("running")))
    }

    /// Starts a runtime in the background with an in-memory database:
    /// `myrmic runtimes start <config> --name <name> --tmp --detached`.
    ///
    /// # Errors
    ///
    /// [`Error::Spawn`] when `myrmic` cannot be started, [`Error::Myrmic`]
    /// with its message when it reports a failure.
    pub fn runtime_start(&self, name: &str, config: &Path) -> Result<(), Error> {
        let mut command = self.local();
        command.args(["runtimes", "start"]).arg(config).args([
            "--name",
            name,
            "--tmp",
            "--detached",
        ]);
        let output = self.output(command)?;
        if output.status.success() {
            return Ok(());
        }
        Err(Error::Myrmic(failure_details(&output.stderr)))
    }

    /// Stops a runtime: `myrmic runtimes delete <name>`. Returns `false`
    /// when no runtime of that name was running.
    ///
    /// # Errors
    ///
    /// [`Error::Spawn`] when `myrmic` cannot be started, [`Error::Myrmic`]
    /// with its message when it reports any other failure.
    pub fn runtime_stop(&self, name: &str) -> Result<bool, Error> {
        let mut command = self.local();
        command.args(["runtimes", "delete", name]);
        let output = self.output(command)?;
        if output.status.success() {
            return Ok(true);
        }
        let detail = failure_details(&output.stderr);
        if detail.contains("no runtime") {
            return Ok(false);
        }
        Err(Error::Myrmic(detail))
    }

    /// Whether the runtime behind `connect` answers and lists a node called
    /// `name`: `myrmic network --once`. A runtime that has just started is
    /// listed without its name for a moment, and is not ready until it has
    /// one.
    ///
    /// # Errors
    ///
    /// [`Error::Spawn`] when `myrmic` cannot be started.
    pub fn lists_node(&self, name: &str) -> Result<bool, Error> {
        let mut command = self.command();
        command.args(["network", "--once"]);
        let output = self.output(command)?;
        Ok(output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line.split_whitespace().any(|word| word == name)))
    }

    /// Deploys a built cell with its init payload:
    /// `myrmic deploy <wasm> --name <name> --init <json>`.
    ///
    /// # Errors
    ///
    /// [`Error::Spawn`] when `myrmic` cannot be started, [`Error::Myrmic`]
    /// with its message when the deployment fails, for example because the
    /// cell refused its payload.
    pub fn deploy(&self, wasm: &Path, name: &str, init: &str) -> Result<Deployed, Error> {
        let mut command = self.command();
        command
            .arg("deploy")
            .arg(wasm)
            .args(["--name", name, "--init", init]);
        let output = self.output(command)?;
        if output.status.success() {
            return Ok(Deployed::Now);
        }
        let detail = failure_details(&output.stderr);
        if detail.contains("is already deployed") {
            return Ok(Deployed::Already);
        }
        Err(Error::Myrmic(detail))
    }

    /// Starts `myrmic subscribe <events...>` and waits until it reports that
    /// the subscription is in place, or `deadline` passes.
    ///
    /// # Errors
    ///
    /// [`Error::Spawn`] when `myrmic` cannot be started, [`Error::Myrmic`]
    /// when it fails or does not become ready in time.
    pub fn subscribe(&self, events: &[&str], deadline: Instant) -> Result<Subscription, Error> {
        let mut child = self
            .command()
            .arg("subscribe")
            .args(events)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| self.spawn_error(source))?;
        let (lines, received) = mpsc::channel();
        if let Some(stdout) = child.stdout.take() {
            forward_lines(stdout, lines.clone(), Line::Out);
        }
        if let Some(stderr) = child.stderr.take() {
            forward_lines(stderr, lines, Line::Err);
        }
        let mut subscription = Subscription {
            child,
            received,
            parser: EventParser::new(),
        };
        // `myrmic` gives up by itself after the same timeout and says why
        // (no runtime found, for one); the grace lets its message arrive
        // instead of ours.
        subscription.wait_until_ready(deadline + READY_GRACE)?;
        Ok(subscription)
    }
}

/// A line of the subscriber's output and the stream it came from.
enum Line {
    Out(String),
    Err(String),
}

/// Reads `stream` line by line on its own thread. The thread ends when the
/// stream closes (the child exited) or nobody is listening any more.
fn forward_lines(
    stream: impl Read + Send + 'static,
    lines: Sender<Line>,
    wrap: fn(String) -> Line,
) {
    thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if lines.send(wrap(line)).is_err() {
                break;
            }
        }
    });
}

/// The message of a failed `myrmic` run: its `ERROR` lines, or all it wrote.
fn failure_message(stderr: &str) -> String {
    let errors: Vec<&str> = stderr
        .lines()
        .filter_map(|line| line.strip_prefix("ERROR"))
        .map(str::trim)
        .collect();
    if errors.is_empty() {
        stderr.trim().to_string()
    } else {
        errors.join("; ")
    }
}

/// Everything a failed `myrmic` run wrote apart from its progress lines.
/// Unlike [`failure_message`] this keeps the `Caused by:` lines, which is
/// where a failed deployment says why.
fn failure_details(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .map(|line| line.strip_prefix("ERROR").unwrap_or(line).trim())
        .filter(|line| !line.is_empty() && !line.starts_with("INFO"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A running `myrmic subscribe`. Dropping it stops the subscriber.
pub struct Subscription {
    child: Child,
    received: Receiver<Line>,
    parser: EventParser,
}

impl Subscription {
    /// Waits for the subscriber's "subscribed to" line.
    fn wait_until_ready(&mut self, deadline: Instant) -> Result<(), Error> {
        loop {
            match self.next_line(deadline)? {
                Some(Line::Err(line)) if line.contains("subscribed to") => return Ok(()),
                Some(_) => {}
                None => {
                    return Err(Error::Myrmic(
                        "`myrmic subscribe` did not become ready in time".to_string(),
                    ));
                }
            }
        }
    }

    /// The next event, or `None` when `deadline` passes first.
    ///
    /// # Errors
    ///
    /// [`Error::Myrmic`] when the subscriber reports an error or ends.
    pub fn next_event(&mut self, deadline: Instant) -> Result<Option<ReceivedEvent>, Error> {
        loop {
            match self.next_line(deadline)? {
                Some(Line::Out(line)) => {
                    if let Some(event) = self.parser.push_line(&line) {
                        return Ok(Some(event));
                    }
                }
                Some(Line::Err(_)) => {}
                None => return Ok(None),
            }
        }
    }

    /// The next line from either stream; `None` on the deadline. An `ERROR`
    /// line from the subscriber, or its exit, is an error.
    fn next_line(&mut self, deadline: Instant) -> Result<Option<Line>, Error> {
        let wait = deadline.saturating_duration_since(Instant::now());
        match self.received.recv_timeout(wait) {
            Ok(Line::Err(line)) if line.starts_with("ERROR") => {
                Err(Error::Myrmic(failure_message(&line)))
            }
            Ok(line) => Ok(Some(line)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(Error::Myrmic(
                "`myrmic subscribe` ended unexpectedly".to_string(),
            )),
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        // The subscriber runs until it is stopped; both calls can only fail
        // if it is already gone, which is the state wanted.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_message_keeps_the_error_lines_only() {
        let stderr =
            "INFO  trace ID = ae14fb37\nERROR command failed: cell 2d0238fd has no placement\n";
        assert_eq!(
            failure_message(stderr),
            "command failed: cell 2d0238fd has no placement"
        );
        assert_eq!(failure_message("something else\n"), "something else");
        assert_eq!(failure_message("ERROR a\nERROR b\n"), "a; b");
    }

    #[test]
    fn failure_details_keep_the_cause() {
        let stderr = b"INFO  deploying cell (srn = sensor-cell)\nERROR unable to deploy cell\n\
            Caused by: deployment failed:\n  cell 'f24a' failed on runtime ff30: Cell init error: \
            failed to deserialize json\n";
        assert_eq!(
            failure_details(stderr),
            "unable to deploy cell Caused by: deployment failed: cell 'f24a' failed on runtime \
             ff30: Cell init error: failed to deserialize json"
        );
        assert_eq!(failure_details(b""), "");
    }
}
