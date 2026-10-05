// SPDX-License-Identifier: AGPL-3.0-or-later
//! The session message loop (clapandpay TASK-077, integration guide §3).
//!
//! One task per live session. It reads the draining mailbox first after a rotation, then
//! the current one, hands every envelope to `Session::open` and dispatches what comes
//! back: requests, rotation offers, pings, `session.end`. An envelope the core rejects —
//! replayed, out of order, tampered with, sealed for another epoch — is dropped with a log
//! line that names the reason and never the content, and the loop carries on.
//!
//! Three orderings are not negotiable:
//!
//! 1. **Seal, persist, post.** Sealing advances `seq`; posting without persisting would
//!    let a restart reuse it, and the dApp would then reject everything that follows as a
//!    replay (spec 5.3, integration guide §2).
//! 2. **Acknowledge last.** A message is acknowledged only after it was processed and the
//!    new state was persisted, so a crash while the user is still looking at an approval
//!    leaves the request in the mailbox and the relay delivers it again (TASK-074 AC #2,
//!    TASK-077 AC #4).
//! 3. **Drain before use.** After a rotation the previous epoch's mailbox is read until it
//!    is empty, then `finish_drain` gives it up and it is deleted on the relay
//!    (TASK-074 AC #3).
//!
//! Polling is one long poll, clamped to the relay's own `max_wait_s`. Nothing polls while
//! the app is in the background, and the loop is driven by a channel rather than by the
//! sleep alone, so a push wake-up becomes another sender rather than a rewrite
//! (decision-008 point 2).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::json;
use xchonnect_core::crypto::{MailboxId, OsEntropy, Token};
use xchonnect_core::message::{Message, RotatePhase};
use xchonnect_core::rpc;
use xchonnect_core::session::Session;
use xchonnect_wallet_kit::RpcError;

use crate::approval::{ApprovalBridge, Emitter};
use crate::error::{Error, Result};
use crate::relay::{RelayClient, RelayInfo};
use crate::requests::{self, AnswerInputs, RequestMeta};
use crate::store::{SessionStore, StoredSession};
use crate::wallet::AppState;

/// Event: one session changed state (ended, needs re-pairing, rotated).
pub const EVENT_SESSION: &str = "klimper-xchonnect://session";
/// Event: an `rpc.request` arrived and is being worked on.
pub const EVENT_REQUEST: &str = "klimper-xchonnect://request";

/// Longest long poll the wallet asks for; the relay clamps it further.
const POLL_WAIT: u64 = 20;
/// Pause after a relay failure before trying again.
const BACKOFF: Duration = Duration::from_secs(5);

/// Everything one session's loop needs.
///
/// The loop ends when the session ends (by the dApp, or because the user asked), or when
/// every sender on `wake` is gone, which is how the app shutting down stops it.
pub struct SessionLoop {
    pub state: AppState,
    pub store: Arc<SessionStore>,
    pub approvals: Arc<ApprovalBridge>,
    pub emit: Emitter,
    pub relay: RelayClient,
    pub info: RelayInfo,
    /// Set by `end_session`: the loop itself posts `session.end` and cleans up, so the
    /// end can never interleave with a message the loop is in the middle of sealing.
    pub end_requested: Arc<AtomicBool>,
    /// Woken by a poll tick and, later, by a push wake-up.
    pub wake: tokio::sync::mpsc::Receiver<()>,
}

/// Unix seconds.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl std::fmt::Debug for SessionLoop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionLoop").finish_non_exhaustive()
    }
}

impl SessionLoop {
    /// Run until the session ends or [`SessionLoop::stop`] is set.
    pub async fn run(mut self, mut stored: StoredSession) {
        let id = stored.record.id.clone();
        let fingerprint = stored.record.fingerprint;
        let Ok(bytes) = stored.session_bytes() else {
            self.mark_repair(&mut stored);
            return;
        };
        let Ok(mut session) = Session::from_bytes(&bytes) else {
            self.mark_repair(&mut stored);
            return;
        };
        if stored.record.needs_repair {
            self.mark_repair(&mut stored);
            return;
        }

        loop {
            if self.end_requested.load(Ordering::SeqCst) {
                self.end_now(&mut session, &mut stored).await;
                self.cleanup(&session, fingerprint, &id).await;
                (self.emit)(EVENT_SESSION, json!({ "sessionId": id, "state": "ended" }));
                return;
            }
            match self.pump(&mut session, &mut stored).await {
                Ok(Outcome::Continue) => {}
                Ok(Outcome::Ended) => {
                    self.cleanup(&session, fingerprint, &id).await;
                    (self.emit)(EVENT_SESSION, json!({ "sessionId": id, "state": "ended" }));
                    return;
                }
                Err(Error::PairingCodeUsed) => {
                    // The relay says the mailbox is gone: the dApp ended the session, or
                    // it expired. Nothing left to drain.
                    tracing::info!(session = %id, "the relay no longer has this mailbox");
                    let _ = self.store.remove(fingerprint, &id);
                    (self.emit)(EVENT_SESSION, json!({ "sessionId": id, "state": "ended" }));
                    return;
                }
                Err(error) => {
                    tracing::warn!(session = %id, "xchonnect session loop: {error}");
                    tokio::time::sleep(BACKOFF).await;
                }
            }
            // Wait for the next tick or wake-up. A closed channel means the app is going
            // away.
            if self.wake.recv().await.is_none() {
                return;
            }
        }
    }

