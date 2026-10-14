// SPDX-License-Identifier: AGPL-3.0-or-later
//! Klimper ↔ Xchonnect: connect the wallet to a Chia dApp as a remote signing target
//! (clapandpay decision-008, TASK-071 to TASK-079).
//!
//! The protocol and every safety decision live in two Apache-2.0 crates,
//! `xchonnect-core` (URIs, origin verification, pairing, sessions, envelopes) and
//! `xchonnect-wallet-kit` (local simulation, signature policy, permissions, limits,
//! binding checks, CHIP-0002 handlers). This plugin supplies only what a host has to:
//! HTTP to the relay and to the dApp's origin document, session storage in the OS
//! keyring, the approval UI, and signing through Sage's own key storage. No Sage engine
//! file is changed; the wiring points are listed in `klimper/docs/UPSTREAM.md`.
//!
//! ## Command and event surface
//!
//! Commands are `plugin:klimper-xchonnect|<name>`:
//!
//! | Command | Arguments | Returns |
//! |---|---|---|
//! | `status` | — | [`Status`] |
//! | `inspect_pairing_uri` | `uri: string` | [`PairingPreview`] |
//! | `approve_pairing` | `pairingId: string` | [`PairingSas`] |
//! | `cancel_pairing` | `pairingId: string` | `null` |
//! | `confirm_sas` | `pairingId: string`, `matches: boolean` | [`SessionSummary`] or `null` |
//! | `list_sessions` | — | [`SessionSummary`]`[]` (and resumes their message loops) |
//! | `end_session` | `sessionId: string` | `null` |
//! | `rotate_session_keys` | `sessionId: string` | `null` |
//! | `set_dapp_policy` | `sessionId: string`, `policy:` [`DappPolicy`] | [`SessionSummary`] |
//! | `pending_approvals` | — | [`Approval`]`[]` |
//! | `resolve_approval` | `approvalId: string`, `approved: boolean` | `null` |
//! | `cancel_approval` | `approvalId: string` | `null` (the dApp gets `rpc.cancel` and 4102) |
//!
//! Events:
//!
//! | Event | Payload |
//! |---|---|
//! | `klimper-xchonnect://pairing` | `{ pairingId, state, sas?, dappConfirmed?, error? }` |
//! | `klimper-xchonnect://sessions` | `{}` — the list changed, call `list_sessions` |
//! | `klimper-xchonnect://session` | `{ sessionId, state, epoch?, dapp? }` |
//! | `klimper-xchonnect://request` | `{ sessionId, dapp, method }` |
//! | `klimper-xchonnect://approval` | [`Approval`] — something is waiting for the user |
//! | `klimper-xchonnect://approval-resolved` | `{ id, approved }` |

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{
    AppHandle, Manager, Runtime, State, command,
    plugin::{Builder, TauriPlugin},
};
use tokio::sync::Mutex;
use xchonnect_core::domain::display_domain;

pub mod approval;
pub mod chain;
pub mod error;
pub mod execute;
pub mod native_push;
pub mod origin;
pub mod pairing;
pub mod push;
pub mod relay;
pub mod requests;
pub mod session_loop;
pub mod sign_guard;
pub mod store;
mod testing;
pub mod wallet;

use approval::{Approval, ApprovalBridge, Emitter};

/// The webview the Klimper capability grants this plugin to (`capabilities/klimper.json`).
const MAIN_WEBVIEW: &str = "main";
use error::{Error, Result};
use pairing::{Attempt, EVENT_PAIRING, EVENT_SESSIONS, PairingPreview, PairingSas};
use session_loop::SessionLoop;
use store::{OsKeyring, SessionStore, StoredLimit, StoredSession};
use wallet::AppState;

/// Plain-HTTP loopback relays and `localhost` origin documents are a development
/// affordance. A release build refuses them outright (clapandpay TASK-074 AC #5).
pub const fn developer_mode_allowed() -> bool {
    cfg!(debug_assertions)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A session loop's control handles.
///
/// The entry outlives the loop: a loop that returned (the dApp ended the session, the state
/// needed re-pairing) leaves it behind, and the ticker keeps its own `wake` sender, so
/// removing the entry does not stop a running loop either. [`Live::running`] is therefore
/// the only test of whether a loop is there to talk to.
struct Live {
    end_requested: Arc<AtomicBool>,
    rotate_requested: Arc<AtomicBool>,
    permissions_changed: Arc<AtomicBool>,
    interrupt: Arc<tokio::sync::Notify>,
    wake: tokio::sync::mpsc::Sender<()>,
    /// Background wake-ups: ask for a full read, and hear when it was done.
    drain: Arc<session_loop::Drain>,
}

impl Live {
    /// The loop still holds its receiver.
    fn running(&self) -> bool {
        !self.wake.is_closed()
    }

    /// The user disconnected and the loop is posting `session.end`: no longer listed.
    fn ending(&self) -> bool {
        self.running() && self.end_requested.load(Ordering::SeqCst)
    }

    fn request_rotation(&self) {
        self.rotate_requested.store(true, Ordering::SeqCst);
        self.interrupt.notify_one();
        let _ = self.wake.try_send(());
    }

    /// The exposed keys changed: the loop tells the dApp at its next pass.
    fn announce_permissions(&self) {
        self.permissions_changed.store(true, Ordering::SeqCst);
        self.interrupt.notify_one();
        let _ = self.wake.try_send(());
    }

    fn request_end(&self) {
        self.end_requested.store(true, Ordering::SeqCst);
        // `notify_one` keeps a permit when the loop is not waiting yet, so the end is
        // seen by the next poll rather than lost.
        self.interrupt.notify_one();
        let _ = self.wake.try_send(());
    }
}

/// Longest wait for the loops after a background wake-up, inside the time iOS allows.
const BACKGROUND_DRAIN: std::time::Duration = std::time::Duration::from_secs(16);
/// How long a wake-up that launched the app waits for the first loops to start.
const LOOPS_START_WAIT: std::time::Duration = std::time::Duration::from_secs(8);

/// A pairing attempt, locked on its own: a poll for `session.confirm` on one attempt never
/// holds up cancelling it, confirming it, or starting another.
type SharedAttempt = Arc<Mutex<Attempt>>;

/// Plugin state.
pub struct Xchonnect<R: Runtime> {
    app: AppHandle<R>,
    store: Arc<SessionStore>,
    approvals: Arc<ApprovalBridge>,
    emit: Emitter,
    attempts: Mutex<BTreeMap<String, SharedAttempt>>,
    live: Mutex<BTreeMap<String, Live>>,
    push: Arc<push::PushHandle>,
    native: native_push::NativePush<R>,
}

impl<R: Runtime> std::fmt::Debug for Xchonnect<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Xchonnect").finish_non_exhaustive()
    }
}

