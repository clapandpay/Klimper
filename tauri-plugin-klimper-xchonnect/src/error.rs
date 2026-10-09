// SPDX-License-Identifier: AGPL-3.0-or-later
//! Errors shown to the user and written to the log.
//!
//! No variant carries key material, a capability token, a mailbox read/write token or
//! envelope bytes: this type is serialised straight into the webview and into
//! `tracing` (xchonnect spec 12.1, "never log secrets"). `xchonnect_core::Error` is
//! rendered through its own `Display`, which names the failure without echoing input.

use serde::{Serialize, Serializer};

/// Everything that can stop an Xchonnect operation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The pairing URI does not parse, has expired, or asks for developer mode in a
    /// release build.
    #[error("this pairing code is not valid: {0}")]
    InvalidUri(String),

    /// Developer-mode URIs (plain-HTTP loopback relay) are refused outside debug builds.
    #[error("developer-mode pairing codes are only accepted in a development build")]
    DeveloperModeRefused,

    /// The origin document could not be fetched under the rules of spec 6.1.
    #[error("could not check who this website is: {0}")]
    Origin(String),

    /// Expiry or the origin signature did not verify. Pairing stops here; there is no
    /// continue-anyway path (spec 6.3 step 2).
    #[error("this website could not prove it owns its domain, so pairing was stopped: {0}")]
    OriginVerification(String),

    /// The relay refused or could not be reached.
    #[error("the dApp's relay could not be reached: {0}")]
    Relay(String),

    /// The pairing code was already used, or the dApp never confirmed.
    #[error(
        "this pairing code was already used or has expired; another device may have paired with it"
    )]
    PairingCodeUsed,

    /// A protocol-level failure from `xchonnect-core`.
    #[error("xchonnect: {0}")]
    Protocol(String),

    /// No such pairing attempt (or it was already finished or cancelled).
    #[error("that pairing attempt is no longer open")]
    UnknownPairing,

    /// No such session.
    #[error("that connection no longer exists")]
    UnknownSession,

    /// The session exists but may not be used: the SAS was never confirmed, or stored
    /// state was lost or rolled back (spec 5.3, 6.3 step 8).
    #[error("this connection is not confirmed and cannot be used; pair with the dApp again")]
    SessionNotUsable,

    /// Session storage (the OS keyring) failed.
    #[error("the system keychain could not store this connection: {0}")]
    Keyring(String),

    /// The wallet holds no secret key (a watch-only wallet).
    #[error("this wallet has no signing key")]
    NoSigningKey,

    /// No wallet is selected.
    #[error("no wallet is open")]
    NoWallet,

    /// The Sage engine failed.
    #[error("wallet: {0}")]
    Sage(String),

    /// No approval is outstanding under that id.
    #[error("that approval is no longer waiting")]
    UnknownApproval,

    /// An operation that must not block the UI could not be scheduled.
    #[error("internal: {0}")]
    Internal(String),
}

impl Error {
    /// Taken by value so it can be used as `map_err(Error::protocol)`.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn protocol(error: xchonnect_core::Error) -> Self {
        Error::Protocol(error.to_string())
    }

    pub(crate) fn internal(error: impl std::fmt::Display) -> Self {
        Error::Internal(error.to_string())
    }
}

impl From<sage::Error> for Error {
    fn from(error: sage::Error) -> Self {
        Error::Sage(error.to_string())
    }
}

impl From<sage_keychain::KeychainError> for Error {
    fn from(error: sage_keychain::KeychainError) -> Self {
        Error::Sage(error.to_string())
    }
}

impl From<sage_database::DatabaseError> for Error {
    fn from(error: sage_database::DatabaseError) -> Self {
        Error::Sage(error.to_string())
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
