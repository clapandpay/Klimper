// SPDX-License-Identifier: AGPL-3.0-or-later
//! Answering one `rpc.request` (clapandpay TASK-079).
//!
//! Everything that decides whether a spend is safe lives in
//! `xchonnect_wallet_kit::handle`: it runs every spend through the chain's CLVM
//! interpreter, derives the effect per asset **from the conditions only**, computes the
//! exact signing messages, refuses `AGG_SIG_UNSAFE` unless the user enabled it for this
//! dApp, refuses a request for another network, never signs for a key the wallet does not
//! hold, refuses a `partialSign` request whose user spends are not bound to the
//! counterparty's payment (spec 11.2 — there is no override), applies the per-dApp
//! permissions and limits, and verifies every signature the host signer returns.
//!
//! This module supplies what it needs from Klimper: the request context, the approval UI,
//! the signer, the limit store and — since the wallet broadcasts (xchonnect-wiki 04) — the
//! broadcaster. It also routes the methods Klimper builds itself (`chia_send`, see
//! [`crate::execute`]) and, for a dApp-built bundle, compares Sage's summary with
//! wallet-kit's simulation before anything is shown. Two checks are made here, before any
//! of that:
//!
//! * **The SAS gate.** A session whose six digits the user never confirmed, or whose
//!   stored state was rolled back, answers nothing at all (spec 6.3 step 8, 5.3).
//! * **The network.** A session approved for another chain is refused before any prompt
//!   is shown, for every method and not only for `signCoinSpends` (TASK-079 AC #2).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;
use xchonnect_core::rpc::{canonical_method, codes};
use xchonnect_wallet_kit::{self as kit, Approver, Host, Prompt, RequestContext, RpcError};

use crate::approval::{Approval, ApprovalBridge, ApprovalKind, DECISION_TIMEOUT};
use crate::error::Error;
use crate::execute::{self, SageBroadcaster};
use crate::store::{PersistentLimits, SessionRecord, SessionStore};
use crate::wallet::{AppState, KlimperSigner, WalletSnapshot};

fn rpc_error(code: i64, message: &str) -> RpcError {
    RpcError {
        code,
        message: message.to_owned(),
        data: None,
    }
}

/// Why a session may not answer at all.
///
/// Returned before the wallet is even looked at, so an unconfirmed session cannot be
/// probed for the wallet's keys either.
/// A request on a connection approved for another network than the wallet is on now. `data`
/// names both, so the website can tell the user which one Klimper is on.
pub fn wrong_network(wallet: &str, session: &str) -> RpcError {
    RpcError {
        code: codes::UNAUTHORIZED,
        message: "this connection was approved for another network".to_owned(),
        data: Some(
            serde_json::json!({
                "reason": "wrong_network",
                "walletChainId": wallet,
                "sessionChainId": session,
            })
            .to_string(),
        ),
    }
}

pub fn check_usable(record: &SessionRecord) -> std::result::Result<(), RpcError> {
    if record.revoked {
        return Err(rpc_error(
            codes::UNAUTHORIZED,
            "this connection was revoked: the website withdrew the key it was paired with",
        ));
    }
    if record.needs_repair {
        return Err(rpc_error(
            codes::UNAUTHORIZED,
            "this connection must be paired again",
        ));
    }
    if !record.sas_confirmed {
        return Err(rpc_error(
            codes::UNAUTHORIZED,
            "the six-digit code for this connection was never confirmed",
        ));
    }
    Ok(())
}

/// Where a request is, for the dApp (`rpc.status`, xchonnect spec 9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// The prompt is on the user's screen.
    Shown,
    /// The user approved it.
    Approved,
    /// Klimper submitted the transaction with this id.
    Broadcast([u8; 32]),
}

/// Reports [`Progress`] to the session loop, which seals it as `rpc.status`.
pub type ProgressSender = tokio::sync::mpsc::UnboundedSender<Progress>;