/// What the Xchonnect surface can do right now.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Developer-mode pairing codes are accepted (debug builds only).
    pub developer_mode_allowed: bool,
    /// Active wallet, if one is open.
    pub fingerprint: Option<u32>,
    /// Chain the active wallet is on, as CHIP-0002 `chainId`.
    pub chain_id: Option<String>,
    /// Connected dApps for the active wallet.
    pub session_count: usize,
    /// OS keyring service the sessions are stored under. Shown in settings so the
    /// separation from the signing keychain is visible rather than asserted.
    pub keyring_service: String,
}

/// One connected dApp.
///
/// The flags stay flags: this crosses into the webview as JSON and each one answers a
/// separate question the screen asks.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    /// Verified domain, ASCII (A-label) form.
    pub dapp: String,
    /// Verified domain as the user reads it.
    pub domain_unicode: String,
    /// `true` when the two forms differ: show both.
    pub domain_differs: bool,
    /// Homograph warnings for the domain.
    pub warnings: Vec<String>,
    /// Name the origin document claims. Untrusted text.
    pub dapp_name: String,
    pub chain_id: String,
    pub created_at: u64,
    /// Current key epoch; it grows when the dApp rotates.
    pub epoch: u64,
    pub sas_confirmed: bool,
    /// Stored state was lost or rolled back: the dApp must be paired again.
    pub needs_repair: bool,
    /// The website withdrew the origin key this connection was paired under, so Klimper
    /// ended it (spec 6.1). Only removing it is left.
    pub revoked: bool,
    /// New keys were offered and the dApp has not answered yet, or the previous epoch's
    /// mailbox is still being read.
    pub rotating: bool,
    pub developer_mode: bool,
    pub methods: Vec<String>,
    pub exposed_keys: Vec<String>,
    pub limits: BTreeMap<String, LimitView>,
    pub allow_agg_sig_unsafe: bool,
    pub allow_unknown_contracts: bool,
    /// The website sees every receive address, not only its own (the user allowed it, or it
    /// was paired before websites had an address of their own).
    pub all_receive_keys: bool,
    /// The message loop is running for this session.
    pub connected: bool,
}

/// One limit as the webview reads and writes it (`perRequest`, `perDay`, in the asset's
/// smallest unit; absent or `null` is no limit).
///
/// The stored form, [`StoredLimit`], keeps its `snake_case` field names: records in the
/// keyring are written with them. Before this type the webview's `perRequest` / `perDay`
/// were read as `StoredLimit`, whose unknown fields serde skips, so every limit the user
/// set was stored as no limit. A field this does not know is now an error, never a missing
/// limit.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LimitView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_request: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_day: Option<u128>,
}

impl From<StoredLimit> for LimitView {
    fn from(limit: StoredLimit) -> Self {
        LimitView {
            per_request: limit.per_request,
            per_day: limit.per_day,
        }
    }
}

impl From<LimitView> for StoredLimit {
    fn from(limit: LimitView) -> Self {
        StoredLimit {
            per_request: limit.per_request,
            per_day: limit.per_day,
        }
    }
}

/// The stored limits in the webview's words.
pub fn limit_views(limits: &BTreeMap<String, StoredLimit>) -> BTreeMap<String, LimitView> {
    limits
        .iter()
        .map(|(key, limit)| (key.clone(), LimitView::from(*limit)))
        .collect()
}

/// Changes a user can make to one dApp's permissions (the full screen is TASK-083).
///
/// Read strictly: a field the webview names differently from this is refused, so a naming
/// mismatch fails the save instead of silently loosening a permission.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DappPolicy {
    /// Replace the per-asset limits. Keys are `xch` or `cat:<64 hex>`.
    #[serde(default)]
    pub limits: Option<BTreeMap<String, LimitView>>,
    /// Allow `AGG_SIG_UNSAFE` for this dApp. Off by default and a scary confirmation in
    /// the UI: such a signature is valid in any spend context.
    #[serde(default)]
    pub allow_agg_sig_unsafe: Option<bool>,
    /// Allow spends of puzzles the wallet does not recognise.
    #[serde(default)]
    pub allow_unknown_contracts: Option<bool>,
    /// Restrict the allowed CHIP-0002 methods.
    #[serde(default)]
    pub methods: Option<Vec<String>>,
    /// Let the website see every receive address (it can then build spends from all the
    /// user's coins, and link them), or only its own. On by default on the pairing screen.
    #[serde(default)]
    pub all_receive_keys: Option<bool>,
}

