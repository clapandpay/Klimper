// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pairing and SAS confirmation (clapandpay TASK-075 and TASK-076, xchonnect spec 6.3,
//! integration guide §1).
//!
//! The order is fixed and every step can only fail closed:
//!
//! 1. parse the URI, fetch the origin document fresh and verify expiry and the origin
//!    signature — [`crate::origin`], no continue-anyway path;
//! 2. show the verified domain (Unicode **and** ASCII when they differ, every homograph
//!    warning) and the dApp's name, and wait for the user;
//! 3. create the wallet's own mailbox (ticket or proof-of-work, solved off the UI thread);
//! 4. post the pairing reply. A `not_found` means the code was already used;
//! 5. show the six digits and wait up to five minutes for `session.confirm`; a timeout
//!    carries the same warning as a `not_found`;
//! 6. the user compares the digits. Matching: persist the session **before** posting
//!    `session.ready`. Different: post `session.end` and delete everything.
//!
//! Until step 6 the session is unusable: [`crate::requests::check_usable`] refuses
//! anything on a record whose `sas_confirmed` is false, and `xchonnect-core` refuses to
//! seal or open an `rpc.*` message on a session that is not active.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use xchonnect_core::crypto::{MailboxId, OsEntropy, Token};
use xchonnect_core::domain::{DomainWarning, display_domain};
use xchonnect_core::message::{Message, WalletMeta};
use xchonnect_core::pairing::{CONFIRM_TIMEOUT_S, VerifiedUri, WalletPairing};
use xchonnect_core::session::Session;
use xchonnect_wallet_kit::DappPermissions;

use crate::error::{Error, Result};
use crate::relay::{RelayClient, RelayInfo};
use crate::requests::new_id;
use crate::store::{SessionRecord, SessionStore, StoredPermissions, StoredSession};

/// Event: a pairing attempt changed state.
pub const EVENT_PAIRING: &str = "klimper-xchonnect://pairing";
/// Event: the set of connected dApps changed.
pub const EVENT_SESSIONS: &str = "klimper-xchonnect://sessions";

/// Name and link the wallet sends to the dApp.
pub const WALLET_NAME: &str = "Klimper";

/// How long one poll for `session.confirm` waits on the relay.
const CONFIRM_POLL_WAIT: u64 = 5;

/// What the pairing screen shows before the user agrees to anything.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingPreview {
    /// Hand this back to `approve_pairing` / `cancel_pairing`.
    pub pairing_id: String,
    /// Verified domain as the user reads it.
    pub domain_unicode: String,
    /// The ASCII (A-label) form. Equal to `domainUnicode` for a plain ASCII domain; the
    /// screen must show it separately when it differs.
    pub domain_ascii: String,
    /// `true` when the two differ, so the UI does not have to compare strings itself.
    pub domain_differs: bool,
    /// Homograph warnings: `NonAscii`, `MixedScripts`, … Show every one of them.
    pub warnings: Vec<String>,
    /// Name from the origin document. Untrusted text: it is whatever the domain claims,
    /// and only the domain itself was verified.
    pub dapp_name: String,
    pub relay: String,
    pub expires_at: u64,
    /// A developer-mode URI (plain-HTTP loopback). Only possible in a debug build.
    pub developer_mode: bool,
    /// What the wallet will grant: the CHIP-0002 required methods and the wallet's receive
    /// keys (spec 9.3). Nothing is auto-approved — every request still needs its own prompt.
    pub granted_methods: Vec<String>,
    /// Hex of the first exposed key (the wallet's first receive address), if it has one.
    pub granted_key: Option<String>,
    /// How many receive keys will be exposed. With them the dApp can see every receive
    /// address's balance on chain, so the screen has to say so.
    pub granted_key_count: usize,
    pub chain_id: String,
}

/// The SAS to compare, and whether the dApp has answered yet.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingSas {
    pub pairing_id: String,
    /// Six digits, zero padded. Show them large (TASK-076 AC #1).
    pub sas: String,
    /// The dApp's `session.confirm` arrived: the user may now confirm or reject.
    pub dapp_confirmed: bool,
    /// Unix seconds after which the attempt is abandoned.
    pub deadline: u64,
}

fn warning_name(warning: DomainWarning) -> String {
    format!("{warning:?}")
}

