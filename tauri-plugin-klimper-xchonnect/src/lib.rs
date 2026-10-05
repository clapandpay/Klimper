// SPDX-License-Identifier: AGPL-3.0-or-later
//! Klimper ↔ Xchonnect: connect the wallet to a Chia dApp as a remote signing target
//! (clapandpay decision-008, TASK-071 to TASK-079).
//!
//! The protocol and every safety decision live in two Apache-2.0 crates,
//! `xchonnect-core` (URIs, origin verification, pairing, sessions, envelopes) and
//! `xchonnect-wallet-kit` (local simulation, signature policy, permissions, limits,
//! binding checks, CHIP-0002 handlers). This plugin supplies only what a host has to:
//! HTTP to the relay and to the dApp's origin document, session storage in the OS
//! keyring, the approval UI, and signing through Klimper's own key storage. No Sage
//! engine file is changed — the plugin follows `tauri-plugin-klimper-passkey` exactly,
//! and the wiring points are listed in `klimper/docs/UPSTREAM.md`.
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
//! | `set_dapp_policy` | `sessionId: string`, `policy:` [`DappPolicy`] | [`SessionSummary`] |
//! | `pending_approvals` | — | [`Approval`]`[]` |
//! | `resolve_approval` | `approvalId: string`, `approved: boolean` | `null` |
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
pub mod error;
pub mod origin;
pub mod pairing;
pub mod relay;
pub mod requests;
pub mod session_loop;
pub mod store;
mod testing;
pub mod wallet;

use approval::{Approval, ApprovalBridge, Emitter};
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

/// A running session's control handles. Dropping the sender is what stops the loop and
/// its ticker, so removing the entry is all a shutdown has to do.
struct Live {
    end_requested: Arc<AtomicBool>,
    wake: tokio::sync::mpsc::Sender<()>,
}

/// Plugin state.
pub struct Xchonnect<R: Runtime> {
    app: AppHandle<R>,
    store: Arc<SessionStore>,
    approvals: Arc<ApprovalBridge>,
    emit: Emitter,
    attempts: Mutex<BTreeMap<String, Attempt>>,
    live: Mutex<BTreeMap<String, Live>>,
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
    pub developer_mode: bool,
    pub methods: Vec<String>,
    pub exposed_keys: Vec<String>,
    pub limits: BTreeMap<String, StoredLimit>,
    pub allow_agg_sig_unsafe: bool,
    pub allow_unknown_contracts: bool,
    /// The message loop is running for this session.
    pub connected: bool,
}

/// Changes a user can make to one dApp's permissions (the full screen is TASK-083).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DappPolicy {
    /// Replace the per-asset limits. Keys are `xch` or `cat:<64 hex>`.
    #[serde(default)]
    pub limits: Option<BTreeMap<String, StoredLimit>>,
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
}

/// Describe one stored session for the webview.
fn summary(stored: &StoredSession, epoch: u64, connected: bool) -> SessionSummary {
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
        epoch,
        sas_confirmed: stored.record.sas_confirmed,
        needs_repair: stored.record.needs_repair,
        developer_mode: stored.record.developer_mode,
        methods: stored.record.permissions.methods.clone(),
        exposed_keys: stored.record.permissions.exposed_keys.clone(),
        limits: stored.record.permissions.limits.clone(),
        allow_agg_sig_unsafe: stored.record.allow_agg_sig_unsafe,
        allow_unknown_contracts: stored.record.allow_unknown_contracts,
        connected,
    }
}