impl DappPolicy {
    /// The limits to store, if this change replaces them.
    pub fn stored_limits(&self) -> Option<BTreeMap<String, StoredLimit>> {
        self.limits.as_ref().map(|limits| {
            limits
                .iter()
                .map(|(key, limit)| (key.clone(), StoredLimit::from(*limit)))
                .collect()
        })
    }
}

/// Describe one stored session for the webview.
fn summary(stored: &StoredSession, keys: KeyState, connected: bool) -> SessionSummary {
    let shown = display_domain(&stored.record.dapp);
    SessionSummary {
        session_id: stored.record.id.clone(),
        domain_differs: shown.unicode != shown.ascii,
        domain_unicode: shown.unicode,
        warnings: shown
            .warnings
            .into_iter()
            .map(|w| format!("{w:?}"))
            .collect(),
        dapp: stored.record.dapp.clone(),
        dapp_name: stored.record.dapp_name.clone(),
        chain_id: stored.record.chain_id.clone(),
        created_at: stored.record.created_at,
        epoch: keys.epoch,
        sas_confirmed: stored.record.sas_confirmed,
        needs_repair: stored.record.needs_repair,
        revoked: stored.record.revoked,
        rotating: keys.rotating,
        developer_mode: stored.record.developer_mode,
        methods: stored.record.permissions.methods.clone(),
        exposed_keys: stored.record.permissions.exposed_keys.clone(),
        limits: limit_views(&stored.record.permissions.limits),
        allow_agg_sig_unsafe: stored.record.allow_agg_sig_unsafe,
        allow_unknown_contracts: stored.record.allow_unknown_contracts,
        all_receive_keys: stored.record.permissions.exposed_keys.len() > 1,
        connected,
    }
}

/// What a website sees: every receive key when the user allowed that, the wallet's first
/// receive address first, and its own key last; or only its own key. Websites take the first
/// key as "the wallet" (its address, its fingerprint), so that must be the address the user
/// knows, not an empty key made for the website (clapandpay TASK-125.01). The own key stays
/// in the list either way, so coins paid to the website's address never vanish from it.
fn exposed_keys(
    own: Option<chia_bls::PublicKey>,
    all: bool,
    receive_keys: &[chia_bls::PublicKey],
) -> Vec<chia_bls::PublicKey> {
    let mut keys: Vec<chia_bls::PublicKey> = Vec::new();
    if all {
        keys.extend(receive_keys.iter().filter(|key| Some(**key) != own));
    }
    keys.extend(own);
    keys
}

/// A connection stored between 2026-10-08 and TASK-125.01 lists the website's own key first:
/// move it last. `true` when the order changed.
fn own_key_last(exposed: &mut [String], own: Option<&str>) -> bool {
    match (exposed.first(), own) {
        (Some(first), Some(own)) if exposed.len() > 1 && first == own => {
            exposed.rotate_left(1);
            true
        }
        _ => false,
    }
}

fn key_hex(key: &chia_bls::PublicKey) -> String {
    format!("0x{}", hex::encode(key.to_bytes()))
}

impl<R: Runtime> Xchonnect<R> {
    /// Start the message loop for one session, unless it is already running or the
    /// session needs re-pairing.
    async fn ensure_loop(&self, stored: StoredSession) -> Result<bool> {
        let id = stored.record.id.clone();
        if stored.record.revoked || stored.record.needs_repair || !stored.record.sas_confirmed {
            return Ok(false);
        }
        let mut live = self.live.lock().await;
        if live.get(&id).is_some_and(Live::running) {
            return Ok(true);
        }
        // A session paired by a development build keeps its flag in the store; a release
        // build never honours it (plain-HTTP relays are a development affordance only).
        let developer_mode = stored.record.developer_mode && developer_mode_allowed();
        let relay = relay::RelayClient::new(&stored.record.relay, developer_mode)?;
        let end_requested = Arc::new(AtomicBool::new(false));
        let rotate_requested = Arc::new(AtomicBool::new(false));
        let permissions_changed = Arc::new(AtomicBool::new(false));
        let interrupt = Arc::new(tokio::sync::Notify::new());
        let drain = Arc::new(session_loop::Drain::default());
        let (wake_tx, wake_rx) = tokio::sync::mpsc::channel(8);
        let state: AppState = self.app.state::<AppState>().inner().clone();
        let message_loop = SessionLoop {
            state,
            store: self.store.clone(),
            approvals: self.approvals.clone(),
            emit: self.emit.clone(),
            relay,
            // The loop reads the relay's own limits when it starts: no network call is
            // made while `live` is locked.
            info: relay::RelayInfo::default(),
            end_requested: end_requested.clone(),
            rotate_requested: rotate_requested.clone(),
            permissions_changed: permissions_changed.clone(),
            interrupt: interrupt.clone(),
            wake: wake_rx,
            drain: drain.clone(),
            push: self.push.clone(),
            answer: session_loop::default_answerer(),
        };
        session_loop::spawn_ticker(wake_tx.clone());
        tauri::async_runtime::spawn(message_loop.run(stored));
        live.insert(
            id,
            Live {
                end_requested,
                rotate_requested,
                permissions_changed,
                interrupt,
                wake: wake_tx,
                drain,
            },
        );
        Ok(true)
    }

