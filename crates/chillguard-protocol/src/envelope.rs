//! The versioned envelope every message travels in, and its codec.

use alloc::vec::Vec;

use myrmic_sdk::{Codec, Decoder, Encoder, Json};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::validate::ValidationError;

/// The version of this wire contract. Bumped when a change would break a
/// receiver built against the previous version; the policy is in
/// `docs/decisions/0004-wire-contract.md`.
pub const SCHEMA_VERSION: u16 = 1;

/// How a payload travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Published to every subscriber under this event name.
    Event(&'static str),
    /// Sent to one cell under this command name.
    Command(&'static str),
    /// Sent back to a caller under the command name the caller supplied in
    /// its request (`ReplyTo`).
    Response,
}

/// A message body.
///
/// Implemented by every type that may be the `body` of an [`Envelope`]. The
/// implementation says how the payload travels and what, beyond its shape,
/// makes an instance valid.
pub trait Payload: Serialize + DeserializeOwned {
    /// How the payload travels, and under which name.
    const KIND: Kind;

    /// Whether a cell may only accept this payload when its development
    /// features are switched on.
    const DEV_ONLY: bool = false;

    /// Checks the content of a payload whose shape already decoded.
    ///
    /// # Errors
    ///
    /// Returns the first fault found.
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

/// A message on the wire: the schema version and a body.
///
/// ```json
/// {"v":1,"body":{...}}
/// ```
///
/// `Envelope<T>` implements the SDK's `Encoder` and `Decoder`, so it is what
/// a cell hands to `publish` or `send` and what an event handler takes as its
/// payload parameter. A command handler should take the raw bytes and call
/// [`Envelope::decode`]: the runtime redelivers a command whose payload the
/// SDK failed to decode, without end. Both directions validate the body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    /// The schema version the body was written against.
    pub v: u16,
    /// The message.
    pub body: T,
}

impl<T: Payload> Envelope<T> {
    /// Wraps a body with the current [`SCHEMA_VERSION`].
    pub const fn new(body: T) -> Self {
        Self {
            v: SCHEMA_VERSION,
            body,
        }
    }

    /// Encodes the envelope for the wire.
    ///
    /// # Errors
    ///
    /// [`ProtocolError::Invalid`] when the body fails [`Payload::validate`];
    /// [`ProtocolError::EncodingFailed`] when the codec refuses it.
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        self.body.validate()?;
        <Json as Codec>::encode(self).map_err(|_| ProtocolError::EncodingFailed)
    }

    /// Decodes an envelope from the wire.
    ///
    /// Checks, in this order: that the bytes are a JSON object with a `v`
    /// field, that `v` is the current [`SCHEMA_VERSION`], that the body has
    /// the shape of `T`, and that its content is valid.
    ///
    /// # Errors
    ///
    /// [`ProtocolError::Malformed`], [`ProtocolError::UnsupportedVersion`],
    /// or [`ProtocolError::Invalid`], for the first check that fails.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let probe: VersionProbe =
            <Json as Codec>::decode(bytes).map_err(|_| ProtocolError::Malformed)?;
        if probe.v != SCHEMA_VERSION {
            return Err(ProtocolError::UnsupportedVersion { found: probe.v });
        }
        let envelope: Self =
            <Json as Codec>::decode(bytes).map_err(|_| ProtocolError::Malformed)?;
        envelope.body.validate()?;
        Ok(envelope)
    }
}

/// Reads the version before the body is interpreted, so that a message from
/// another version is reported as such and not as malformed.
#[derive(Deserialize)]
struct VersionProbe {
    v: u16,
}

impl<T: Payload> Encoder for Envelope<T> {
    fn to_bytes(&self) -> myrmic_sdk::Result<myrmic_sdk::Bytes> {
        self.encode().map_err(|error| error.as_str())
    }
}

impl<T: Payload> Decoder for Envelope<T> {
    fn from_bytes(bytes: myrmic_sdk::Bytes) -> myrmic_sdk::Result<Self> {
        Self::decode(&bytes).map_err(|error| error.as_str())
    }
}

/// Why a message could not be encoded or decoded.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ProtocolError {
    /// The bytes are not a JSON envelope of the expected shape.
    #[error("payload is not a well-formed ChillGuard message")]
    Malformed,
    /// The envelope was written against another schema version.
    #[error("unsupported schema version {found}, this build speaks {SCHEMA_VERSION}")]
    UnsupportedVersion {
        /// The version the sender wrote.
        found: u16,
    },
    /// The shape is right but the content is not.
    #[error("invalid message content: {0}")]
    Invalid(#[from] ValidationError),
    /// The codec could not serialise the envelope.
    #[error("failed to encode message")]
    EncodingFailed,
}

impl ProtocolError {
    /// The error as the SDK's `&'static str`, for the `Encoder`/`Decoder`
    /// impls. The host logs this string when a handler's payload fails to
    /// decode.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Malformed => "payload is not a well-formed ChillGuard message",
            Self::UnsupportedVersion { .. } => "unsupported ChillGuard schema version",
            Self::Invalid(error) => error.as_str(),
            Self::EncodingFailed => "failed to encode ChillGuard message",
        }
    }
}