impl<R: Runtime> Xchonnect<R> {
    /// Start the message loop for one session, unless it is already running or the
    /// session needs re-pairing.
    async fn ensure_loop(&self, stored: StoredSession) -> Result<bool> {
        let id = stored.record.id.clone();
        if stored.record.needs_repair || !stored.record.sas_confirmed {
            return Ok(false);
        }
        let mut live = self.live.lock().await;
        if live.contains_key(&id) {
            return Ok(true);
        }
        let relay = relay::RelayClient::new(&stored.record.relay, stored.record.developer_mode)?;
        let info = relay.info().await.unwrap_or_default();
        let end_requested = Arc::new(AtomicBool::new(false));
        let (wake_tx, wake_rx) = tokio::sync::mpsc::channel(8);
        let state: AppState = self.app.state::<AppState>().inner().clone();
        let message_loop = SessionLoop {
            state,
            store: self.store.clone(),
            approvals: self.approvals.clone(),
            emit: self.emit.clone(),
            relay,
            info,
            end_requested: end_requested.clone(),
            wake: wake_rx,
        };
        session_loop::spawn_ticker(wake_tx.clone());
        tauri::async_runtime::spawn(message_loop.run(stored));
        live.insert(
            id,
            Live {
                end_requested,
                wake: wake_tx,
            },
        );
        Ok(true)
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
        Some(fingerprint) => state.store.list(fingerprint)?.len(),
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
    let attempt = pairing::inspect(
        &uri,
        now(),
        developer_mode_allowed(),
        snapshot.fingerprint,
        snapshot.network.chain_id().to_owned(),
        snapshot.receive_keys,
    )
    .await?;
    let preview = attempt.preview();
    state
        .attempts
        .lock()
        .await
        .insert(attempt.id.clone(), attempt);
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
    let sas = {
        let mut attempts = state.attempts.lock().await;
        let attempt = attempts.get_mut(&pairing_id).ok_or(Error::UnknownPairing)?;
        pairing::reply(attempt, now()).await?
    };
    spawn_confirm_poller(app, pairing_id);
    Ok(sas)
}

#[command]
async fn cancel_pairing<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
    pairing_id: String,
) -> Result<()> {
    let attempt = state.attempts.lock().await.remove(&pairing_id);
    if let Some(attempt) = attempt {
        attempt.discard().await;
    }
    Ok(())
}

/// The user compared the digits. `matches: false` ends the session and deletes everything.
#[command]
async fn confirm_sas<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
    pairing_id: String,
    matches: bool,
) -> Result<Option<SessionSummary>> {
    let mut attempts = state.attempts.lock().await;
    let attempt = attempts.get_mut(&pairing_id).ok_or(Error::UnknownPairing)?;
    if !matches {
        pairing::reject(attempt, now()).await?;
        attempts.remove(&pairing_id);
        drop(attempts);
        (state.emit)(EVENT_SESSIONS, json!({}));
        return Ok(None);
    }
    let stored = pairing::confirm(attempt, &state.store, now()).await?;
    let epoch = attempt
        .session
        .as_ref()
        .map_or(0, xchonnect_core::session::Session::epoch);
    attempts.remove(&pairing_id);
    drop(attempts);

    let connected = state.ensure_loop(stored.clone()).await?;
    let summary = summary(&stored, epoch, connected);
    (state.emit)(EVENT_SESSIONS, json!({}));
    Ok(Some(summary))
}

/// The connected dApps of the active wallet. Also resumes the message loop of every
/// session that is not already running, which is how sessions come back after a restart
/// or after the wallet was unlocked.
#[command]
async fn list_sessions<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Xchonnect<R>>,
) -> Result<Vec<SessionSummary>> {
    let snapshot = wallet::snapshot(&app.state::<AppState>().inner().clone()).await?;
    let sessions = state.store.list(snapshot.fingerprint)?;
    // A loop that has returned (the dApp ended the session) leaves a stale entry behind;
    // dropping it also drops the last `wake` sender.
    {
        let known: Vec<String> = sessions.iter().map(|s| s.record.id.clone()).collect();
        state.live.lock().await.retain(|id, _| known.contains(id));
    }
    let mut out = Vec::with_capacity(sessions.len());
    for stored in sessions {
        let epoch = epoch_of(&stored);
        let connected = state.ensure_loop(stored.clone()).await.unwrap_or(false);
        out.push(summary(&stored, epoch, connected));
    }
    Ok(out)
}