    /// Listen to the native push side, which registers with APNs when the user already
    /// allowed notifications. Off without a gateway: nothing could receive a wake-up.
    fn start_push(&self) {
        if self.push.gateway().is_none() {
            return;
        }
        let (native, app) = (self.native.clone(), self.app.clone());
        let push = self.push.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let state = native.start(move |event| {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let this = app.state::<Xchonnect<R>>();
                    match event {
                        native_push::NativeEvent::Token { platform, token } => {
                            this.set_device_token(platform, &token).await;
                        }
                        native_push::NativeEvent::Wake => this.wake_all().await,
                        native_push::NativeEvent::BackgroundWake => {
                            this.drain_in_background().await;
                        }
                    }
                });
            });
            if let Some(state) = state {
                push.set_permission(state);
            }
        });
    }

    /// Ask for notification permission, at the first pairing and not at launch, so the
    /// prompt comes with a reason. The answer goes to the screen as [`push::EVENT_PUSH`], so
    /// a refusal can be explained: connections then update while Klimper is open.
    fn ask_for_push_permission(&self) {
        if self.push.gateway().is_none() {
            return;
        }
        let (native, emit, push) = (self.native.clone(), self.emit.clone(), self.push.clone());
        tauri::async_runtime::spawn_blocking(move || {
            if let Some(state) = native.request_permission() {
                push.set_permission(state);
                emit(push::EVENT_PUSH, json!({ "state": state }));
            }
        });
    }

    /// The native side hands over the phone's APNs device token (at launch and whenever iOS
    /// issues a new one). A new token makes every running loop register it at its next pass.
    pub async fn set_device_token(&self, kind: push::Kind, token: &str) {
        if self.push.set_device_token(kind, token) {
            self.wake_all().await;
        }
    }

    /// A push wake-up arrived: every running loop reads its mailbox now instead of at the
    /// next tick. What the notification carried is never looked at.
    pub async fn wake_all(&self) {
        for live in self
            .live
            .lock()
            .await
            .values()
            .filter(|live| live.running())
        {
            live.interrupt.notify_one();
            let _ = live.wake.try_send(());
        }
    }

    /// A push woke the app in the background (TASK-125.04): every loop reads its mailbox,
    /// answers what needs no one, stores what needs the user (shown when Klimper is opened),
    /// and then iOS is told it may suspend the app again. Bounded, since iOS allows about
    /// 30 seconds.
    async fn drain_in_background(&self) {
        // Launched by the push itself: the loops start once the app's screen has loaded
        // (`list_sessions`). Give them a moment rather than telling iOS "done" at once.
        let started = std::time::Instant::now();
        let drains: Vec<_> = loop {
            let drains: Vec<_> = {
                let live = self.live.lock().await;
                live.values()
                    .filter(|live| live.running())
                    .map(|live| live.drain.clone())
                    .collect()
            };
            if !drains.is_empty() || started.elapsed() >= LOOPS_START_WAIT {
                break drains;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        };
        // Done means: a read that started after this wake-up has finished in every loop.
        let before: Vec<u64> = drains
            .iter()
            .map(|drain| {
                let done = drain.done.load(Ordering::SeqCst);
                drain.requested.store(true, Ordering::SeqCst);
                done
            })
            .collect();
        self.wake_all().await;
        let _ = tokio::time::timeout(BACKGROUND_DRAIN, async {
            for (drain, before) in drains.iter().zip(before) {
                loop {
                    let passed = drain.passed.notified();
                    if drain.done.load(Ordering::SeqCst) > before {
                        break;
                    }
                    passed.await;
                }
            }
        })
        .await;
        let native = self.native.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || native.background_done()).await;
    }

    /// Sessions the user has disconnected whose loop is still posting `session.end`.
    async fn ending(&self) -> Vec<String> {
        self.live
            .lock()
            .await
            .iter()
            .filter(|(_, live)| live.ending())
            .map(|(id, _)| id.clone())
            .collect()
    }

    async fn attempt(&self, pairing_id: &str) -> Result<SharedAttempt> {
        self.attempts
            .lock()
            .await
            .get(pairing_id)
            .cloned()
            .ok_or(Error::UnknownPairing)
    }

    /// Forget an attempt and delete its mailbox. Returns at once: the deletion waits for
    /// whatever holds the attempt (a poll, a reply being posted) in the background.
    async fn drop_attempt(&self, pairing_id: &str) {
        let attempt = self.attempts.lock().await.remove(pairing_id);
        if let Some(attempt) = attempt {
            tauri::async_runtime::spawn(async move {
                attempt.lock().await.discard().await;
            });
        }
    }
}

// ---------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------

#[command]
async fn status<R: Runtime>(app: AppHandle<R>, state: State<'_, Xchonnect<R>>) -> Result<Status> {
    let app_state = app.state::<AppState>();
    let (fingerprint, chain_id) = {
        let sage = app_state.lock().await;
        match sage.wallet() {
            Ok(wallet) => (
                Some(wallet.fingerprint),
                Some(wallet::network_of(sage.network()).chain_id().to_owned()),
            ),
            Err(_) => (None, None),
        }
    };
    let session_count = match fingerprint {
        Some(fingerprint) => {
            let ending = state.ending().await;
            state
                .store
                .list(fingerprint)?
                .iter()
                .filter(|s| !ending.contains(&s.record.id))
                .count()
        }
        None => 0,
    };
    Ok(Status {
        developer_mode_allowed: developer_mode_allowed(),
        fingerprint,
        chain_id,
        session_count,
        keyring_service: store::KEYRING_SERVICE.to_owned(),
    })
}