/// Identity of the request, used to label its approvals.
#[derive(Debug, Clone)]
pub struct RequestMeta {
    pub session_id: String,
    pub dapp: String,
    pub method: String,
    /// The Xchonnect request id (base64url), so `rpc.cancel` can find its approval.
    pub request_id: String,
    /// Unix seconds after which the dApp will no longer accept an answer.
    pub expires_at: u64,
    /// Where to report progress; `None` reports nothing.
    pub progress: Option<ProgressSender>,
}

impl RequestMeta {
    fn report(&self, progress: Progress) {
        if let Some(sender) = &self.progress {
            // The loop has gone away only if the session ended; nothing to tell then.
            let _ = sender.send(progress);
        }
    }
}

/// Approval UI as wallet-kit's `Approver`: one approval per request.
pub struct UiApprover {
    bridge: Arc<ApprovalBridge>,
    meta: RequestMeta,
    /// Sage's summary and wallet-kit's simulation were compared and agree; the screen says
    /// so.
    cross_checked: AtomicBool,
}

impl std::fmt::Debug for UiApprover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiApprover")
            .field("dapp", &self.meta.dapp)
            .finish_non_exhaustive()
    }
}

impl UiApprover {
    pub fn new(bridge: Arc<ApprovalBridge>, meta: RequestMeta) -> Self {
        UiApprover {
            bridge,
            meta,
            cross_checked: AtomicBool::new(false),
        }
    }

    /// Record that the two engines agreed on this request.
    pub fn mark_cross_checked(&self) {
        self.cross_checked.store(true, Ordering::SeqCst);
    }
}

impl Approver for UiApprover {
    fn approve(&self, prompt: &Prompt<'_>) -> bool {
        // The prompt is serialised exactly as wallet-kit built it from the local
        // simulation. Nothing the dApp wrote is passed to the screen.
        let Ok(mut payload) = serde_json::to_value(prompt) else {
            tracing::error!("an approval prompt could not be serialised; refusing");
            return false;
        };
        if self.cross_checked.load(Ordering::SeqCst)
            && let Some(fields) = payload.as_object_mut()
        {
            fields.insert("cross_checked".to_owned(), Value::Bool(true));
        }
        let approved = self.bridge.ask_with(
            Approval {
                id: new_id(),
                session_id: self.meta.session_id.clone(),
                dapp: self.meta.dapp.clone(),
                method: self.meta.method.clone(),
                request_id: self.meta.request_id.clone(),
                expires_at: self.meta.expires_at,
                kind: ApprovalKind::Request { prompt: payload },
            },
            || self.meta.report(Progress::Shown),
        );
        if approved {
            self.meta.report(Progress::Approved);
        }
        approved
    }
}

/// A random id for one approval. Not a secret; only has to be unguessable enough that two
/// prompts never collide.
pub fn new_id() -> String {
    use rand::RngCore;
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    xchonnect_core::b64::encode(&bytes)
}

/// Inputs for [`answer`].
///
/// `Debug` names the method and the dApp only: `params` is dApp-controlled request
/// content and must not reach a log line.
pub struct AnswerInputs {
    pub state: AppState,
    pub store: Arc<SessionStore>,
    pub approvals: Arc<ApprovalBridge>,
    pub record: SessionRecord,
    pub meta: RequestMeta,
    pub params: String,
    pub now: u64,
}

impl std::fmt::Debug for AnswerInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnswerInputs")
            .field("dapp", &self.meta.dapp)
            .field("method", &self.meta.method)
            .finish_non_exhaustive()
    }
}

