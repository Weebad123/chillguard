//! `chillguard`: the ChillGuard operator interface. See the library docs.

use std::io::{ErrorKind, Write};
use std::process::ExitCode;

use chillguard_cli::Error;
use chillguard_cli::cli::{Cli, Settings};
use chillguard_cli::run::run;
use clap::Parser;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let settings = Settings::resolve(&cli.global, |name| std::env::var(name).ok());
    let mut out = std::io::stdout().lock();
    match run(&cli.command, &settings, &mut out) {
        Ok(()) => ExitCode::SUCCESS,
        // The reader went away (`chillguard watch | head`): not a failure.
        Err(Error::Output(error)) if error.kind() == ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            // Unlike `eprintln!`, a closed stderr is not a reason to panic.
            let _ = writeln!(std::io::stderr(), "error: {error}");
            ExitCode::FAILURE
        }
    }
}