/// A pairing attempt in progress. Never persisted: a half-finished pairing is worth
/// nothing after a restart, and the pairing code is single use anyway.
pub struct Attempt {
    pub id: String,
    pub verified: VerifiedUri,
    pub developer_mode: bool,
    pub relay: RelayClient,
    pub info: RelayInfo,
    /// Wallet this pairing will belong to.
    pub fingerprint: u32,
    pub chain_id: String,
    pub permissions: DappPermissions,
    /// Set by `approve_pairing`.
    pub pairing: Option<WalletPairing>,
    pub own_mailbox: Option<MailboxId>,
    pub own_read: Option<Token>,
    /// Set once `session.confirm` arrived. Still unusable until the SAS is confirmed.
    pub session: Option<Session>,
    pub deadline: u64,
}

impl std::fmt::Debug for Attempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Attempt")
            .field("id", &self.id)
            .field("domain", &self.verified.uri().domain)
            .finish_non_exhaustive()
    }
}

impl Attempt {
    pub fn preview(&self) -> PairingPreview {
        let uri = self.verified.uri();
        let shown = display_domain(&uri.domain);
        PairingPreview {
            pairing_id: self.id.clone(),
            domain_differs: shown.unicode != shown.ascii,
            domain_unicode: shown.unicode,
            domain_ascii: shown.ascii,
            warnings: shown.warnings.into_iter().map(warning_name).collect(),
            dapp_name: self.verified.dapp_name().to_owned(),
            relay: uri.relay.clone(),
            expires_at: uri.expires_at,
            developer_mode: self.developer_mode,
            granted_methods: self.permissions.methods.clone(),
            granted_key: self
                .permissions
                .exposed_keys
                .first()
                .map(|key| format!("0x{}", hex::encode(key.to_bytes()))),
            granted_key_count: self.permissions.exposed_keys.len(),
            chain_id: self.chain_id.clone(),
        }
    }

    pub fn sas(&self) -> Option<PairingSas> {
        let pairing = self.pairing.as_ref()?;
        Some(PairingSas {
            pairing_id: self.id.clone(),
            sas: pairing.sas().digits(),
            dapp_confirmed: self.session.is_some(),
            deadline: self.deadline,
        })
    }

    /// Delete the mailbox this attempt created, if any.
    pub async fn discard(&self) {
        if let (Some(mailbox), Some(read)) = (self.own_mailbox, self.own_read.as_ref()) {
            self.relay.delete(mailbox, read).await;
        }
    }
}

/// Step 1 and 2: verify, then describe what the user has to agree to.
pub async fn inspect(
    uri_text: &str,
    now: u64,
    allow_developer_mode: bool,
    fingerprint: u32,
    chain_id: String,
    receive_keys: Vec<chia_bls::PublicKey>,
) -> Result<Attempt> {
    let (verified, developer_mode) =
        crate::origin::verify_pairing_uri(uri_text, now, allow_developer_mode).await?;
    let relay = RelayClient::new(&verified.uri().relay, developer_mode)?;
    let info = relay.info().await?;
    // Spec 9.3: the required methods, plus the wallet's receive keys rather than one fresh
    // key, so the dApp can read every receive address's coins from the chain in a few
    // batched calls instead of asking the wallet for each balance (the pairing screen says
    // how many addresses it will see). No limits are configured, so every request is
    // prompted; a per-dApp limit is set afterwards (TASK-083).
    let permissions = DappPermissions {
        // wallet-kit's defaults (signCoinSpends, xchonnect_submitCoinSpends, …) and the
        // methods Klimper builds itself (chia_send). Each asks before anything is signed.
        methods: xchonnect_wallet_kit::permissions::DEFAULT_METHODS
            .iter()
            .chain(crate::execute::WALLET_BUILT_METHODS.iter())
            .map(|m| (*m).to_owned())
            .collect(),
        exposed_keys: receive_keys,
        limits: std::collections::BTreeMap::new(),
    };
    Ok(Attempt {
        id: new_id(),
        developer_mode,
        relay,
        info,
        fingerprint,
        chain_id,
        permissions,
        pairing: None,
        own_mailbox: None,
        own_read: None,
        session: None,
        deadline: now.saturating_add(CONFIRM_TIMEOUT_S),
        verified,
    })
}