/// Step 1 and 2 of pairing. Verifies the URI and the origin document and returns what the
/// user has to agree to. Nothing is created on the relay yet.
#[command]
async fn inspect_pairing_uri<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
    uri: String,
) -> Result<PairingPreview> {
    let snapshot = wallet::snapshot(&app.state::<AppState>().inner().clone()).await?;
    let mut attempt = pairing::inspect(
        &uri,
        now(),
        developer_mode_allowed(),
        snapshot.fingerprint,
        snapshot.network.chain_id().to_owned(),
        Vec::new(),
    )
    .await?;
    // Now that the domain is verified: the key it would get (claimed only once the user has
    // compared the six digits), and the limits remembered for it.
    let dapp = attempt.verified.uri().domain.clone();
    let own = state
        .store
        .peek_website_key(snapshot.fingerprint, &dapp, &snapshot.website_keys)?;
    attempt.permissions.exposed_keys = own.into_iter().collect();
    if let Some(memory) = state.store.load_domain(snapshot.fingerprint, &dapp)? {
        attempt.permissions.limits = store::StoredPermissions {
            limits: memory.limits,
            ..Default::default()
        }
        .to_kit()
        .limits;
    }
    let preview = attempt.preview();
    let mut attempts = state.attempts.lock().await;
    // A preview the user walked away from is never cancelled; one past its deadline is
    // worth nothing, so it goes when the next code is scanned. One that is busy (being
    // replied to or polled) is left to its own deadline.
    let now_s = now();
    attempts.retain(|_, a| a.try_lock().map_or(true, |a| a.deadline >= now_s));
    attempts.insert(attempt.id.clone(), Arc::new(Mutex::new(attempt)));
    Ok(preview)
}

/// The user agreed to the verified domain: create the mailbox, post the reply, show the
/// six digits and start waiting for the dApp.
#[command]
async fn approve_pairing<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
    pairing_id: String,
) -> Result<PairingSas> {
    let attempt = state.attempt(&pairing_id).await?;
    let sas = pairing::reply(&mut *attempt.lock().await, now()).await?;
    spawn_confirm_poller(app, pairing_id);
    Ok(sas)
}

#[command]
async fn cancel_pairing<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
    pairing_id: String,
) -> Result<()> {
    state.drop_attempt(&pairing_id).await;
    Ok(())
}

/// The user compared the digits. `matches: false` ends the session and deletes everything.
#[command]
async fn confirm_sas<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
    pairing_id: String,
    matches: bool,
    all_receive_keys: Option<bool>,
) -> Result<Option<SessionSummary>> {
    let shared = state.attempt(&pairing_id).await?;
    let mut attempt = shared.lock().await;
    if !matches {
        pairing::reject(&mut attempt, now()).await?;
        drop(attempt);
        state.attempts.lock().await.remove(&pairing_id);
        (state.emit)(EVENT_SESSIONS, json!({}));
        return Ok(None);
    }
    // The dApp has not answered yet: nothing to confirm, and nothing to throw away either.
    if attempt.session.is_none() {
        return Err(Error::UnknownPairing);
    }
    // The six digits match: claim the website's own key now, under the store's lock, so two
    // pairings can never take the same one.
    let snapshot = wallet::snapshot(&app.state::<AppState>().inner().clone()).await?;
    if snapshot.fingerprint != attempt.fingerprint {
        return Err(Error::NoWallet);
    }
    let dapp = attempt.verified.uri().domain.clone();
    let own = state
        .store
        .claim_website_key(snapshot.fingerprint, &dapp, &snapshot.website_keys)?;
    // Every receive key if the user chose that on the pairing screen, then its own key (the
    // record keeps it as `own_key`).
    attempt.own_key = own;
    attempt.permissions.exposed_keys =
        exposed_keys(own, all_receive_keys == Some(true), &snapshot.receive_keys);
    let stored = match pairing::confirm(&mut attempt, &state.store, now()).await {
        Ok(stored) => stored,
        Err(error) => {
            // Nothing is left half made: `confirm` undid the stored session, and the
            // attempt and its mailbox go, so the screen can only offer to start again.
            drop(attempt);
            state.drop_attempt(&pairing_id).await;
            return Err(error);
        }
    };
    let keys = attempt
        .session
        .as_ref()
        .map_or_else(KeyState::default, KeyState::of);
    drop(attempt);
    state.attempts.lock().await.remove(&pairing_id);

    let connected = state.ensure_loop(stored.clone()).await?;
    let summary = summary(&stored, keys, connected);
    (state.emit)(EVENT_SESSIONS, json!({}));
    state.ask_for_push_permission();
    Ok(Some(summary))
}

/// Whether wake-ups can reach this phone, for the screen that explains why they do not:
/// `enabled` is false in a build without a push gateway, and `permission` is what the
/// phone's notification settings said when last asked (`null` before the first answer).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PushStatus {
    enabled: bool,
    permission: Option<push::PermissionState>,
}

#[command]
async fn push_status<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
) -> Result<PushStatus> {
    Ok(PushStatus {
        enabled: state.push.gateway().is_some(),
        permission: state.push.permission(),
    })
}

/// The connected dApps of the active wallet. Also resumes the message loop of every
/// session that is not already running, which is how sessions come back after a restart
/// or after a wallet was opened.
#[command]
async fn list_sessions<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
) -> Result<Vec<SessionSummary>> {
    let snapshot = wallet::snapshot(&app.state::<AppState>().inner().clone()).await?;
    let sessions = state.store.list(snapshot.fingerprint)?;
    // Only entries whose loop has returned are dropped. Pruning by this wallet's list
    // would orphan the running loops of another wallet's sessions, and switching back
    // would then start a second loop on the same session: two writers of one `seq`.
    state.live.lock().await.retain(|_, live| live.running());
    let ending = state.ending().await;
    let mut out = Vec::with_capacity(sessions.len());
    for mut stored in sessions {
        // Disconnected a moment ago: gone as far as the user is concerned, while the loop
        // finishes telling the dApp.
        if ending.contains(&stored.record.id) {
            continue;
        }
        let mut reordered = own_key_last(
            &mut stored.record.permissions.exposed_keys.clone(),
            stored.record.own_key.as_deref(),
        );
        if reordered {
            // Reordered on the record as it is under the store's lock, so a permission change
            // the user made a moment ago is never written back over (TASK-125 review).
            stored =
                state
                    .store
                    .update_policy(snapshot.fingerprint, &stored.record.id, |record| {
                        let own = record.own_key.clone();
                        reordered =
                            own_key_last(&mut record.permissions.exposed_keys, own.as_deref());
                    })?;
        }
        // Under the store's lock for the same reason as the reordering above.
        let mut upgraded = false;
        if stored.record.methods_revision < execute::METHODS_REVISION {
            stored =
                state
                    .store
                    .update_policy(snapshot.fingerprint, &stored.record.id, |record| {
                        upgraded = execute::upgrade_methods(record);
                    })?;
        }
        let keys = KeyState::stored(&stored);
        let connected = state.ensure_loop(stored.clone()).await.unwrap_or(false);
        if (reordered || upgraded)
            && let Some(live) = state.live.lock().await.get(&stored.record.id)
        {
            live.announce_permissions();
        }
        out.push(summary(&stored, keys, connected));
    }
    Ok(out)
}

