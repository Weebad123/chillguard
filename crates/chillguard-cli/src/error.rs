//! What can go wrong between the operator and the cells.

use chillguard_protocol::ProtocolError;
use thiserror::Error;

/// A failed command. The message is what the operator sees.
#[derive(Debug, Error)]
pub enum Error {
    /// The `myrmic` binary could not be started.
    #[error(
        "cannot run `{bin}`: {source} (is the Myrmic CLI installed? set MYRMIC_BIN or --myrmic)"
    )]
    Spawn {
        /// The program that was tried.
        bin: String,
        /// Why it could not be started.
        source: std::io::Error,
    },
    /// The cell a message was meant for is not deployed.
    #[error(
        "`{target}` is not deployed on a runtime this client can reach; \
         check --prefix and `myrmic cells --once` (myrmic: {detail})"
    )]
    NotDeployed {
        /// The cell that was addressed.
        target: String,
        /// What `myrmic` said.
        detail: String,
    },
    /// `myrmic` ran and reported a failure.
    #[error("myrmic: {0}")]
    Myrmic(String),
    /// A query went out and no answer came back.
    #[error(
        "no answer from `{target}` to `{command}` within {waited}; \
         is the cell deployed under that name? (`myrmic cells --once`, --prefix)"
    )]
    NoAnswer {
        /// The cell that was asked.
        target: String,
        /// The query that was sent.
        command: &'static str,
        /// How long the client waited.
        waited: String,
    },
    /// An answer arrived that is not a valid message of this contract.
    #[error("`{target}` answered `{command}` with a message this client cannot use: {source}")]
    BadAnswer {
        /// The cell that answered.
        target: String,
        /// The query it answered.
        command: &'static str,
        /// What is wrong with the answer.
        source: ProtocolError,
    },
    /// What the operator asked for cannot be sent.
    #[error("{0}")]
    Invalid(String),
    /// A message could not be encoded.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    /// Writing the output failed.
    #[error("cannot write output: {0}")]
    Output(#[from] std::io::Error),
}