/// Steps 3 to 5: create the mailbox, post the reply, and show the SAS.
pub async fn reply(attempt: &mut Attempt, now: u64) -> Result<PairingSas> {
    if attempt.pairing.is_some() {
        // Already replied; show the same SAS rather than burning a second mailbox.
        return attempt.sas().ok_or(Error::UnknownPairing);
    }
    let read = Token::random(&mut OsEntropy);
    let write = Token::random(&mut OsEntropy);
    let mailbox = attempt
        .relay
        .create_mailbox(&read, &write, attempt.verified.uri().ticket.as_ref())
        .await?;

    let meta = WalletMeta {
        name: Some(WALLET_NAME.to_owned()),
        ..WalletMeta::default()
    };
    let (pairing, outgoing) = WalletPairing::reply(
        &mut OsEntropy,
        now,
        &attempt.verified,
        mailbox,
        read.clone(),
        write,
        Some(meta),
    )
    .map_err(Error::protocol)?;

    // A 404 here is the relay saying the pairing mailbox is gone: the code was used.
    if let Err(error) = attempt.relay.post(&outgoing).await {
        attempt.relay.delete(mailbox, &read).await;
        return Err(error);
    }
    attempt.own_mailbox = Some(mailbox);
    attempt.own_read = Some(read);
    attempt.deadline = now.saturating_add(CONFIRM_TIMEOUT_S);
    attempt.pairing = Some(pairing);
    attempt.sas().ok_or(Error::UnknownPairing)
}

/// One poll for `session.confirm`. `Ok(true)` means the session was created.
pub async fn poll_for_confirm(attempt: &mut Attempt, now: u64) -> Result<bool> {
    let (Some(pairing), Some(mailbox), Some(read)) = (
        attempt.pairing.as_ref(),
        attempt.own_mailbox,
        attempt.own_read.as_ref(),
    ) else {
        return Err(Error::UnknownPairing);
    };
    if attempt.session.is_some() {
        return Ok(true);
    }
    if pairing.timed_out(now) {
        return Err(Error::PairingCodeUsed);
    }
    let messages = attempt
        .relay
        .fetch(mailbox, read, CONFIRM_POLL_WAIT, &attempt.info)
        .await?;
    let mut ids = Vec::new();
    let mut session = None;
    for (id, envelope) in messages {
        ids.push(id);
        if session.is_none() && !envelope.is_empty() {
            match pairing.on_confirm(now, &envelope) {
                Ok(created) => session = Some(created),
                // Anything else on this mailbox before the session exists is noise or an
                // attacker's envelope: dropped without its content being logged.
                Err(error) => {
                    tracing::debug!("ignored an envelope on the pairing mailbox: {error}");
                }
            }
        }
    }
    attempt.relay.ack(mailbox, read, &ids).await?;
    if let Some(created) = session {
        attempt.session = Some(created);
        return Ok(true);
    }
    Ok(false)
}

/// How long to wait between polls when the relay returned nothing.
pub const POLL_GAP: Duration = Duration::from_millis(250);

/// Step 6, matching digits: persist first, then post `session.ready` and the granted
/// permissions.
///
/// Returns the stored session. The caller starts the message loop.
pub async fn confirm(
    attempt: &mut Attempt,
    store: &Arc<SessionStore>,
    now: u64,
) -> Result<StoredSession> {
    let mut session = attempt.session.take().ok_or(Error::UnknownPairing)?;
    let uri = attempt.verified.uri();
    let meta = WalletMeta {
        name: Some(WALLET_NAME.to_owned()),
        ..WalletMeta::default()
    };
    let ready = session
        .confirm_sas(&mut OsEntropy, now, Some(meta))
        .map_err(Error::protocol)?;

    let mut stored = StoredSession {
        record: SessionRecord {
            id: attempt.id.clone(),
            fingerprint: attempt.fingerprint,
            dapp: uri.domain.clone(),
            dapp_name: attempt.verified.dapp_name().to_owned(),
            relay: uri.relay.clone(),
            chain_id: attempt.chain_id.clone(),
            created_at: now,
            state_version: 0,
            developer_mode: attempt.developer_mode,
            sas_confirmed: true,
            needs_repair: false,
            permissions: StoredPermissions::from_kit(&attempt.permissions),
            allow_agg_sig_unsafe: false,
            allow_unknown_contracts: false,
            origin_kid: Some(uri.kid.clone()),
            revoked: false,
            push: None,
        },
        session_hex: hex::encode(session.to_bytes().map_err(Error::protocol)?),
    };
    // Persist before posting: a crash here must never leave the dApp holding an envelope
    // whose `seq` the wallet would reuse (integration guide §2).
    store.save(&mut stored)?;

    if let Some(ready) = ready
        && let Err(error) = attempt.relay.post_retrying(&ready).await
    {
        // The dApp never learns the session exists, so keeping it would list a connection
        // that cannot work and leave the website waiting. Undo it; the caller discards the
        // attempt and its mailbox, and the user connects again.
        if let Err(remove) = store.remove(stored.record.fingerprint, &stored.record.id) {
            tracing::warn!("could not remove a half-made session: {remove}");
        }
        return Err(error);
    }
    // Tell the dApp what it may ask for, so it does not have to learn by being refused
    // (spec 9.3). This changes the session state, so it is persisted first as well.
    let permissions = Message::SessionPermissions(attempt.permissions.to_message());
    let outgoing = session
        .seal(&mut OsEntropy, now, permissions, 3600)
        .map_err(Error::protocol)?;
    stored.session_hex = hex::encode(session.to_bytes().map_err(Error::protocol)?);
    store.save(&mut stored)?;
    if let Err(error) = attempt.relay.post(&outgoing).await {
        // Not fatal: the dApp discovers the limits by being refused.
        tracing::warn!("could not send the granted permissions: {error}");
    }
    attempt.session = Some(session);
    Ok(stored)
}