/// What the screen shows about a session's keys.
#[derive(Debug, Clone, Copy, Default)]
struct KeyState {
    epoch: u64,
    rotating: bool,
}

impl KeyState {
    fn of(session: &xchonnect_core::session::Session) -> Self {
        KeyState {
            epoch: session.epoch(),
            rotating: session.pending_rotation_mailbox().is_some()
                || session.draining_mailbox().is_some(),
        }
    }

    /// From the stored bytes. A revoked session has none left, and reads as epoch 0.
    fn stored(stored: &StoredSession) -> Self {
        stored
            .session_bytes()
            .ok()
            .and_then(|bytes| xchonnect_core::session::Session::from_bytes(&bytes).ok())
            .map_or_else(KeyState::default, |session| KeyState::of(&session))
    }
}

/// End a connection: `session.end` to the dApp, then the mailboxes and the keyring entries
/// go. Performed by the session's own loop, so it can never interleave with a message the
/// loop is in the middle of sealing.
#[command]
async fn end_session<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
    session_id: String,
) -> Result<()> {
    // Whatever the session is waiting on is rejected now, not when the loop gets to it: a
    // prompt for a connection the user just ended must not stay on screen.
    state.approvals.withdraw_session(&session_id);
    let mut live = state.live.lock().await;
    if let Some(handle) = live.get(&session_id) {
        if handle.running() {
            handle.request_end();
            drop(live);
            // `list_sessions` leaves an ending session out, so the list is right at once.
            (state.emit)(EVENT_SESSIONS, json!({}));
            return Ok(());
        }
        // The loop returned without removing the session (it was marked for re-pairing):
        // flagging it would do nothing at all, so end it as if no loop had ever run.
        live.remove(&session_id);
    }
    drop(live);
    // No loop running (no wallet was open when it would have started, or the session
    // needs re-pairing): the website is still told, and this side's mailboxes go, so
    // disconnecting always unpairs both sides.
    let snapshot = wallet::snapshot(&app.state::<AppState>().inner().clone()).await?;
    if let Ok(mut stored) = state.store.load(snapshot.fingerprint, &session_id) {
        session_loop::end_without_loop(&state.store, &mut stored).await;
    }
    state.store.remove(snapshot.fingerprint, &session_id)?;
    (state.emit)(EVENT_SESSIONS, json!({}));
    Ok(())
}

/// Renew one connection's keys (spec 9.2.1, the wallet as initiator). The session's loop
/// creates a mailbox and posts the offer; the switch happens when the dApp accepts, and
/// the `rotating` flag on the summary stays set until the old mailbox is drained.
#[command]
async fn rotate_session_keys<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
    session_id: String,
) -> Result<()> {
    let live = state.live.lock().await;
    match live.get(&session_id) {
        Some(handle) if handle.running() && !handle.ending() => {
            handle.request_rotation();
            Ok(())
        }
        // Not listening (no wallet open, needs re-pairing, revoked, being ended): there is
        // no loop to carry the offer, and a rotation needs the dApp to answer.
        _ => Err(Error::SessionNotUsable),
    }
}

