// SPDX-License-Identifier: AGPL-3.0-or-later
//! The card issuer, behind one interface (TASK-164). Klimper's card screens, its commands
//! and its key never name an issuer; an adapter does. Gnosis Pay (`gnosis_pay.rs`) is the
//! first. The build picks one with `KLIMPER_CARD_ISSUER` (`docs/card.md`).
//!
//! The rules every adapter keeps (owner decision 2026-10-27):
//!
//! * **Device only.** The adapter calls the issuer from the device with the user's own
//!   session. There is no Klimper server, so an issuer whose API needs a partner backend
//!   has no adapter.
//! * **The adapter checks what it signs.** It may ask the card key for a signature only
//!   through [`Signer`], and only for a message or digest it has checked first, so that
//!   nothing that can move money is ever signed.
//! * **Only what a screen shows reaches the webview**, in the shared types below.
//! * **Tokens in memory only**, errors cut down to a status and a short code (`ApiError`).

use async_trait::async_trait;
use serde::Serialize;

use crate::Result;
use crate::account::{Account, Statement};
use crate::evm::Address;
use crate::onboarding::{Step, Term};
use crate::pci::CardDetails;
use crate::siwe::SiweError;

/// The issuers this build of the plugin has an adapter for.
pub const KNOWN: &[&str] = &[crate::gnosis_pay::ID];

/// What the webview learns about the issuer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuerInfo {
    /// `gnosis-pay`; the value of `KLIMPER_CARD_ISSUER`.
    pub id: &'static str,
    /// The name the screens show, e.g. `Gnosis Pay`.
    pub name: &'static str,
    /// The card scheme, e.g. `Visa`.
    pub scheme: &'static str,
    /// The chain the card account is on, as the screens name it, e.g. `Gnosis Chain`.
    pub network: &'static str,
    /// What a top-up sends to the card account, e.g. `EURe`.
    pub top_up_asset: &'static str,
    /// `sandbox`, `production` or `custom`.
    pub environment: &'static str,
    pub capabilities: Capabilities,
}

/// What this issuer's adapter can do. A screen hides what is `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Number, expiry and CVV on the device.
    pub card_details: bool,
    /// Freeze and unfreeze.
    pub freeze: bool,
}

/// The card key, as an adapter may use it. Each call opens the wallet's key for the length
/// of one signature, after the adapter has checked what it is about to sign.
#[async_trait]
pub trait Signer: Send + Sync {
    /// The card address (EIP-55 checksummed when shown).
    fn address(&self) -> Address;
    /// EIP-191 `personal_sign` of a message the adapter has checked (a sign-in).
    async fn personal_sign(&self, message: &[u8]) -> Result<[u8; 65]>;
    /// A digest the adapter has checked (EIP-712 typed data that cannot move money).
    async fn sign_digest(&self, digest: &[u8; 32]) -> Result<[u8; 65]>;
}

/// One card issuer. Every method but [`Issuer::sign_in`] works on the session that sign-in
/// made, and fails with [`ApiError::SignedOut`] without one.
#[async_trait]
pub trait Issuer: Send + Sync + std::fmt::Debug {
    fn info(&self) -> IssuerInfo;

    /// The chain id the card key signs for, where the issuer's signatures name one.
    fn chain_id(&self) -> Option<u64> {
        None
    }

    // The session.

    /// Signs in with the card key; the session is kept for `fingerprint` only.
    async fn sign_in(&self, fingerprint: u32, signer: &dyn Signer, now: u64) -> Result<()>;
    /// The address signed in for `fingerprint`, if any. Another wallet's session is dropped.
    async fn signed_in(&self, fingerprint: u32) -> Option<Address>;
    /// Ends the session at the issuer (best effort) and forgets it here.
    async fn sign_out(&self);

    // Onboarding: the issuer decides the order; Klimper shows one step at a time.

    async fn onboarding(&self) -> Result<Step>;
    /// `true` when a code was sent, `false` when this programme needs none.
    async fn send_email_code(&self, email: &str) -> Result<bool>;
    async fn register(&self, email: &str, code: Option<&str>) -> Result<()>;
    async fn accept_terms(&self, terms: &[Term]) -> Result<()>;
    async fn answer_source_of_funds(&self, answers: &[(String, String)]) -> Result<()>;
    async fn create_account(&self) -> Result<()>;
    async fn send_phone_code(&self, phone: &str) -> Result<()>;
    async fn verify_phone(&self, code: &str) -> Result<()>;
    async fn order_virtual_card(&self, name: &str) -> Result<()>;

    // The account.

    async fn account(&self) -> Result<Account>;
    async fn statement(&self, cursor: Option<&str>) -> Result<Statement>;
    async fn set_frozen(&self, card_id: &str, frozen: bool) -> Result<()>;
    /// Number, expiry and CVV, for one screen. Only when `capabilities.card_details`.
    async fn card_details(&self, card_id: &str, signer: &dyn Signer) -> Result<CardDetails>;
}

/// What went wrong talking to the issuer. Never carries a response body: only the HTTP
/// status and the issuer's short error code, since a body can echo personal data.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ApiError {
    #[error("not signed in to the card")]
    SignedOut,
    #[error(transparent)]
    Siwe(#[from] SiweError),
    #[error("the card issuer could not be reached")]
    Network,
    #[error("the card issuer answered {status}{}", code.as_ref().map(|c| format!(" ({c})")).unwrap_or_default())]
    Status { status: u16, code: Option<String> },
    #[error("the card issuer's answer was not understood")]
    Unreadable,
}

impl ApiError {
    /// The issuer's error code, e.g. `USER_ALREADY_EXISTS`.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Status { code, .. } => code.as_deref(),
            _ => None,
        }
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;

/// The issuer this build was made for: `KLIMPER_CARD_ISSUER` and that issuer's settings.
/// `None` means the build has no card (the screen says so).
pub fn from_build() -> Option<Box<dyn Issuer>> {
    choose(option_env!("KLIMPER_CARD_ISSUER"), |id| match id {
        crate::gnosis_pay::ID => crate::gnosis_pay::Settings::from_build()
            .map(|settings| Box::new(crate::gnosis_pay::GnosisPay::new(settings)) as _),
        _ => None,
    })
}

/// `id` trimmed and known, then `build` for it. An unset or unknown issuer is no card.
fn choose<T>(id: Option<&str>, build: impl FnOnce(&str) -> Option<T>) -> Option<T> {
    let id = id.map(str::trim).filter(|id| !id.is_empty())?;
    if !KNOWN.contains(&id) {
        tracing::warn!("card: this build names an issuer it has no adapter for");
        return None;
    }
    build(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_known_issuer_is_built() {
        let built = |id| choose(id, |id: &str| Some(id.to_string()));
        assert_eq!(built(Some("gnosis-pay")), Some("gnosis-pay".to_string()));
        assert_eq!(built(Some(" gnosis-pay ")), Some("gnosis-pay".to_string()));
        assert_eq!(built(None), None);
        assert_eq!(built(Some("")), None);
        assert_eq!(built(Some("rain")), None);
        assert_eq!(built(Some("Gnosis Pay")), None);
    }
}