/// Step 6, different digits: end the session, post the rejection and delete everything.
pub async fn reject(attempt: &mut Attempt, now: u64) -> Result<()> {
    if let Some(mut session) = attempt.session.take() {
        match session.reject_sas(&mut OsEntropy, now) {
            Ok(outgoing) => {
                if let Err(error) = attempt.relay.post(&outgoing).await {
                    tracing::warn!("could not tell the dApp the codes did not match: {error}");
                }
            }
            Err(error) => tracing::warn!("could not build the rejection: {error}"),
        }
    }
    // Nothing was ever persisted for this attempt, so deleting the mailbox is all the
    // clean-up there is.
    attempt.discard().await;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testing;

    fn attempt_for(f: &mut testing::Fixture) -> Attempt {
        let (_, uri_text) = testing::pairing_uri(f);
        let uri = xchonnect_core::uri::PairingUri::parse(
            &uri_text,
            xchonnect_core::uri::ParseOptions::default(),
        )
        .unwrap();
        let verified = VerifiedUri::new(uri, &f.doc, testing::NOW + 1).unwrap();
        let key = chia_bls::SecretKey::from_seed(&[5; 64]).public_key();
        Attempt {
            id: "p1".to_owned(),
            developer_mode: false,
            relay: RelayClient::new("https://relay.example", false).unwrap(),
            info: RelayInfo::default(),
            fingerprint: 1,
            chain_id: "testnet11".to_owned(),
            permissions: DappPermissions::new_default(key),
            pairing: None,
            own_mailbox: None,
            own_read: None,
            session: None,
            deadline: testing::NOW + CONFIRM_TIMEOUT_S,
            verified,
        }
    }

    #[test]
    fn the_preview_carries_both_domain_forms_and_the_default_grant() {
        let mut f = testing::fixture();
        let preview = attempt_for(&mut f).preview();
        assert_eq!(preview.domain_ascii, testing::DOMAIN);
        assert_eq!(preview.domain_unicode, testing::DOMAIN);
        assert!(!preview.domain_differs);
        assert_eq!(preview.dapp_name, "Pengui");
        // Spec 9.3: the required methods and the exposed receive keys.
        assert_eq!(
            preview.granted_methods,
            xchonnect_wallet_kit::permissions::DEFAULT_METHODS.to_vec()
        );
        assert!(preview.granted_key.is_some());
        assert_eq!(preview.granted_key_count, 1);
    }

    /// A homograph domain must reach the screen with both forms and a warning.
    #[test]
    fn a_non_ascii_domain_is_shown_in_both_forms_with_warnings() {
        let shown = display_domain("xn--pengu-rta.xyz");
        assert_ne!(shown.unicode, shown.ascii);
        assert!(!shown.warnings.is_empty());
        let names: Vec<String> = shown.warnings.into_iter().map(warning_name).collect();
        assert!(names.iter().any(|w| w == "NonAscii"), "{names:?}");
    }

    /// `session.ready` is only ever produced by `confirm_sas`, and the session is not
    /// active before it.
    #[test]
    fn a_session_is_inactive_until_the_sas_is_confirmed() {
        let mut f = testing::fixture();
        let (_, mut wallet, _) = testing::handshake_without_sas(&mut f);
        assert!(!wallet.is_active());
        let ready = wallet
            .confirm_sas(&mut OsEntropy, testing::NOW + 9, None)
            .unwrap();
        assert!(ready.is_some() && wallet.is_active());
    }
}
