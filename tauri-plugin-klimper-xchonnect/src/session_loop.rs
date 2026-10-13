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
//!    new state was persisted, so a crash before that leaves it in the mailbox and the relay
//!    delivers it again (TASK-074 AC #2, TASK-077 AC #4). A request that reached the user
//!    is the exception by design: its delivery receipt persisted the session, so after a
//!    crash a redelivery is a replay and the dApp's request expires (4100) instead of
//!    prompting twice. Messages read while a request waits are acknowledged as they are
//!    read for the same reason.
//! 3. **Drain before use.** After a rotation the previous epoch's mailbox is read until it
//!    is empty, then `finish_drain` gives it up and it is deleted on the relay
//!    (TASK-074 AC #3).
//!
//! Polling is one long poll, clamped to the relay's own `max_wait_s`. Nothing polls while
//! the app is in the background, and the loop is driven by a channel rather than by the
//! sleep alone, so a push wake-up becomes another sender rather than a rewrite
//! (decision-008 point 2).
//!
//! While a request waits for the user, the loop keeps reading the mailbox (TASK-100): an
//! `rpc.cancel` for it withdraws the prompt and is answered 4102, progress is reported as
//! `rpc.status` (shown, approved, broadcast), reads that never prompt (keys, network,
//! balances, coins) are answered at once, and every other message waits until the request
//! is answered (TASK-125.03): a website's wallet info never waits behind a signing prompt.
//!
//! Phones (TASK-125.02): the first long poll goes out before anything else a new loop does
//! (the relay's limits, the origin check, push registration), since that is when a website
//! that has just paired sends its first reads. A request is never dropped once opened: its
//! delivery receipt is best-effort, and its answer is posted again until the request
//! expires.
//!
//! A disconnect does not wait for the long poll: `interrupt` cuts the wait short, and no
//! further message is dispatched once the end was asked for. Only the poll is raced, never
//! a dispatch, so an end can never land between sealing a message and persisting it.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::Notify;

use serde_json::json;
use xchonnect_core::b64;
use xchonnect_core::crypto::{MailboxId, OsEntropy, Token};
use xchonnect_core::message::{Inner, Message, RotatePhase};
use xchonnect_core::rpc;
use xchonnect_core::session::Session;
use xchonnect_wallet_kit::RpcError;

use xchonnect_core::uri::ParseOptions;

use crate::approval::{ApprovalBridge, Emitter, Withdrawn};
use crate::developer_mode_allowed;
use crate::error::{Error, Result};
use crate::origin;
use crate::relay::{RelayClient, RelayInfo};
use crate::requests::{self, AnswerInputs, Progress, RequestMeta};
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
/// "No such mailbox" answers in a row, a pause apart, before a connection counts as gone.
const GONE_STRIKES: u32 = 3;
/// Reads answered while one request waits for the user; beyond, 4029.
const MAX_READS_WHILE_WAITING: usize = 32;
/// Most messages held while a request waits for the user. Requests beyond it are refused
/// with 4029; anything else beyond it (pings) is dropped.
const MAX_DEFERRED: usize = 16;
/// Spec 6.1: an origin document is cached for no more than 24 hours.
const ORIGIN_RECHECK_S: u64 = 24 * 60 * 60;
/// After a failed origin fetch (the website down), the next try.
const ORIGIN_RETRY_S: u64 = 10 * 60;

/// What re-fetching the origin document said.
enum OriginCheck {
    /// Fetched, and it still lists the key this session was paired under (or the session
    /// predates keeping the key, and cannot be checked).
    Kept,
    /// Fetched and parsed, and the key is gone: the website revoked it.
    Removed,
    /// Not fetched (the website is down, or a disconnect cut it short).
    Unknown,
}

/// Answers one request. [`default_answerer`] in the app; the live tests put a stand-in
/// here, because the real one needs an open Sage wallet.
pub type Answerer = Arc<
    dyn Fn(
            AnswerInputs,
        ) -> Pin<Box<dyn Future<Output = std::result::Result<String, RpcError>> + Send>>
        + Send
        + Sync,
>;