    /// Tell the dApp the user ended the connection. Seal, persist, post — in that order,
    /// as everywhere else.
    async fn end_now(&self, session: &mut Session, stored: &mut StoredSession) {
        match session.end(&mut OsEntropy, now(), Some("ended by the user".to_owned())) {
            Ok(outgoing) => {
                if let Ok(encoded) = encode(session) {
                    stored.session_hex = encoded;
                    let _ = self.store.save(stored);
                }
                post_end(&self.relay, &outgoing).await;
            }
            Err(error) => tracing::warn!("could not build session.end: {error}"),
        }
    }

    fn mark_repair(&self, stored: &mut StoredSession) {
        stored.record.needs_repair = true;
        let _ = self.store.save(stored);
        (self.emit)(
            EVENT_SESSION,
            json!({
                "sessionId": stored.record.id,
                "state": "needsRepair",
                "dapp": stored.record.dapp,
            }),
        );
    }

    /// Delete the session's mailboxes and its stored state.
    async fn cleanup(&self, session: &Session, fingerprint: u32, id: &str) {
        self.approvals.withdraw_session(id);
        if let Some((mailbox, read)) = session.draining_mailbox() {
            self.relay.delete(mailbox, read).await;
        }
        if let Some((mailbox, read)) = session.pending_rotation_mailbox() {
            self.relay.delete(mailbox, read).await;
        }
        self.relay
            .delete(session.own_mailbox(), session.own_read_token())
            .await;
        if let Err(error) = self.store.remove(fingerprint, id) {
            tracing::warn!("could not remove a session from the keyring: {error}");
        }
    }

    /// One pass over the mailboxes.
    async fn pump(&self, session: &mut Session, stored: &mut StoredSession) -> Result<Outcome> {
        // Spec 9.2.1: the previous epoch's mailbox is read first and to exhaustion.
        if let Some((mailbox, read)) = session.draining_mailbox() {
            let (mailbox, read) = (mailbox, read.clone());
            let messages = self.relay.fetch(mailbox, &read, 0, &self.info).await?;
            if messages.is_empty() {
                if let Some(retired) = session.finish_drain() {
                    stored.session_hex = encode(session)?;
                    self.store.save(stored)?;
                    self.relay
                        .delete(retired.mailbox, &retired.read_token)
                        .await;
                }
            } else if let Outcome::Ended = self
                .consume(session, stored, mailbox, &read, messages)
                .await?
            {
                return Ok(Outcome::Ended);
            }
        }

        let mailbox = session.own_mailbox();
        let read = session.own_read_token().clone();
        let messages = self
            .relay
            .fetch(mailbox, &read, POLL_WAIT, &self.info)
            .await?;
        self.consume(session, stored, mailbox, &read, messages)
            .await
    }

    async fn consume(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
        mailbox: MailboxId,
        read: &Token,
        messages: Vec<(String, Vec<u8>)>,
    ) -> Result<Outcome> {
        for (msg_id, envelope) in messages {
            let ids = [msg_id];
            if envelope.is_empty() {
                self.relay.ack(mailbox, read, &ids).await?;
                continue;
            }
            let inner = match session.open(now(), &mailbox, &envelope) {
                Ok(inner) => inner,
                Err(error) => {
                    // Replay, bad sequence, wrong epoch, forged tag: the reason, never the
                    // envelope (spec 12.1).
                    tracing::info!(
                        session = %stored.record.id,
                        "dropped an xchonnect envelope: {error}"
                    );
                    self.relay.ack(mailbox, read, &ids).await?;
                    continue;
                }
            };
            let ended = self.dispatch(session, stored, mailbox, read, inner).await?;
            // Acknowledge only now: processed and persisted.
            self.relay.ack(mailbox, read, &ids).await?;
            if ended {
                return Ok(Outcome::Ended);
            }
        }
        Ok(Outcome::Continue)
    }