fn epoch_of(stored: &StoredSession) -> u64 {
    stored
        .session_bytes()
        .ok()
        .and_then(|bytes| xchonnect_core::session::Session::from_bytes(&bytes).ok())
        .map_or(0, |session| session.epoch())
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
    let live = state.live.lock().await;
    if let Some(handle) = live.get(&session_id) {
        handle.end_requested.store(true, Ordering::SeqCst);
        let _ = handle.wake.try_send(());
        return Ok(());
    }
    drop(live);
    // No loop running (the session needs re-pairing, or the wallet is locked): forget it
    // locally. The relay drops an unused mailbox after 30 days.
    let snapshot = wallet::snapshot(&app.state::<AppState>().inner().clone()).await?;
    state.store.remove(snapshot.fingerprint, &session_id)?;
    (state.emit)(EVENT_SESSIONS, json!({}));
    Ok(())
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
    let snapshot = wallet::snapshot(&app.state::<AppState>().inner().clone()).await?;
    let mut stored = state.store.load(snapshot.fingerprint, &session_id)?;
    if let Some(limits) = policy.limits {
        stored.record.permissions.limits = limits;
    }
    if let Some(methods) = policy.methods {
        stored.record.permissions.methods = methods;
    }
    if let Some(allow) = policy.allow_agg_sig_unsafe {
        stored.record.allow_agg_sig_unsafe = allow;
    }
    if let Some(allow) = policy.allow_unknown_contracts {
        stored.record.allow_unknown_contracts = allow;
    }
    state.store.save(&mut stored)?;
    let epoch = epoch_of(&stored);
    let connected = state.live.lock().await.contains_key(&session_id);
    (state.emit)(EVENT_SESSIONS, json!({}));
    Ok(summary(&stored, epoch, connected))
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

/// Polls the wallet's mailbox for `session.confirm` and gives up after five minutes with
/// the same warning as a `not_found` (TASK-076 AC #3).
fn spawn_confirm_poller<R: Runtime>(app: AppHandle<R>, pairing_id: String) {
    tauri::async_runtime::spawn(async move {
        loop {
            let state = app.state::<Xchonnect<R>>();
            let outcome = {
                let mut attempts = state.attempts.lock().await;
                let Some(attempt) = attempts.get_mut(&pairing_id) else {
                    return; // cancelled
                };
                pairing::poll_for_confirm(attempt, now()).await
            };
            match outcome {
                Ok(true) => {
                    let attempts = state.attempts.lock().await;
                    let sas = attempts.get(&pairing_id).and_then(Attempt::sas);
                    drop(attempts);
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
                Ok(false) => tokio::time::sleep(pairing::POLL_GAP).await,
                Err(error) => {
                    let message = error.to_string();
                    let attempt = state.attempts.lock().await.remove(&pairing_id);
                    if let Some(attempt) = attempt {
                        attempt.discard().await;
                    }
                    (state.emit)(
                        EVENT_PAIRING,
                        json!({
                            "pairingId": pairing_id,
                            "state": "failed",
                            "error": message,
                        }),
                    );
                    return;
                }
            }
        }
    });
}

/// Register the plugin. Called from `src-tauri/src/lib.rs`, next to
/// `tauri_plugin_klimper_passkey::init()`.
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
            set_dapp_policy,
            pending_approvals,
            resolve_approval
        ])
        .setup(|app, _api| {
            let data_dir = app.path().app_data_dir()?;
            let store = Arc::new(SessionStore::open(&data_dir, Box::new(OsKeyring))?);
            let handle = app.clone();
            let emit: Emitter = Arc::new(move |event, payload| {
                use tauri::Emitter as _;
                if let Err(error) = handle.emit(event, payload) {
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
            });
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    #[test]
    fn a_release_build_never_accepts_a_developer_mode_uri() {
        // The constant is the only switch; there is no setting and no environment
        // variable that turns developer mode on in a release build.
        assert_eq!(developer_mode_allowed(), cfg!(debug_assertions));
    }
}