/// [`requests::answer`]: the wallet's simulation, the approval UI, the signer and Sage.
pub fn default_answerer() -> Answerer {
    Arc::new(|inputs| Box::pin(requests::answer(inputs)))
}

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
    /// Read from the relay when the loop starts, so starting a loop never waits on the
    /// network (and never holds up `list_sessions`).
    pub info: RelayInfo,
    /// Set by `end_session`: the loop itself posts `session.end` and cleans up, so the
    /// end can never interleave with a message the loop is in the middle of sealing.
    pub end_requested: Arc<AtomicBool>,
    /// Set by `rotate_session_keys`: the loop offers new keys at its next pass.
    pub rotate_requested: Arc<AtomicBool>,
    /// Set by `set_dapp_policy` when the exposed keys changed: the loop sends the dApp a
    /// fresh `session.permissions` at its next pass.
    pub permissions_changed: Arc<AtomicBool>,
    /// Notified alongside either request, to cut a long poll short.
    pub interrupt: Arc<Notify>,
    /// Woken by a poll tick and by a push wake-up.
    pub wake: tokio::sync::mpsc::Receiver<()>,
    /// A background wake-up's request for one full read, and the report that it was done.
    pub drain: Arc<Drain>,
    /// The gateway and the phone's device token, for registering this session's wake-ups.
    pub push: Arc<crate::push::PushHandle>,
    /// Answers requests: [`default_answerer`].
    pub answer: Answerer,
}

/// One loop's side of a background wake-up (TASK-125.04): the wake-up sets `requested`, the
/// loop's next pass reads without waiting and counts it in `done`, and `passed` is notified
/// after every pass.
#[derive(Debug, Default)]
pub struct Drain {
    pub requested: AtomicBool,
    pub done: std::sync::atomic::AtomicU64,
    pub passed: Notify,
}

/// Pauses between posts of an answer that did not go out (doubling up to the cap).
const DELIVER_FIRST_RETRY: Duration = Duration::from_millis(500);
const DELIVER_MAX_RETRY: Duration = Duration::from_secs(5);
/// Longest an answer is posted again, whatever the request's own expiry.
const DELIVER_WINDOW_S: u64 = 120;