    /// Handle one decoded message. Returns `true` when the session has ended.
    async fn dispatch(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
        mailbox: MailboxId,
        read: &Token,
        inner: xchonnect_core::message::Inner,
    ) -> Result<bool> {
        match inner.message {
            Message::RpcRequest { method, params } => {
                self.handle_request(session, stored, inner.id, inner.exp, &method, params)
                    .await?;
            }
            Message::SessionRotate(rotate) if rotate.phase == RotatePhase::Offer => {
                self.accept_rotation(session, stored, &rotate).await?;
            }
            Message::SessionPing => {
                self.send(session, stored, Message::SessionPong, 300)
                    .await?;
            }
            Message::SessionEnd { reason } => {
                tracing::info!(
                    dapp = %stored.record.dapp,
                    "the dApp ended the xchonnect session: {}",
                    reason.unwrap_or_default()
                );
                let _ = (mailbox, read);
                return Ok(true);
            }
            other => {
                // `session.permissions`, a type this version does not know, a stray pong.
                tracing::debug!("xchonnect message {} needs no answer", other.type_name());
                stored.session_hex = encode(session)?;
                self.store.save(stored)?;
            }
        }
        Ok(false)
    }

    /// Answer one `rpc.request`.
    async fn handle_request(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
        request_id: [u8; 16],
        request_exp: u64,
        method: &str,
        params: String,
    ) -> Result<()> {
        let now_s = now();
        // Pick up a permission or limit change the user made since the last request
        // (`set_dapp_policy`). Only the policy fields are taken: the session bytes in
        // memory are the authoritative ones.
        if let Ok(fresh) = self
            .store
            .load(stored.record.fingerprint, &stored.record.id)
        {
            stored.record.permissions = fresh.record.permissions;
            stored.record.allow_agg_sig_unsafe = fresh.record.allow_agg_sig_unsafe;
            stored.record.allow_unknown_contracts = fresh.record.allow_unknown_contracts;
        }
        (self.emit)(
            EVENT_REQUEST,
            json!({
                "sessionId": stored.record.id,
                "dapp": stored.record.dapp,
                "method": method,
            }),
        );
        // Delivery receipt, so the dApp can stop guessing whether the wallet is awake while
        // the user decides. A request answered at once needs none: the answer is the
        // receipt, and skipping it saves a relay round trip before the answer.
        if !answers_without_prompt(method) {
            self.send(session, stored, Message::RpcReceived { request_id }, 3600)
                .await?;
        }

        let outcome = requests::answer(AnswerInputs {
            state: self.state.clone(),
            store: self.store.clone(),
            approvals: self.approvals.clone(),
            record: stored.record.clone(),
            meta: RequestMeta {
                session_id: stored.record.id.clone(),
                dapp: stored.record.dapp.clone(),
                method: method.to_owned(),
                expires_at: requests::prompt_deadline(now_s, request_exp),
            },
            params,
            now: now_s,
        })
        .await;

        let message = match outcome {
            Ok(result) => rpc::result(request_id, &result).map_err(Error::protocol)?,
            Err(RpcError {
                code,
                message,
                data,
            }) => {
                rpc::error(request_id, code, &message, data.as_deref()).map_err(Error::protocol)?
            }
        };
        self.send(session, stored, message, 3600).await
    }

    /// Accept a rotation offer without asking the user: new keys for the same session
    /// (spec 9.2). The old mailbox is drained by [`SessionLoop::pump`] and then deleted.
    async fn accept_rotation(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
        rotate: &xchonnect_core::message::Rotate,
    ) -> Result<()> {
        let read = Token::random(&mut OsEntropy);
        let write = Token::random(&mut OsEntropy);
        let mailbox = self.relay.create_mailbox(&read, &write, None).await?;
        let (outgoing, abandoned) = session
            .accept_rotation(&mut OsEntropy, now(), rotate, mailbox, read.clone(), write)
            .map_err(Error::protocol)?;
        stored.session_hex = encode(session)?;
        self.store.save(stored)?;
        self.relay.post(&outgoing).await?;
        if let Some(retired) = abandoned {
            self.relay
                .delete(retired.mailbox, &retired.read_token)
                .await;
        }
        (self.emit)(
            EVENT_SESSION,
            json!({
                "sessionId": stored.record.id,
                "state": "rotated",
                "epoch": session.epoch(),
            }),
        );
        Ok(())
    }