/// Change one dApp's permissions and limits. The session loop reloads them before every
/// request, so a change applies to the next request without restarting anything.
#[command]
async fn set_dapp_policy<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
    session_id: String,
    policy: DappPolicy,
) -> Result<SessionSummary> {
    // A limit on an asset this build cannot read would be dropped on load and leave the asset
    // unlimited: refuse it here instead.
    if let Some(limits) = &policy.limits
        && let Some(bad) = limits
            .keys()
            .find(|key| store::parse_asset_key(key).is_none())
    {
        return Err(Error::Protocol(format!("unknown asset in a limit: {bad}")));
    }
    let snapshot = wallet::snapshot(&app.state::<AppState>().inner().clone()).await?;
    // Which keys the website sees: its own key (claimed now for a website paired before it
    // had one), and every receive key when the user allows that.
    let keys = match policy.all_receive_keys {
        None => None,
        Some(all) => {
            let current = state.store.load(snapshot.fingerprint, &session_id)?;
            let own = match current
                .record
                .own_key
                .as_deref()
                .and_then(store::parse_public_key)
                // A record from before own keys were claimed may name a receive key here.
                .filter(|key| snapshot.website_keys.contains(key))
            {
                Some(own) => Some(own),
                None => state.store.claim_website_key(
                    snapshot.fingerprint,
                    &current.record.dapp,
                    &snapshot.website_keys,
                )?,
            };
            let exposed = exposed_keys(own, all, &snapshot.receive_keys)
                .iter()
                .map(key_hex)
                .collect::<Vec<_>>();
            Some((own.map(|key| key_hex(&key)), exposed))
        }
    };
    let keys_changed = keys.is_some();
    let limits = policy.stored_limits();
    let stored = state
        .store
        .update_policy(snapshot.fingerprint, &session_id, |record| {
            if let Some((own, exposed)) = keys {
                record.own_key = own;
                record.permissions.exposed_keys = exposed;
            }
            if let Some(limits) = limits {
                record.permissions.limits = limits;
            }
            if let Some(methods) = policy.methods {
                record.permissions.methods = methods;
            }
            if let Some(allow) = policy.allow_agg_sig_unsafe {
                record.allow_agg_sig_unsafe = allow;
            }
            if let Some(allow) = policy.allow_unknown_contracts {
                record.allow_unknown_contracts = allow;
            }
        })?;
    // A prompt already on screen was checked against the old settings: withdraw it.
    state.approvals.withdraw_waiting(&session_id);
    // Remembered for the domain, so connecting again brings the limits back.
    let memory = store::DomainMemory {
        limits: stored.record.permissions.limits.clone(),
    };
    state
        .store
        .save_domain(snapshot.fingerprint, &stored.record.dapp, &memory)?;
    let keys = KeyState::stored(&stored);
    let connected = {
        let live = state.live.lock().await;
        let entry = live.get(&session_id);
        if keys_changed && let Some(entry) = entry {
            entry.announce_permissions();
        }
        entry.is_some_and(Live::running)
    };
    (state.emit)(EVENT_SESSIONS, json!({}));
    Ok(summary(&stored, keys, connected))
}

/// Everything waiting for the user, so a reloaded screen can rebuild itself.
#[command]
async fn pending_approvals<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
) -> Result<Vec<Approval>> {
    Ok(state.approvals.outstanding())
}

/// Answer one approval. `approved: false` is a CHIP-0002 user rejection; no answer within
/// five minutes is one too.
#[command]
async fn resolve_approval<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
    approval_id: String,
    approved: bool,
) -> Result<()> {
    state.approvals.resolve(&approval_id, approved)
}

/// Withdraw a waiting request from the requests page. Unlike rejecting it (4002), the
/// dApp is told the request was cancelled (`rpc.cancel`, then 4102), so it can drop it from
/// its own list.
#[command]
async fn cancel_approval<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
    approval_id: String,
) -> Result<()> {
    state.approvals.cancel(&approval_id)
}

/// Polls the wallet's mailbox for `session.confirm` and gives up after five minutes with
/// the same warning as a `not_found` (TASK-076 AC #3).
fn spawn_confirm_poller<R: Runtime>(app: AppHandle<R>, pairing_id: String) {
    tauri::async_runtime::spawn(async move {
        loop {
            let state = app.state::<Xchonnect<R>>();
            // Looked up afresh each round: a cancelled attempt is simply no longer there.
            let Ok(shared) = state.attempt(&pairing_id).await else {
                return;
            };
            let outcome = {
                let mut attempt = shared.lock().await;
                pairing::poll_for_confirm(&mut attempt, now())
                    .await
                    .map(|created| created.then(|| attempt.sas()))
            };
            match outcome {
                Ok(Some(sas)) => {
                    (state.emit)(
                        EVENT_PAIRING,
                        json!({
                            "pairingId": pairing_id,
                            "state": "awaitingSas",
                            "sas": sas.as_ref().map(|s| s.sas.clone()),
                            "dappConfirmed": true,
                        }),
                    );
                    return;
                }
                Ok(None) => tokio::time::sleep(pairing::POLL_GAP).await,
                Err(error) => {
                    state.drop_attempt(&pairing_id).await;
                    (state.emit)(
                        EVENT_PAIRING,
                        json!({
                            "pairingId": pairing_id,
                            "state": "failed",
                            "error": error.to_string(),
                        }),
                    );
                    return;
                }
            }
        }
    });
}