/// Pause after a failed push registration before the next try.
const PUSH_RETRY_S: u64 = 300;

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
        // Its secrets are erased: there is nothing to run.
        if stored.record.revoked {
            return;
        }
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
        let mut info_known = false;
        let mut first_pass = true;
        let mut gone = 0;
        // Persisted: a restart does not reset it, and a loop whose last check is older than a
        // day checks before reading anything (spec 6.1).
        let mut origin_checked_at = stored.record.origin_checked_at;
        let mut origin_retry_at = 0;
        let mut push_retry_at = 0;
        loop {
            if self.end_requested.load(Ordering::SeqCst) {
                self.end_now(&mut session, &mut stored, "ended by the user")
                    .await;
                self.cleanup(&session, fingerprint, &id).await;
                (self.emit)(EVENT_SESSION, json!({ "sessionId": id, "state": "ended" }));
                return;
            }
            // Asked for since the last pass: tell the dApp before reading on.
            if self.permissions_changed.swap(false, Ordering::SeqCst)
                && let Err(error) = self.announce_permissions(&mut session, &mut stored).await
            {
                tracing::warn!(session = %id, "could not send the new permissions: {error}");
            }
            // A website that withdrew its origin key gets nothing more answered, not even on
            // the first pass after a restart.
            let now_s = now();
            if now_s >= origin_retry_at
                && origin_checked_at.is_none_or(|at| now_s.saturating_sub(at) >= ORIGIN_RECHECK_S)
            {
                match self.origin_check(&stored).await {
                    OriginCheck::Kept => {
                        origin_checked_at = Some(now_s);
                        stored.record.origin_checked_at = Some(now_s);
                        if let Err(error) = self.store.save(&mut stored) {
                            tracing::warn!(session = %id, "could not store the origin check: {error}");
                        }
                    }
                    OriginCheck::Removed => {
                        self.revoke(&mut session, &mut stored).await;
                        return;
                    }
                    // The website is down, or the user is disconnecting: not a check. Again in
                    // a while, not at every pass.
                    OriginCheck::Unknown => origin_retry_at = now_s + ORIGIN_RETRY_S,
                }
                if self.end_requested.load(Ordering::SeqCst) {
                    continue;
                }
            }
            // Read first: what a website sends right after pairing, or while Klimper was in
            // the background, is answered before any housekeeping below. The first pass only
            // takes what is there, so the housekeeping is not held up by a long poll either.
            // A background wake-up asks for one full read: no long poll, and no wake-up can cut
            // it short (it would end the pass with nothing read, and iOS would suspend the app
            // before the request is handled).
            let draining = self.drain.requested.swap(false, Ordering::SeqCst);
            let wait = if first_pass || draining { 0 } else { POLL_WAIT };
            first_pass = false;
            let pumped = self.pump(&mut session, &mut stored, wait).await;
            if draining {
                self.drain.done.fetch_add(1, Ordering::SeqCst);
            }
            self.drain.passed.notify_waiters();
            if !matches!(pumped, Err(Error::PairingCodeUsed)) {
                gone = 0;
            }
            match pumped {
                Ok(Outcome::Continue) => {}
                Ok(Outcome::Ended) => {
                    self.cleanup(&session, fingerprint, &id).await;
                    // The website ended it: the user is told (TASK-125.05).
                    (self.emit)(
                        EVENT_SESSION,
                        json!({
                            "sessionId": id,
                            "state": "ended",
                            "by": "website",
                            "dapp": stored.record.dapp,
                        }),
                    );
                    return;
                }
                Err(Error::PairingCodeUsed) if gone + 1 < GONE_STRIKES => {
                    // Once is not enough to forget a connection (TASK-125.05): a relay being
                    // redeployed, or a proxy in front of it, can answer like that for a moment.
                    gone += 1;
                    tracing::info!(session = %id, "the relay does not know a mailbox ({gone})");
                    tokio::select! {
                        () = tokio::time::sleep(BACKOFF) => {}
                        () = self.interrupt.notified() => {}
                    }
                }
                Err(Error::PairingCodeUsed) => {
                    // The relay keeps saying a mailbox is gone: the dApp ended the session, or
                    // it expired. The full clean-up still runs: our own mailbox may be the one
                    // that is left, and a prompt may still be waiting for the user.
                    tracing::info!(session = %id, "the relay no longer has this mailbox");
                    self.cleanup(&session, fingerprint, &id).await;
                    (self.emit)(
                        EVENT_SESSION,
                        json!({
                            "sessionId": id,
                            "state": "ended",
                            "by": "website",
                            "dapp": stored.record.dapp,
                        }),
                    );
                    return;
                }
                Err(error) => {
                    tracing::warn!(session = %id, "xchonnect session loop: {error}");
                    tokio::select! {
                        () = tokio::time::sleep(BACKOFF) => {}
                        () = self.interrupt.notified() => {}
                    }
                }
            }
            if self.end_requested.load(Ordering::SeqCst) {
                continue;
            }
            // Housekeeping, after the read. An unreachable relay keeps the defaults, and a
            // disconnect does not wait for it.
            if !info_known {
                tokio::select! {
                    info = self.relay.info() => {
                        if let Ok(info) = info {
                            self.info = info;
                            info_known = true;
                        }
                    }
                    () = self.interrupt.notified() => {}
                }
            }
            let asked = self.rotate_requested.swap(false, Ordering::SeqCst);
            if (asked || session.needs_rotation(now())) && rotation_idle(&session) {
                if let Err(error) = self.begin_rotation(&mut session, &mut stored).await {
                    tracing::warn!(session = %id, "could not start a key rotation: {error}");
                }
            } else if asked {
                tracing::info!(session = %id, "a key rotation is already in progress");
            }
            self.ensure_push(&session, &mut stored, &mut push_retry_at)
                .await;
            // The checks above may have taken the disconnect's notification: look at the flag
            // itself before waiting.
            if self.end_requested.load(Ordering::SeqCst) {
                continue;
            }
            // Wait for the next tick or wake-up. A closed channel means the app is going
            // away.
            if self.wake.recv().await.is_none() {
                return;
            }
        }
    }

    /// Register this session's wake-ups with the relay when that is due: the phone has a
    /// device token, a gateway is configured, and the session has no registration yet, has
    /// moved to a new mailbox, is within 30 days of expiry or was registered under another
    /// device token. Never fatal: the loop polls whether or not push works, and a refusal
    /// (a relay that has not allowlisted our gateway) is tried again in five minutes.
    async fn ensure_push(&self, session: &Session, stored: &mut StoredSession, retry_at: &mut u64) {
        let (Some(gateway), Some(device_token)) = (self.push.gateway(), self.push.device_token())
        else {
            return;
        };
        let (mailbox, now) = (session.own_mailbox(), now());
        let (mailbox_b64, tag) = (
            mailbox.to_b64(),
            crate::push::token_tag(device_token.kind, &device_token.token),
        );
        let current = stored
            .record
            .push
            .as_ref()
            .is_some_and(|reg| !reg.due(&mailbox_b64, &tag, now));
        if current || now < *retry_at {
            return;
        }
        let sealed = match crate::push::seal(
            &mut OsEntropy,
            gateway,
            &device_token,
            cfg!(debug_assertions),
            now,
        ) {
            Ok(sealed) => sealed,
            Err(error) => {
                tracing::warn!("could not seal a push token: {error}");
                *retry_at = now + PUSH_RETRY_S;
                return;
            }
        };
        let read = session.own_read_token().clone();
        if let Err(error) = self.relay.set_push(mailbox, &read, sealed.body).await {
            tracing::warn!(session = %stored.record.id, "could not register for push: {error}");
            *retry_at = now + PUSH_RETRY_S;
            return;
        }
        let mut registration = sealed.registration;
        registration.mailbox = mailbox_b64;
        stored.record.push = Some(registration);
        if let Err(error) = self.store.save(stored) {
            tracing::warn!("could not store a push registration: {error}");
        }
    }

    /// Tell the dApp the user ended the connection. Seal, persist, post — in that order,
    /// as everywhere else.
    async fn end_now(&self, session: &mut Session, stored: &mut StoredSession, reason: &str) {
        match session.end(&mut OsEntropy, now(), Some(reason.to_owned())) {
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
        delete_mailboxes(&self.relay, session).await;
        if let Err(error) = self.store.remove(fingerprint, id) {
            tracing::warn!("could not remove a session from the keyring: {error}");
        }
    }

    /// Spec 6.1: re-fetch the origin document and look for the key the pairing code was
    /// signed with. Only a document that was fetched and parsed and no longer lists the key
    /// counts as a removal: a website that is down, slow or serving garbage must not be
    /// able to end its users' connections by accident, and a session paired before the
    /// `kid` was kept cannot be checked at all.
    ///
    /// Only a disconnect cuts the fetch short: a wake-up (push, coming to the front) must not
    /// turn into "checked" without a check.
    async fn origin_check(&self, stored: &StoredSession) -> OriginCheck {
        let Some(kid) = stored.record.origin_kid.as_deref() else {
            return OriginCheck::Kept;
        };
        let options = ParseOptions {
            developer_mode: stored.record.developer_mode && developer_mode_allowed(),
        };
        let fetch = origin::fetch(&stored.record.dapp, options);
        tokio::pin!(fetch);
        let document = loop {
            tokio::select! {
                document = &mut fetch => break document,
                () = self.interrupt.notified() => {
                    if self.end_requested.load(Ordering::SeqCst) {
                        return OriginCheck::Unknown;
                    }
                }
            }
        };
        match document {
            Ok(document) if document.keys.iter().any(|key| key.kid == kid) => OriginCheck::Kept,
            Ok(_) => OriginCheck::Removed,
            Err(error) => {
                tracing::debug!(dapp = %stored.record.dapp, "origin document not checked: {error}");
                OriginCheck::Unknown
            }
        }
    }

    /// The dApp withdrew the origin key this session was paired under: end it, erase its
    /// secrets, and keep the record marked so the user sees why it is gone.
    async fn revoke(&self, session: &mut Session, stored: &mut StoredSession) {
        let id = stored.record.id.clone();
        tracing::warn!(dapp = %stored.record.dapp, "origin key removed; revoking the session");
        self.approvals.withdraw_session(&id);
        self.end_now(session, stored, "origin key revoked").await;
        delete_mailboxes(&self.relay, session).await;
        stored.record.revoked = true;
        stored.session_hex = String::new();
        if let Err(error) = self.store.save(stored) {
            tracing::warn!("could not mark a revoked session: {error}");
        }
        (self.emit)(
            EVENT_SESSION,
            json!({ "sessionId": id, "state": "revoked", "dapp": stored.record.dapp }),
        );
    }

    /// Tell the dApp what it may see now (`session.permissions`, spec 9.3), from the record
    /// as just reloaded, after the user changed which keys it sees.
    async fn announce_permissions(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
    ) -> Result<()> {
        if let Ok(fresh) = self
            .store
            .load(stored.record.fingerprint, &stored.record.id)
        {
            stored.record.permissions = fresh.record.permissions;
            stored.record.own_key = fresh.record.own_key;
        }
        let message = Message::SessionPermissions(stored.record.permissions.to_kit().to_message());
        let outgoing = session
            .seal(&mut OsEntropy, now(), message, 3600)
            .map_err(Error::protocol)?;
        stored.session_hex = encode(session)?;
        self.store.save(stored)?;
        self.relay.post_retrying(&outgoing).await
    }

    /// Offer new keys (spec 9.2.1, this side as initiator): a fresh mailbox, then
    /// `session.rotate` offer under the current epoch. The switch happens when the dApp's
    /// accept arrives, in [`SessionLoop::dispatch`].
    async fn begin_rotation(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
    ) -> Result<()> {
        let read = Token::random(&mut OsEntropy);
        let write = Token::random(&mut OsEntropy);
        let mailbox = self.relay.create_mailbox(&read, &write, None).await?;
        let outgoing =
            match session.begin_rotation(&mut OsEntropy, now(), mailbox, read.clone(), write) {
                Ok(outgoing) => outgoing,
                Err(error) => {
                    self.relay.delete(mailbox, &read).await;
                    return Err(Error::protocol(error));
                }
            };
        stored.session_hex = encode(session)?;
        self.store.save(stored)?;
        self.relay.post_retrying(&outgoing).await?;
        (self.emit)(
            EVENT_SESSION,
            json!({ "sessionId": stored.record.id, "state": "rotating", "epoch": session.epoch() }),
        );
        Ok(())
    }

    /// One pass over the mailboxes.
    async fn pump(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
        wait: u64,
    ) -> Result<Outcome> {
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
        // The only await raced against a disconnect: nothing has changed yet, so dropping
        // the poll loses nothing, and the relay keeps any message it was about to deliver. A
        // read that does not wait is not raced: it returns at once anyway.
        let messages = if wait == 0 {
            self.relay.fetch(mailbox, &read, 0, &self.info).await?
        } else {
            tokio::select! {
                messages = self.relay.fetch(mailbox, &read, wait, &self.info) => messages?,
                () = self.interrupt.notified() => return Ok(Outcome::Continue),
            }
        };
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
            // The user disconnected: answer nothing more. What is left unacknowledged goes
            // with the mailbox.
            if self.end_requested.load(Ordering::SeqCst) {
                return Ok(Outcome::Continue);
            }
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
            let mut queue = VecDeque::from([inner]);
            let mut ended = false;
            // Messages that arrived while a request waited for the user come next, before
            // the rest of this batch (which the wait already read; those are now replays).
            while let Some(inner) = queue.pop_front() {
                let (done, deferred) = self.dispatch(session, stored, mailbox, read, inner).await?;
                queue.extend(deferred);
                if done {
                    ended = true;
                    break;
                }
            }
            // Acknowledge only now: processed and persisted.
            self.relay.ack(mailbox, read, &ids).await?;
            if ended {
                return Ok(Outcome::Ended);
            }
        }
        Ok(Outcome::Continue)
    }

    /// Handle one decoded message. Returns `true` when the session has ended, and the
    /// messages that arrived while a request waited for the user, to handle next.
    async fn dispatch(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
        mailbox: MailboxId,
        read: &Token,
        inner: Inner,
    ) -> Result<(bool, Vec<Inner>)> {
        match inner.message {
            Message::RpcRequest { method, params } => {
                let deferred = self
                    .handle_request(session, stored, inner.id, inner.exp, &method, params)
                    .await?;
                return Ok((false, deferred));
            }
            Message::SessionRotate(rotate) if rotate.phase == RotatePhase::Offer => {
                self.accept_rotation(session, stored, &rotate).await?;
            }
            Message::SessionRotate(_) => {
                // The dApp accepted our offer and `Session::open` has switched epochs. A
                // first message on the dApp's new mailbox is what lets it retire its old
                // one (spec 9.2.1 step 4); `send` persists the new state before posting.
                self.send(session, stored, Message::SessionPing, 300)
                    .await?;
                (self.emit)(
                    EVENT_SESSION,
                    json!({
                        "sessionId": stored.record.id,
                        "state": "rotated",
                        "epoch": session.epoch(),
                    }),
                );
            }
            Message::SessionPing => {
                self.send(session, stored, Message::SessionPong, 300)
                    .await?;
            }
            Message::SessionEnd { .. } => {
                // The reason is the dApp's own text: not logged.
                tracing::info!(dapp = %stored.record.dapp, "the dApp ended the xchonnect session");
                let _ = (mailbox, read);
                return Ok((true, Vec::new()));
            }
            other => {
                // `session.permissions`, a type this version does not know, a stray pong, an
                // `rpc.cancel` for a request that was already answered.
                tracing::debug!("xchonnect message {} needs no answer", other.type_name());
                stored.session_hex = encode(session)?;
                self.store.save(stored)?;
            }
        }
        Ok((false, Vec::new()))
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
    ) -> Result<Vec<Inner>> {
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
        // receipt, and skipping it saves a relay round trip before the answer. Best-effort:
        // the request is opened and the session persisted already, so a failed post must not
        // end this pass, or the request would come back as a replay and never be answered.
        if !answers_without_prompt(method)
            && let Err(error) = self
                .send(session, stored, Message::RpcReceived { request_id }, 3600)
                .await
        {
            tracing::info!(session = %stored.record.id, "could not send rpc.received: {error}");
        }

        let request_b64 = b64::encode(&request_id);
        let (progress, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
        let answer = (self.answer)(self.inputs(
            stored,
            request_id,
            request_exp,
            method,
            params,
            Some(progress),
            now_s,
        ));
        tokio::pin!(answer);

        let mailbox = session.own_mailbox();
        let read = session.own_read_token().clone();
        let mut deferred: Vec<Inner> = Vec::new();
        let mut reads = 0;
        let mut polling = true;
        let outcome = loop {
            tokio::select! {
                biased;
                outcome = &mut answer => break outcome,
                Some(step) = progress_rx.recv(), if !session.is_ended() => {
                    self.report(session, stored, request_id, step).await;
                }
                fetched = self.relay.fetch(mailbox, &read, POLL_WAIT, &self.info), if polling => {
                    let read_all = match fetched {
                        Ok(messages) => {
                            self.read_while_waiting(
                                session,
                                stored,
                                mailbox,
                                &read,
                                request_id,
                                messages,
                                &mut deferred,
                                &mut reads,
                            )
                            .await
                        }
                        Err(error) => Err(error),
                    };
                    if let Err(error) = read_all {
                        // Never abandon a request the user may be approving: stop reading,
                        // keep waiting, answer it. Only cancellation is lost.
                        tracing::info!(
                            session = %stored.record.id,
                            "stopped reading while a request waits: {error}"
                        );
                        polling = false;
                    }
                }
                () = self.interrupt.notified() => {
                    if self.end_requested.load(Ordering::SeqCst) {
                        // The user disconnected: the prompt goes, the loop ends after this.
                        self.approvals.withdraw_session(&stored.record.id);
                    }
                }
            }
        };
        let withdrawn = self
            .approvals
            .take_withdrawn(&stored.record.id, &request_b64);
        // The dApp ended the session while the user decided: nobody to answer. The
        // deferred `session.end` ends the loop next.
        if session.is_ended() {
            return Ok(deferred);
        }
        // Progress reported just before the answer goes out first, so the dApp sees it in
        // order.
        while let Ok(step) = progress_rx.try_recv() {
            self.report(session, stored, request_id, step).await;
        }

        let message = match (outcome, withdrawn) {
            // Already signed (and maybe broadcast) when the cancel came: the answer stands.
            (Ok(result), _) => rpc::result(request_id, &result).map_err(Error::protocol)?,
            (Err(_), Some(by)) => {
                // Informative: the 4102 that follows is what settles the request.
                if by == Withdrawn::ByUser
                    && let Err(error) = self
                        .send(session, stored, Message::RpcCancel { request_id }, 3600)
                        .await
                {
                    tracing::info!(
                        session = %stored.record.id,
                        "could not send rpc.cancel: {error}"
                    );
                }
                cancelled(request_id)?
            }
            (
                Err(RpcError {
                    code,
                    message,
                    data,
                }),
                None,
            ) => {
                rpc::error(request_id, code, &message, data.as_deref()).map_err(Error::protocol)?
            }
        };
        // The one message that must arrive: the transaction may already be on its way, and
        // a dApp left without the answer could ask again.
        self.deliver(session, stored, message, request_exp).await?;
        Ok(deferred)
    }

    /// What [`Answerer`] gets for one request.
    #[allow(clippy::too_many_arguments)]
    fn inputs(
        &self,
        stored: &StoredSession,
        request_id: [u8; 16],
        request_exp: u64,
        method: &str,
        params: String,
        progress: Option<tokio::sync::mpsc::UnboundedSender<Progress>>,
        now_s: u64,
    ) -> AnswerInputs {
        AnswerInputs {
            state: self.state.clone(),
            store: self.store.clone(),
            approvals: self.approvals.clone(),
            record: stored.record.clone(),
            meta: RequestMeta {
                session_id: stored.record.id.clone(),
                dapp: stored.record.dapp.clone(),
                method: method.to_owned(),
                request_id: b64::encode(&request_id),
                expires_at: requests::prompt_deadline(now_s, request_exp),
                progress,
            },
            params,
            now: now_s,
        }
    }

    /// Answer a read that never prompts at once, while another request waits for the user
    /// (TASK-125.03). A failed post is logged: the waiting request must not be abandoned.
    async fn answer_read(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
        inner: Inner,
    ) -> Result<()> {
        let Message::RpcRequest { method, params } = inner.message else {
            return Ok(());
        };
        let outcome =
            (self.answer)(self.inputs(stored, inner.id, inner.exp, &method, params, None, now()))
                .await;
        let message = match outcome {
            Ok(result) => rpc::result(inner.id, &result),
            Err(RpcError {
                code,
                message,
                data,
            }) => rpc::error(inner.id, code, &message, data.as_deref()),
        };
        // An answer that cannot be built is logged: the request still waiting must keep its
        // cancel handling, which a returned error would stop.
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                tracing::info!(session = %stored.record.id, "could not build an answer: {error}");
                return Ok(());
            }
        };
        if session.is_ended() {
            return Ok(());
        }
        if let Err(error) = self.deliver(session, stored, message, inner.exp).await {
            tracing::info!(session = %stored.record.id, "could not answer a read: {error}");
        }
        Ok(())
    }

    /// Seal and persist an answer, then post it until it arrives or the request it answers
    /// has expired (the dApp has given up on it by then). Only a gone mailbox stops it early.
    async fn deliver(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
        message: Message,
        request_exp: u64,
    ) -> Result<()> {
        let outgoing = session
            .seal(&mut OsEntropy, now(), message, 3600)
            .map_err(Error::protocol)?;
        stored.session_hex = encode(session)?;
        self.store.save(stored)?;
        // Never longer than DELIVER_WINDOW_S: `exp` is the website's to choose (up to 7 days),
        // and a loop posting one answer again reads nothing else.
        let until = request_exp.min(now() + DELIVER_WINDOW_S);
        let mut delay = DELIVER_FIRST_RETRY;
        let mut gone = 0;
        loop {
            match self.relay.post(&outgoing).await {
                Ok(()) => return Ok(()),
                // A gone mailbox counts like everywhere else: three times, not once.
                Err(error @ Error::PairingCodeUsed) if gone + 1 >= GONE_STRIKES => {
                    return Err(error);
                }
                Err(error)
                    if !crate::relay::worth_retrying(&error)
                        || now() >= until
                        || self.end_requested.load(Ordering::SeqCst) =>
                {
                    tracing::warn!(session = %stored.record.id, "gave up on an answer: {error}");
                    return Ok(());
                }
                Err(Error::PairingCodeUsed) => {
                    gone += 1;
                    tokio::select! {
                        () = tokio::time::sleep(BACKOFF) => {}
                        () = self.interrupt.notified() => {}
                    }
                }
                Err(error) => {
                    tracing::info!(session = %stored.record.id, "answer not posted yet: {error}");
                    tokio::select! {
                        () = tokio::time::sleep(delay) => {}
                        () = self.interrupt.notified() => {}
                    }
                    delay = (delay * 2).min(DELIVER_MAX_RETRY);
                }
            }
        }
    }

    /// `rpc.status` is informative: a post that fails is logged and never ends the wait,
    /// so it cannot abandon a request the user is deciding or has approved.
    async fn report(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
        request_id: [u8; 16],
        step: Progress,
    ) {
        if let Err(error) = self
            .send(session, stored, status(request_id, step), 3600)
            .await
        {
            tracing::info!(session = %stored.record.id, "could not report progress: {error}");
        }
    }

    /// Messages read while a request waits for the user. An `rpc.cancel` for it withdraws
    /// it; one for a request that has not started yet answers that request 4102 at once;
    /// the rest is kept for after the answer. Each is acknowledged as soon as it is read,
    /// since it was opened (and the replay window moved) and is now held here.
    #[allow(clippy::too_many_arguments)]
    async fn read_while_waiting(
        &self,
        session: &mut Session,
        stored: &mut StoredSession,
        mailbox: MailboxId,
        read: &Token,
        current: [u8; 16],
        messages: Vec<(String, Vec<u8>)>,
        deferred: &mut Vec<Inner>,
        reads: &mut usize,
    ) -> Result<()> {
        for (msg_id, envelope) in messages {
            let ids = [msg_id];
            let opened = if envelope.is_empty() {
                None
            } else {
                // A replay is expected here: the request itself, and the rest of the batch
                // it came in, are still unacknowledged.
                session.open(now(), &mailbox, &envelope).ok()
            };
            stored.session_hex = encode(session)?;
            self.store.save(stored)?;
            self.relay.ack(mailbox, read, &ids).await?;
            let Some(inner) = opened else { continue };
            match inner.message {
                Message::RpcCancel { request_id } if request_id == current => {
                    self.approvals.withdraw_request(
                        &stored.record.id,
                        &b64::encode(&request_id),
                        Withdrawn::ByDapp,
                    );
                }
                Message::RpcCancel { request_id } => {
                    let queued = deferred.iter().position(|d| {
                        d.id == request_id && matches!(d.message, Message::RpcRequest { .. })
                    });
                    if let Some(index) = queued
                        && !session.is_ended()
                    {
                        deferred.remove(index);
                        self.send(session, stored, cancelled(request_id)?, 3600)
                            .await?;
                    }
                }
                Message::SessionEnd { .. } => {
                    // The dApp is gone: close its prompt now, end after the answer.
                    self.approvals.withdraw_session(&stored.record.id);
                    deferred.push(inner);
                }
                // A read never waits behind a prompt: the website's wallet info arrives while
                // the user is still deciding (TASK-125.03).
                // A budget per wait, so a website cannot keep the loop busy with reads while a
                // prompt is open.
                Message::RpcRequest { ref method, .. }
                    if answers_without_prompt(method) && !session.is_ended() =>
                {
                    if *reads < MAX_READS_WHILE_WAITING {
                        *reads += 1;
                        self.answer_read(session, stored, inner).await?;
                    } else {
                        let busy = rpc::error(
                            inner.id,
                            rpc::codes::LIMIT_EXCEEDED,
                            "too many requests waiting",
                            None,
                        )
                        .map_err(Error::protocol)?;
                        self.send(session, stored, busy, 3600).await?;
                    }
                }
                // A request beyond the cap is refused at once rather than queued as yet
                // another prompt: a dApp cannot line up prompts faster than the user answers.
                Message::RpcRequest { .. } if deferred.len() >= MAX_DEFERRED => {
                    if !session.is_ended() {
                        let busy = rpc::error(
                            inner.id,
                            rpc::codes::LIMIT_EXCEEDED,
                            "too many requests waiting",
                            None,
                        )
                        .map_err(Error::protocol)?;
                        self.send(session, stored, busy, 3600).await?;
                    }
                }
                _ if deferred.len() >= MAX_DEFERRED => {}
                _ => deferred.push(inner),
            }
        }
        Ok(())
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

/// `rpc.status` for one step.
fn status(request_id: [u8; 16], step: Progress) -> Message {
    let (state, tx_id) = match step {
        Progress::Shown => ("shown", None),
        Progress::Approved => ("approved", None),
        Progress::Broadcast(id) => ("broadcast", Some(id)),
    };
    Message::RpcStatus {
        request_id,
        state: state.to_owned(),
        tx_id,
    }
}

/// The answer to a withdrawn request (xchonnect spec 9.1).
fn cancelled(request_id: [u8; 16]) -> Result<Message> {
    rpc::error(
        request_id,
        rpc::codes::REQUEST_CANCELLED,
        "request cancelled",
        None,
    )
    .map_err(Error::protocol)
}

/// Methods wallet-kit answers without asking the user (spec 9.1: `connect` is answered
/// without a prompt; `chainId` and the exposed keys are read straight from the session; the
/// balance and coin reads from the wallet database, scoped to the exposed keys).
fn answers_without_prompt(method: &str) -> bool {
    matches!(
        rpc::canonical_method(method),
        "chainId"
            | "connect"
            | "getPublicKeys"
            | "getAssetBalance"
            | "getAssetCoins"
            | "filterUnlockedCoins"
    )
}

/// Post `session.end`. A disconnect is the one message the user explicitly asked to reach
/// the dApp, so a relay hiccup must not leave the website believing it is still paired.
async fn post_end(relay: &RelayClient, outgoing: &xchonnect_core::session::Outgoing) {
    if let Err(error) = relay.post_retrying(outgoing).await {
        tracing::warn!("could not tell the dApp the session ended: {error}");
    }
}

/// End a session whose loop is not running (no wallet was open when it started, the
/// loop stopped, the session needs re-pairing). The website is told all the same:
/// `session.end` is sealed from the stored state and posted, and this side's mailboxes are
/// deleted, so even a website that missed the message finds the session gone on its next
/// request instead of waiting on a mailbox nobody reads. Only a session whose stored state
/// cannot be read at all can end locally alone.
pub async fn end_without_loop(store: &SessionStore, stored: &mut StoredSession) {
    let developer_mode = stored.record.developer_mode && developer_mode_allowed();
    let Ok(relay) = RelayClient::new(&stored.record.relay, developer_mode) else {
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
    delete_mailboxes(&relay, &session).await;
}

/// No rotation of ours is waiting for an accept and no previous epoch is still draining.
fn rotation_idle(session: &Session) -> bool {
    session.pending_rotation_mailbox().is_none() && session.draining_mailbox().is_none()
}

/// Delete every mailbox this side holds for the session. Failures are logged by
/// [`RelayClient::delete`]; the relay drops an unused mailbox after 30 days anyway.
async fn delete_mailboxes(relay: &RelayClient, session: &Session) {
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
    fn only_reads_skip_the_receipt_and_never_wait_behind_a_prompt() {
        for method in [
            "chainId",
            "connect",
            "getPublicKeys",
            "chip0002_getPublicKeys",
            "getAssetBalance",
            "getAssetCoins",
            "filterUnlockedCoins",
        ] {
            assert!(answers_without_prompt(method), "{method}");
        }
        for method in [
            "signCoinSpends",
            "chip0002_signCoinSpends",
            "signMessage",
            "sendTransaction",
            "chia_send",
            "chia_takeOffer",
            "xchonnect_submitCoinSpends",
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
            origin_kid: None,
            revoked: false,
            push: None,
            origin_checked_at: None,
            methods_revision: 0,
            own_key: None,
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