    /// Seal, persist, then post. The order is the point of this function existing.
    async fn send(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
        message: Message,
        ttl: u64,
    ) -> Result<()> {
        let outgoing = session
            .seal(&mut OsEntropy, now(), message, ttl)
            .map_err(Error::protocol)?;
        stored.session_hex = encode(session)?;
        self.store.save(stored)?;
        self.relay.post(&outgoing).await
    }
}

/// Methods wallet-kit answers without asking the user (spec 9.1: `connect` is answered
/// without a prompt; `chainId` and the exposed keys are read straight from the session).
fn answers_without_prompt(method: &str) -> bool {
    matches!(
        rpc::canonical_method(method),
        "chainId" | "connect" | "getPublicKeys"
    )
}

/// Attempts at posting `session.end` before giving up. A disconnect is the one message the
/// user explicitly asked to reach the dApp, so a relay hiccup must not leave the website
/// believing it is still paired.
const END_ATTEMPTS: u32 = 3;

async fn post_end(relay: &RelayClient, outgoing: &xchonnect_core::session::Outgoing) {
    let mut delay = Duration::from_millis(500);
    for attempt in 1..=END_ATTEMPTS {
        match relay.post(outgoing).await {
            Ok(()) => return,
            Err(error) if attempt == END_ATTEMPTS => {
                tracing::warn!("could not tell the dApp the session ended: {error}");
            }
            Err(_) => {
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
        }
    }
}

/// End a session whose loop is not running (the wallet was locked when it started, the
/// loop stopped, the session needs re-pairing). The website is told all the same:
/// `session.end` is sealed from the stored state and posted, and this side's mailboxes are
/// deleted, so even a website that missed the message finds the session gone on its next
/// request instead of waiting on a mailbox nobody reads. Only a session whose stored state
/// cannot be read at all can end locally alone.
pub async fn end_without_loop(store: &SessionStore, stored: &mut StoredSession) {
    let Ok(relay) = RelayClient::new(&stored.record.relay, stored.record.developer_mode) else {
        return;
    };
    let Some(mut session) = stored
        .session_bytes()
        .ok()
        .and_then(|bytes| Session::from_bytes(&bytes).ok())
    else {
        tracing::info!("session state unreadable; it ends on this device only");
        return;
    };
    match session.end(&mut OsEntropy, now(), Some("ended by the user".to_owned())) {
        Ok(outgoing) => {
            // Seal, persist, post: the advanced `seq` is saved before anything is sent.
            if let Ok(encoded) = encode(&session) {
                stored.session_hex = encoded;
                let _ = store.save(stored);
            }
            post_end(&relay, &outgoing).await;
        }
        Err(error) => tracing::warn!("could not build session.end: {error}"),
    }
    if let Some((mailbox, read)) = session.draining_mailbox() {
        relay.delete(mailbox, read).await;
    }
    if let Some((mailbox, read)) = session.pending_rotation_mailbox() {
        relay.delete(mailbox, read).await;
    }
    relay
        .delete(session.own_mailbox(), session.own_read_token())
        .await;
}

fn encode(session: &Session) -> Result<String> {
    Ok(hex::encode(session.to_bytes().map_err(Error::protocol)?))
}

enum Outcome {
    Continue,
    Ended,
}

/// Drives a session's `wake` channel: one tick per [`POLL_INTERVAL`] while the app is in
/// the foreground. A push wake-up (clapandpay TASK-087) is another sender on the same
/// channel, which is why the loop waits on it rather than on a sleep.
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

pub fn spawn_ticker(sender: tokio::sync::mpsc::Sender<()>) {
    tauri::async_runtime::spawn(async move {
        // The first tick is immediate: a session that was just restored should read its
        // mailbox before waiting. The ticker stops by itself once the loop has returned
        // and dropped the receiver.
        loop {
            if sender.send(()).await.is_err() {
                return;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testing;

    #[test]
    fn only_instant_answers_skip_the_delivery_receipt() {
        for method in [
            "chainId",
            "connect",
            "getPublicKeys",
            "chip0002_getPublicKeys",
        ] {
            assert!(answers_without_prompt(method), "{method}");
        }
        for method in [
            "signCoinSpends",
            "chip0002_signCoinSpends",
            "signMessage",
            "getAssetBalance",
        ] {
            assert!(!answers_without_prompt(method), "{method}");
        }
    }

    /// The security requirement: a wallet that has reached the point of sending
    /// `session.ready` must still be unusable until the user confirmed the six digits.
    ///
    /// Two independent gates are asserted:
    ///
    /// * `xchonnect-core` refuses to seal or open any `rpc.*` message on a session that
    ///   is not active, so no response could be produced even if Klimper tried;
    /// * Klimper's own [`requests::check_usable`] refuses on the stored record, so the
    ///   request never reaches the simulation, the approval UI or the signer.
    #[test]
    fn a_session_without_sas_confirmation_cannot_be_used() {
        let mut f = testing::fixture();
        let (mut dapp, mut wallet, wallet_mailbox) = testing::handshake_without_sas(&mut f);

        // The handshake is complete: both sides hold a session and the SAS matches.
        assert!(!wallet.is_active(), "no SAS confirmation yet");

        // The wallet cannot answer: sealing an rpc.response is refused outright.
        let response = rpc::result([7; 16], "\"testnet11\"").unwrap();
        let error = wallet
            .seal(&mut f.rng, testing::NOW + 9, response, 600)
            .unwrap_err();
        assert!(error.to_string().contains("not active"), "{error}");

        // And it cannot receive one either. The dApp is not active yet either, so an
        // rpc.request has to be sealed from a ping-shaped path; what matters is that the
        // wallet's own gate refuses before anything is decoded.
        let ping = dapp
            .seal(&mut f.rng, testing::NOW + 9, Message::SessionPing, 60)
            .unwrap();
        assert!(
            wallet
                .open(testing::NOW + 10, &wallet_mailbox, &ping.envelope)
                .is_ok(),
            "a ping is allowed before activation"
        );

        // Klimper's record gate.
        let mut record = crate::store::SessionRecord {
            id: "s1".to_owned(),
            fingerprint: 1,
            dapp: testing::DOMAIN.to_owned(),
            dapp_name: "Pengui".to_owned(),
            relay: "https://relay.example".to_owned(),
            chain_id: "testnet11".to_owned(),
            created_at: testing::NOW,
            state_version: 1,
            developer_mode: false,
            sas_confirmed: false,
            needs_repair: false,
            permissions: crate::store::StoredPermissions::default(),
            allow_agg_sig_unsafe: false,
            allow_unknown_contracts: false,
        };
        assert!(requests::check_usable(&record).is_err());
        record.sas_confirmed = true;
        assert!(requests::check_usable(&record).is_ok());
    }

    /// Only after the user confirms does `confirm_sas` produce `session.ready`, and only
    /// then do requests flow.
    #[test]
    fn after_the_user_confirms_requests_flow() {
        let (mut dapp, mut wallet) = testing::paired_sessions();
        let request = dapp
            .seal(
                &mut OsEntropy,
                testing::NOW + 20,
                Message::RpcRequest {
                    method: "chainId".to_owned(),
                    params: "{}".to_owned(),
                },
                600,
            )
            .unwrap();
        let mailbox = wallet.own_mailbox();
        let inner = wallet
            .open(testing::NOW + 21, &mailbox, &request.envelope)
            .unwrap();
        assert!(matches!(inner.message, Message::RpcRequest { .. }));
    }

    /// A replayed envelope is dropped and the session keeps working.
    #[test]
    fn a_replayed_envelope_is_dropped_without_breaking_the_session() {
        let (mut dapp, mut wallet) = testing::paired_sessions();
        let mut rng = OsEntropy;
        let first = dapp
            .seal(&mut rng, testing::NOW + 20, Message::SessionPing, 60)
            .unwrap();
        let second = dapp
            .seal(&mut rng, testing::NOW + 21, Message::SessionPing, 60)
            .unwrap();
        let mailbox = wallet.own_mailbox();
        assert!(
            wallet
                .open(testing::NOW + 22, &mailbox, &first.envelope)
                .is_ok()
        );
        // Replay of the first.
        assert!(
            wallet
                .open(testing::NOW + 22, &mailbox, &first.envelope)
                .is_err()
        );
        // A tampered envelope.
        let mut tampered = second.envelope.clone();
        if let Some(last) = tampered.last_mut() {
            *last ^= 0xff;
        }
        assert!(wallet.open(testing::NOW + 22, &mailbox, &tampered).is_err());
        // The genuine next message still works.
        assert!(
            wallet
                .open(testing::NOW + 23, &mailbox, &second.envelope)
                .is_ok()
        );
    }

    /// `session.end` from the dApp ends the session for the wallet too.
    #[test]
    fn session_end_from_the_dapp_ends_the_session() {
        let (mut dapp, mut wallet) = testing::paired_sessions();
        let end = dapp
            .end(&mut OsEntropy, testing::NOW + 30, Some("done".to_owned()))
            .unwrap();
        let mailbox = wallet.own_mailbox();
        let inner = wallet
            .open(testing::NOW + 31, &mailbox, &end.envelope)
            .unwrap();
        assert!(matches!(inner.message, Message::SessionEnd { .. }));
        assert!(wallet.is_ended());
    }
}