/// Run one request to completion.
///
/// `handle` calls the approver and the signer inline, and both park until the webview
/// answers, so the whole call runs on a dedicated OS thread: not an async task (it would
/// block a reactor worker) and not `spawn_blocking` (the signer needs
/// `async_runtime::block_on` to reach the Sage mutex, which panics inside a runtime
/// context).
pub async fn answer(inputs: AnswerInputs) -> std::result::Result<String, RpcError> {
    let AnswerInputs {
        state,
        store,
        approvals,
        record,
        meta,
        params,
        now,
    } = inputs;

    // The gate comes before anything else, so an unconfirmed session cannot even be used
    // to find out whether a wallet is open or which keys it holds.
    check_usable(&record)?;

    let snapshot = crate::wallet::snapshot(&state).await?;
    if snapshot.fingerprint != record.fingerprint {
        // The user switched wallets. The session belongs to the other one and its keys
        // are not here.
        return Err(rpc_error(
            codes::UNAUTHORIZED,
            "this connection belongs to another wallet",
        ));
    }
    if snapshot.network.chain_id() != record.chain_id {
        // The user switched networks (TASK-125.05). The website may still learn which one
        // the wallet is on now, so it can follow or say so; everything else stays refused
        // (spec 11.1 item 6), naming both networks.
        if matches!(canonical_method(&meta.method), "chainId" | "connect") {
            return Ok(match canonical_method(&meta.method) {
                "chainId" => serde_json::json!(snapshot.network.chain_id()).to_string(),
                _ => "true".to_owned(),
            });
        }
        return Err(wrong_network(snapshot.network.chain_id(), &record.chain_id));
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    let thread = std::thread::Builder::new()
        .name("xchonnect-request".to_owned())
        .spawn(move || {
            let outcome = run(
                &state, &store, &approvals, &snapshot, &record, &meta, &params, now,
            );
            let _ = tx.send(outcome);
        });
    if let Err(error) = thread {
        tracing::error!("could not start the request thread: {error}");
        return Err(rpc_error(codes::INVALID_PARAMS, "the wallet is busy"));
    }
    match rx.await {
        Ok(outcome) => outcome,
        Err(_) => Err(rpc_error(
            codes::USER_REJECTED,
            "the wallet could not finish this request",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    state: &AppState,
    store: &Arc<SessionStore>,
    approvals: &Arc<ApprovalBridge>,
    snapshot: &WalletSnapshot,
    record: &SessionRecord,
    meta: &RequestMeta,
    params: &str,
    now: u64,
) -> std::result::Result<String, RpcError> {
    let permissions = record.permissions.to_kit();
    let limits = PersistentLimits::new(
        store.clone(),
        record.fingerprint,
        record.dapp.clone(),
        record.id.clone(),
    );
    let approver = UiApprover::new(approvals.clone(), meta.clone());
    // One approval per request, one key unlock per request (xchonnect-wiki 04).
    let signer = KlimperSigner::new(state.clone(), snapshot);
    let broadcaster = SageBroadcaster::new(state.clone());
    // The optional reads and broadcast (TASK-082), scoped by wallet-kit to the keys this
    // dApp was given.
    let chain = crate::chain::KlimperChain::new(state.clone());

    // What the dApp builds itself may only be signed with the keys it was given: a key of
    // the wallet's it does not know then looks exactly like a stranger's, so a guessed key
    // cannot be confirmed by whether a prompt appears. The send Klimper builds from its own
    // coins (chia_send, chia_takeOffer) are the requests that sign with every key of the wallet.
    let exposed: std::collections::HashSet<chia_bls::PublicKey> = permissions
        .exposed_keys
        .iter()
        .filter(|key| snapshot.keys.contains(key))
        .copied()
        .collect();
    let wallet_built = execute::is_wallet_built(canonical_method(&meta.method));
    let context = RequestContext {
        dapp: &record.dapp,
        network: snapshot.network.clone(),
        session_chain_id: &record.chain_id,
        permissions: &permissions,
        allow_agg_sig_unsafe: record.allow_agg_sig_unsafe,
        allow_unknown_contracts: record.allow_unknown_contracts,
        ownership: &snapshot.ownership,
        keys: if wallet_built {
            &snapshot.keys
        } else {
            &exposed
        },
        limits: &limits,
        now,
        chain: Some(&chain),
    };
    let host = Host {
        signer: &signer,
        approver: &approver,
        broadcaster: Some(&broadcaster),
    };
    let outcome = match canonical_method(&meta.method) {
        // The requests Klimper builds from its own coins: one gate for all of them.
        method @ (execute::CHIA_SEND | execute::CHIA_TAKE_OFFER) => {
            if !permissions.allows_method(method) {
                // Most often a connection paired before the method existed: the reason lets
                // the website say "pair again" instead of a bare refusal.
                Err(RpcError {
                    code: codes::UNAUTHORIZED,
                    message: "this connection may not use this request; pair again".to_owned(),
                    data: Some(serde_json::json!({ "reason": "not_granted" }).to_string()),
                })
            } else if approvals.paused(&record.id) {
                // Refused before anything is built: whether a paused website's request could
                // be built would tell it the wallet's balance without a prompt.
                Err(rejected())
            } else {
                approver.mark_cross_checked();
                let build = if method == execute::CHIA_SEND {
                    execute::send
                } else {
                    execute::take_offer
                };
                build(state, &context, &signer, &approver, &broadcaster, params)
            }
        }
        kit::SUBMIT_COIN_SPENDS => {
            // A dApp-built bundle: the wallet database's reading and the simulation must
            // agree before the user is asked anything.
            pre_check_submit(state, &context, params).and_then(|()| {
                approver.mark_cross_checked();
                kit::handle_with_host(&meta.method, params, &context, &host)
            })
        }
        _ => kit::handle_with_host(&meta.method, params, &context, &host),
    };
    // A transaction Klimper broadcast: the webview shows where to look it up.
    if let Ok(result) = &outcome
        && matches!(
            canonical_method(&meta.method),
            execute::CHIA_SEND | execute::CHIA_TAKE_OFFER | kit::SUBMIT_COIN_SPENDS
        )
        && let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(result)
        && let Some(Value::String(transaction_id)) = fields.get("transactionId")
    {
        if let Ok(bytes) = hex::decode(transaction_id.trim_start_matches("0x"))
            && let Ok(id) = <[u8; 32]>::try_from(bytes.as_slice())
        {
            meta.report(Progress::Broadcast(id));
        }
        approvals.sent(serde_json::json!({
            "dapp": record.dapp,
            "method": meta.method,
            "transactionId": transaction_id,
            "status": fields.get("status").cloned().unwrap_or(Value::Null),
        }));
    }
    match &outcome {
        Ok(_) => tracing::info!(
            dapp = %record.dapp,
            method = %meta.method,
            signatures = signer.produced(),
            "answered an xchonnect request"
        ),
        // The code and the fixed message only; `data` can name a reason but never request
        // content, and nothing here logs params.
        Err(error) => tracing::info!(
            dapp = %record.dapp,
            method = %meta.method,
            code = error.code,
            "refused an xchonnect request"
        ),
    }
    outcome
}

/// For `xchonnect_submitCoinSpends`: parse the spends and compare Sage's summary with
/// wallet-kit's simulation. Malformed params are left to wallet-kit to refuse.
fn pre_check_submit(
    state: &AppState,
    context: &RequestContext<'_>,
    params: &str,
) -> std::result::Result<(), RpcError> {
    let Ok(value) = serde_json::from_str::<Value>(params) else {
        return Ok(());
    };
    let Ok(spends) = kit::parse_coin_spends(value.get("coinSpends").unwrap_or(&Value::Null)) else {
        return Ok(());
    };
    execute::check_spends(state, context, &spends)
}

/// Deadline for a prompt: the request's own expiry, or the bridge's cap, whichever is
/// sooner.
pub fn prompt_deadline(now: u64, request_exp: u64) -> u64 {
    let cap = now.saturating_add(DECISION_TIMEOUT.as_secs());
    if request_exp == 0 {
        cap
    } else {
        request_exp.min(cap)
    }
}

/// A CHIP-0002 rejection for an approval the user never saw, e.g. because no wallet is
/// open.
pub fn rejected() -> RpcError {
    rpc_error(codes::USER_REJECTED, "user rejected request")
}

impl From<Error> for RpcError {
    fn from(error: Error) -> Self {
        match error {
            Error::NoSigningKey | Error::NoWallet => {
                rpc_error(codes::NO_SECRET_KEY, "the wallet cannot sign")
            }
            Error::SessionNotUsable => {
                rpc_error(codes::UNAUTHORIZED, "this connection is not confirmed")
            }
            other => {
                tracing::warn!("xchonnect request failed: {other}");
                rpc_error(
                    codes::INVALID_PARAMS,
                    "the wallet could not run this request",
                )
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::store::{StoredPermissions, StoredSession};

    fn record() -> SessionRecord {
        SessionRecord {
            id: "s1".to_owned(),
            fingerprint: 1,
            dapp: "pengui.xyz".to_owned(),
            dapp_name: "Pengui".to_owned(),
            relay: "https://relay.example".to_owned(),
            chain_id: "testnet11".to_owned(),
            created_at: 0,
            state_version: 1,
            developer_mode: false,
            sas_confirmed: false,
            needs_repair: false,
            permissions: StoredPermissions::default(),
            allow_agg_sig_unsafe: false,
            allow_unknown_contracts: false,
            origin_kid: None,
            revoked: false,
            push: None,
            origin_checked_at: None,
            own_key: None,
        }
    }

    #[test]
    fn a_wrong_network_refusal_names_both_networks() {
        let error = wrong_network("testnet11", "mainnet");
        assert_eq!(error.code, codes::UNAUTHORIZED);
        let data: serde_json::Value = serde_json::from_str(error.data.as_deref().unwrap()).unwrap();
        assert_eq!(data["reason"], "wrong_network");
        assert_eq!(data["walletChainId"], "testnet11");
        assert_eq!(data["sessionChainId"], "mainnet");
    }

    /// A session that reached `session.ready` but whose SAS was never confirmed must not
    /// answer anything (the test the security brief asks for; see also
    /// `session_loop::tests::a_session_without_sas_confirmation_cannot_be_used`).
    #[test]
    fn an_unconfirmed_session_answers_nothing() {
        let record = record();
        let error = check_usable(&record).unwrap_err();
        assert_eq!(error.code, codes::UNAUTHORIZED);

        let mut confirmed = record.clone();
        confirmed.sas_confirmed = true;
        assert!(check_usable(&confirmed).is_ok());

        // Rolled-back state is just as unusable, confirmed SAS or not.
        let mut rolled_back = confirmed.clone();
        rolled_back.needs_repair = true;
        assert_eq!(
            check_usable(&rolled_back).unwrap_err().code,
            codes::UNAUTHORIZED
        );

        // So is a session whose origin key the dApp withdrew (spec 6.1).
        let mut revoked = confirmed;
        revoked.revoked = true;
        assert_eq!(
            check_usable(&revoked).unwrap_err().code,
            codes::UNAUTHORIZED
        );
    }

    /// Records written before the origin `kid` and the revocation mark existed still load,
    /// as unchecked and not revoked.
    #[test]
    fn an_older_record_loads_without_the_new_fields() {
        let mut value = serde_json::to_value(record()).unwrap();
        let fields = value.as_object_mut().unwrap();
        fields.remove("origin_kid");
        fields.remove("revoked");
        let old: SessionRecord = serde_json::from_value(value).unwrap();
        assert_eq!(old.origin_kid, None);
        assert!(!old.revoked);
    }

    #[test]
    fn a_prompt_deadline_never_exceeds_the_cap() {
        let now = 1_790_000_000;
        assert_eq!(prompt_deadline(now, 0), now + DECISION_TIMEOUT.as_secs());
        assert_eq!(prompt_deadline(now, now + 10), now + 10);
        assert_eq!(
            prompt_deadline(now, now + 100_000),
            now + DECISION_TIMEOUT.as_secs()
        );
    }

    #[test]
    fn the_stored_record_is_what_the_context_is_built_from() {
        // A record round-trips through storage without changing what the dApp may do.
        let mut stored = StoredSession {
            record: record(),
            session_hex: String::new(),
        };
        stored.record.permissions.methods = vec!["signCoinSpends".to_owned()];
        let perms = stored.record.permissions.to_kit();
        assert!(perms.allows_method("chip0002_signCoinSpends"));
        assert!(!perms.allows_method("signMessage"));
    }
}