/// Register the plugin. Called from `src-tauri/src/lib.rs`.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("klimper-xchonnect")
        .invoke_handler(tauri::generate_handler![
            status,
            inspect_pairing_uri,
            approve_pairing,
            cancel_pairing,
            confirm_sas,
            list_sessions,
            end_session,
            rotate_session_keys,
            set_dapp_policy,
            pending_approvals,
            resolve_approval,
            cancel_approval,
            push_status
        ])
        .setup(|app, api| {
            let data_dir = app.path().app_data_dir()?;
            let store = Arc::new(SessionStore::open(&data_dir, Box::new(OsKeyring))?);
            let handle = app.clone();
            // Addressed to the main webview. Not a boundary on its own: Tauri delivers an
            // event to every listener registered for any target, so an installed app's
            // webview (`core:event:allow-listen`) can still hear it. Mini-apps have no entry
            // point on iOS; on desktop, events that carry no data (details fetched through
            // the commands only `main` may call) are clapandpay TASK-102.
            let emit: Emitter = Arc::new(move |event, payload| {
                use tauri::Emitter as _;
                if let Err(error) = handle.emit_to(MAIN_WEBVIEW, event, payload) {
                    tracing::warn!("could not emit {event}: {error}");
                }
            });
            app.manage(Xchonnect {
                app: app.clone(),
                store,
                approvals: Arc::new(ApprovalBridge::new(emit.clone())),
                emit,
                attempts: Mutex::new(BTreeMap::new()),
                live: Mutex::new(BTreeMap::new()),
                push: Arc::new(push::PushHandle::new(push::GatewayConfig::from_build())),
                native: native_push::NativePush::init(app, &api)?,
            });
            app.state::<Xchonnect<R>>().start_push();
            Ok(())
        })
        // Back in front (TASK-125.02): a long poll opened before the phone suspended the app
        // may sit on a dead socket, so every loop drops it and reads its mailbox now.
        .on_event(|app, event| {
            if matches!(event, tauri::RunEvent::Resumed) {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    app.state::<Xchonnect<R>>().wake_all().await;
                });
            }
        })
        .build()
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    /// The limits exactly as the website-settings screen sends them (the same file is
    /// checked against the screen's payload in `XchonnectDappSettings.test.tsx`).
    const WEBVIEW_LIMITS: &str = include_str!("../tests/fixtures/webview_limits.json");
    const CAT: &str = "cat:abababababababababababababababababababababababababababababababab";

    /// The webview's `perRequest` / `perDay` reach the stored form, and the summary hands
    /// them back in the same words. Before `LimitView` they were read as the stored form's
    /// `per_request` / `per_day`, skipped as unknown, and every limit meant no limit.
    #[test]
    #[allow(clippy::unwrap_used, clippy::indexing_slicing)]
    fn the_webviews_limits_are_read_as_sent() {
        let limits: serde_json::Value = serde_json::from_str(WEBVIEW_LIMITS).unwrap();
        let policy: DappPolicy = serde_json::from_value(json!({ "limits": limits })).unwrap();
        let stored = policy.stored_limits().unwrap();
        assert_eq!(
            stored["xch"],
            StoredLimit {
                per_request: Some(100),
                per_day: Some(1000),
            }
        );
        assert_eq!(
            stored[CAT],
            StoredLimit {
                per_request: None,
                per_day: Some(5000),
            }
        );
        assert_eq!(serde_json::to_value(limit_views(&stored)).unwrap(), limits);
    }

    /// A limit in any other words is refused at the boundary, never read as no limit.
    #[test]
    fn a_limit_the_screen_names_differently_is_refused() {
        for wrong in [
            json!({ "limits": { "xch": { "per_request": 100 } } }),
            json!({ "limits": { "xch": { "perRequest": 100, "perWeek": 1 } } }),
            json!({ "limits": { "xch": { "perRequest": "100" } } }),
            json!({ "limit": { "xch": { "perRequest": 100 } } }),
        ] {
            assert!(
                serde_json::from_value::<DappPolicy>(wrong.clone()).is_err(),
                "{wrong}"
            );
        }
        // Every field the screen sends is still read.
        assert!(
            serde_json::from_value::<DappPolicy>(json!({
                "limits": {},
                "methods": ["chip0002_chainId"],
                "allowAggSigUnsafe": false,
                "allowUnknownContracts": false,
                "allReceiveKeys": true,
            }))
            .is_ok()
        );
    }

    #[test]
    fn a_release_build_never_accepts_a_developer_mode_uri() {
        // The constant is the only switch; there is no setting and no environment
        // variable that turns developer mode on in a release build.
        assert_eq!(developer_mode_allowed(), cfg!(debug_assertions));
    }

    #[test]
    fn all_addresses_keep_the_websites_own_one() {
        let key = |i: u8| chia_bls::SecretKey::from_seed(&[i; 64]).public_key();
        let receive = [key(1), key(2)];
        assert_eq!(exposed_keys(Some(key(9)), false, &receive), vec![key(9)]);
        // The wallet's first address first: websites read the first key as the wallet.
        assert_eq!(
            exposed_keys(Some(key(9)), true, &receive),
            vec![key(1), key(2), key(9)]
        );
        assert_eq!(exposed_keys(None, true, &receive), receive.to_vec());
    }

    #[test]
    fn a_connection_stored_with_its_own_key_first_gets_it_last() {
        let mut exposed = vec!["own".to_owned(), "r0".to_owned(), "r1".to_owned()];
        assert!(own_key_last(&mut exposed, Some("own")));
        assert_eq!(exposed, ["r0", "r1", "own"]);
        assert!(!own_key_last(&mut exposed, Some("own")));
        let mut alone = vec!["own".to_owned()];
        assert!(!own_key_last(&mut alone, Some("own")));
        let mut old = vec!["r0".to_owned(), "r1".to_owned()];
        assert!(!own_key_last(&mut old, None));
    }

    #[test]
    fn websites_never_share_a_key_even_after_one_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            SessionStore::open(dir.path(), Box::new(store::MemorySecrets::default())).unwrap();
        let hardened: Vec<chia_bls::PublicKey> = (0u8..4)
            .map(|i| chia_bls::SecretKey::from_seed(&[i; 64]).public_key())
            .collect();

        // The first free hardened key, in order.
        let a = store.claim_website_key(1, "a.example", &hardened).unwrap();
        assert_eq!(a, Some(hardened[0]));
        // A preview shows what a pairing would get, without taking it.
        assert_eq!(
            store.peek_website_key(1, "b.example", &hardened).unwrap(),
            Some(hardened[1])
        );
        // A's connection is gone (the list does not care): B still gets another key.
        let b = store.claim_website_key(1, "b.example", &hardened).unwrap();
        assert_eq!(b, Some(hardened[1]));
        // A comes back: its own key again, never B's.
        assert_eq!(
            store.claim_website_key(1, "a.example", &hardened).unwrap(),
            a
        );
        // Another wallet has its own list.
        assert_eq!(
            store.claim_website_key(2, "b.example", &hardened).unwrap(),
            Some(hardened[0])
        );
        // No free key left: none, rather than a shared one.
        store.claim_website_key(1, "c.example", &hardened).unwrap();
        store.claim_website_key(1, "d.example", &hardened).unwrap();
        assert_eq!(
            store.claim_website_key(1, "e.example", &hardened).unwrap(),
            None
        );
    }
}
