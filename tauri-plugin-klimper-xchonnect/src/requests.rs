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
//! This module supplies the four things it needs from Klimper and nothing else: the
//! request context, the approval UI, the signer and the limit store. Two checks are made
//! here, before `handle` is reached:
//!
//! * **The SAS gate.** A session whose six digits the user never confirmed, or whose
//!   stored state was rolled back, answers nothing at all (spec 6.3 step 8, 5.3).
//! * **The network.** A session approved for another chain is refused before any prompt
//!   is shown, for every method and not only for `signCoinSpends` (TASK-079 AC #2).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use xchonnect_core::rpc::codes;
use xchonnect_wallet_kit::{self as kit, Approver, Prompt, RequestContext, RpcError};

use crate::approval::{Approval, ApprovalBridge, ApprovalKind, DECISION_TIMEOUT};
use crate::error::Error;
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
pub fn check_usable(record: &SessionRecord) -> std::result::Result<(), RpcError> {
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

/// Identity of the request, used to label its approvals.
#[derive(Debug, Clone)]
pub struct RequestMeta {
    pub session_id: String,
    pub dapp: String,
    pub method: String,
    /// Unix seconds after which the dApp will no longer accept an answer.
    pub expires_at: u64,
}

/// Approval UI as wallet-kit's `Approver`. Records the size of the signing plan so each
/// per-signature prompt can say "2 of 3".
pub struct UiApprover {
    bridge: Arc<ApprovalBridge>,
    meta: RequestMeta,
    plan_len: Arc<AtomicUsize>,
}

impl std::fmt::Debug for UiApprover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiApprover")
            .field("dapp", &self.meta.dapp)
            .finish_non_exhaustive()
    }
}

impl UiApprover {
    /// `plan_len` is shared with the per-signature gate, which reads it to label its
    /// prompts.
    pub fn new(bridge: Arc<ApprovalBridge>, meta: RequestMeta, plan_len: Arc<AtomicUsize>) -> Self {
        UiApprover {
            bridge,
            meta,
            plan_len,
        }
    }
}

impl Approver for UiApprover {
    fn approve(&self, prompt: &Prompt<'_>) -> bool {
        if let Prompt::SignCoinSpends { plan, .. } = prompt {
            self.plan_len.store(plan.ours.len(), Ordering::SeqCst);
        } else {
            self.plan_len.store(1, Ordering::SeqCst);
        }
        // The prompt is serialised exactly as wallet-kit built it from the local
        // simulation. Nothing the dApp wrote is passed to the screen.
        let Ok(payload) = serde_json::to_value(prompt) else {
            tracing::error!("an approval prompt could not be serialised; refusing");
            return false;
        };
        self.bridge.ask(Approval {
            id: new_id(),
            session_id: self.meta.session_id.clone(),
            dapp: self.meta.dapp.clone(),
            method: self.meta.method.clone(),
            expires_at: self.meta.expires_at,
            kind: ApprovalKind::Request { prompt: payload },
        })
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
        return Err(RpcError {
            code: codes::UNAUTHORIZED,
            message: "this connection was approved for another network".to_owned(),
            data: Some(r#"{"reason":"wrong_network"}"#.to_owned()),
        });
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
    let limits = PersistentLimits::new(store.clone(), record.fingerprint, record.id.clone());
    let plan_len = Arc::new(AtomicUsize::new(0));
    let approver = UiApprover::new(approvals.clone(), meta.clone(), plan_len.clone());

    // One confirmation per individual signature, on top of the approval of the request as
    // a whole (spec 11.1 item 4; see `crate::wallet` for what this is on desktop).
    let gate = {
        let bridge = approvals.clone();
        let meta = meta.clone();
        let plan_len = plan_len.clone();
        Arc::new(move |index: usize, public_key: &chia_bls::PublicKey| {
            bridge.ask(Approval {
                id: new_id(),
                session_id: meta.session_id.clone(),
                dapp: meta.dapp.clone(),
                method: meta.method.clone(),
                expires_at: meta.expires_at,
                kind: ApprovalKind::Signature {
                    index,
                    total: plan_len.load(Ordering::SeqCst),
                    public_key: format!("0x{}", hex::encode(public_key.to_bytes())),
                },
            })
        })
    };
    let signer = KlimperSigner::new(state.clone(), snapshot, gate);

    let context = RequestContext {
        dapp: &record.dapp,
        network: snapshot.network.clone(),
        session_chain_id: &record.chain_id,
        permissions: &permissions,
        allow_agg_sig_unsafe: record.allow_agg_sig_unsafe,
        allow_unknown_contracts: record.allow_unknown_contracts,
        ownership: &snapshot.ownership,
        keys: &snapshot.keys,
        limits: &limits,
        now,
    };
    let outcome = kit::handle(&meta.method, params, &context, &signer, &approver);
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

/// A CHIP-0002 rejection for an approval the user never saw, e.g. because the wallet is
/// locked.
pub fn rejected() -> RpcError {
    rpc_error(codes::USER_REJECTED, "user rejected request")
}

impl From<Error> for RpcError {
    fn from(error: Error) -> Self {
        match error {
            Error::WalletLocked | Error::NoWallet => {
                rpc_error(codes::NO_SECRET_KEY, "the wallet is locked")
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
        }
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
