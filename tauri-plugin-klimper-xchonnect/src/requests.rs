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
//!
//! And for spends a website built, one more before wallet-kit sees them: a spend whose
//! conditions the signature would not fix is refused ([`crate::sign_guard`], TASK-141).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde_json::Value;
use xchonnect_core::rpc::{canonical_method, codes};
use xchonnect_wallet_kit::{self as kit, Approver, Host, Prompt, RequestContext, RpcError};

use crate::approval::{Approval, ApprovalBridge, ApprovalKind, DECISION_TIMEOUT};
use crate::error::Error;
use crate::execute::{self, SageBroadcaster};
use crate::sign_guard;
use crate::signed_coins::{self, SignedCoinsHandle};
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
    /// For a take: how many offers its bundle takes together (0: not a take). Counted by
    /// the wallet from the decoded offer (TASK-139).
    offers: AtomicUsize,
    /// For a loan: who lends what to whom, and whether the request only signs an offer
    /// (a borrower's loan request) instead of sending (TASK-140). Computed by the wallet.
    loan: std::sync::Mutex<Option<(Value, bool)>>,
    /// For an offer the wallet makes (`chia_createOffer`, TASK-142): what it gives, options
    /// with their terms, and what it asks. Computed by the wallet from its own build.
    offer: std::sync::Mutex<Option<Value>>,
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
            offers: AtomicUsize::new(0),
            loan: std::sync::Mutex::new(None),
            offer: std::sync::Mutex::new(None),
        }
    }

    /// Record the offer on screen: nothing is sent, an offer is signed.
    pub fn mark_offer(&self, offer: &execute::create_offer::OfferView) {
        if let (Ok(value), Ok(mut slot)) = (serde_json::to_value(offer), self.offer.lock()) {
            *slot = Some(value);
        }
    }

    /// Record the loan on screen; `makes_offer`: nothing is sent, an offer is signed.
    pub fn mark_loan(&self, loan: &execute::options::LoanView, makes_offer: bool) {
        if let (Ok(value), Ok(mut slot)) = (serde_json::to_value(loan), self.loan.lock()) {
            *slot = Some((value, makes_offer));
        }
    }

    /// Record how many offers the take on screen takes together.
    pub fn mark_offers(&self, count: usize) {
        self.offers.store(count, Ordering::SeqCst);
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
        // The trade overview, computed here from the same simulation (TASK-139): what is
        // paid, what is received, the user's fee, the price, how many offers. Klimper's
        // own key next to `cross_checked`; nothing in it comes from the website.
        let offers = Some(self.offers.load(Ordering::SeqCst)).filter(|n| *n > 0);
        if let Some(trade) = crate::trade::of_prompt(prompt, offers)
            && let Ok(trade) = serde_json::to_value(trade)
            && let Some(fields) = payload.as_object_mut()
        {
            fields.insert("trade".to_owned(), trade);
        }
        // A loan (TASK-140), Klimper's own key like `trade`: the principal and the other
        // side, from the wallet's own build or the simulation, never from the website.
        if let Ok(slot) = self.loan.lock()
            && let Some((loan, makes_offer)) = slot.as_ref()
            && let Some(fields) = payload.as_object_mut()
        {
            fields.insert("loan".to_owned(), loan.clone());
            if *makes_offer {
                fields.insert("makes_offer".to_owned(), Value::Bool(true));
            }
        }
        // An offer the wallet makes (TASK-142), Klimper's own key like `loan`.
        if let Ok(slot) = self.offer.lock()
            && let Some(offer) = slot.as_ref()
            && let Some(fields) = payload.as_object_mut()
        {
            fields.insert("offer".to_owned(), offer.clone());
            fields.insert("makes_offer".to_owned(), Value::Bool(true));
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
    // The record of the signatures outside the wallet (xchonnect spec 11.2): what decides
    // whether a request Klimper builds and broadcasts is sealed, and which signatures the
    // wallet never makes again. Requests that sign coin spends run one at a time, from
    // reading the record to writing what they release.
    let signs = wallet_built
        || matches!(
            canonical_method(&meta.method),
            kit::SUBMIT_COIN_SPENDS | SIGN_COIN_SPENDS | SIGN_MESSAGE
        );
    let record_of_signed = SignedCoinsHandle::new(
        store.clone(),
        record.fingerprint,
        snapshot.network.chain_id(),
    );
    // Held until this function returns: across wallet-kit's check of the record, the
    // approval prompt, the signature and what is recorded of it.
    let (_one_at_a_time, signed) =
        signed_coins::checked_record(&record_of_signed, signs, |handle| {
            signed_coins::refresh(state, handle)
        })
        .map_err(|error| {
            tracing::error!("xchonnect: the record of signed coins could not be read: {error}");
            rpc_error(
                codes::INVALID_PARAMS,
                "the wallet could not run this request",
            )
        })?;
    // Whatever Klimper broadcasts, wallet-kit first reports what goes into the record
    // (Broadcaster::record): written before anything is sent, or nothing is sent.
    let broadcaster = SageBroadcaster::new(state.clone(), &record_of_signed);
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
        signed_coins: &signed,
    };
    let host = Host {
        signer: &signer,
        approver: &approver,
        broadcaster: Some(&broadcaster),
    };
    let outcome = match canonical_method(&meta.method) {
        // The requests Klimper builds from its own coins: one gate for all of them.
        method if execute::is_wallet_built(method) => {
            if !permissions.allows_method(method) {
                // Most often a connection paired before the method existed, or one the user
                // never turned option contracts on for: the reason lets the website say "pair
                // again" or "allow it in Klimper" instead of a bare refusal.
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
                let on_loan = |loan: &execute::options::LoanView, makes_offer: bool| {
                    approver.mark_loan(loan, makes_offer);
                };
                match method {
                    execute::CHIA_SEND => {
                        execute::send(state, &context, &signer, &approver, &broadcaster, params)
                    }
                    execute::CHIA_TAKE_OFFER => execute::take_offer(
                        state,
                        &context,
                        &signer,
                        &approver,
                        &broadcaster,
                        params,
                        &|count| approver.mark_offers(count),
                        &on_loan,
                    ),
                    execute::CHIA_CREATE_OFFER => execute::create_offer::create_offer(
                        state,
                        &context,
                        &signer,
                        &approver,
                        params,
                        &|offer| approver.mark_offer(offer),
                    ),
                    execute::options::CHIA_MINT_OPTION => execute::options::mint_option(
                        state,
                        &context,
                        &signer,
                        &approver,
                        &broadcaster,
                        params,
                        &on_loan,
                    ),
                    execute::options::CHIA_EXERCISE_OPTION => execute::options::exercise_option(
                        state,
                        &context,
                        &signer,
                        &approver,
                        &broadcaster,
                        params,
                    ),
                    execute::options::CHIA_CLAIM_OPTION => execute::options::claim_option(
                        state,
                        &context,
                        &signer,
                        &approver,
                        &broadcaster,
                        params,
                    ),
                    _ => execute::options::transfer_option(
                        state,
                        &context,
                        &signer,
                        &approver,
                        &broadcaster,
                        params,
                    ),
                }
            }
        }
        kit::SUBMIT_COIN_SPENDS => {
            // A dApp-built bundle: the wallet database's reading and the simulation must
            // agree before the user is asked anything.
            sign_guard::check_request(&meta.method, params, context.keys, &context.network)
                .and_then(|()| pre_check_submit(state, &context, params))
                .and_then(|()| {
                    approver.mark_cross_checked();
                    kit::handle_with_host(&meta.method, params, &context, &host)
                })
        }
        _ => handle_dapp_built(&meta.method, params, &context, &host),
    };
    // Signatures that go back to the website are recorded before they leave: if that fails,
    // they do not leave.
    let outcome = outcome.and_then(|result| {
        record_released(
            canonical_method(&meta.method),
            &result,
            &snapshot.ownership,
            &record_of_signed,
        )
        .map(|()| result)
    });
    let outcome = outcome.map_err(explain);
    // An agent's offer is kept in the wallet (TASK-144): its coins are locked now.
    if outcome.is_ok() && canonical_method(&meta.method) == execute::CHIA_CREATE_OFFER {
        approvals.coins_changed();
    }
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

/// What a website is told for the refusals of the record of signed coins (xchonnect spec
/// 11.2), of a contract in a multi-party request Klimper cannot read, and of the option
/// limit (spec 11.1 item 5), in words it can show its user. The code and reason stay
/// wallet-kit's.
pub fn explain(mut error: RpcError) -> RpcError {
    let reason = error
        .data
        .as_deref()
        .and_then(|data| serde_json::from_str::<Value>(data).ok())
        .and_then(|data| Some(data.get("reason")?.as_str()?.to_owned()));
    match reason.as_deref() {
        Some("sealed_signature") => {
            "Klimper already signed this exact spend for a transaction it sent, and never \
             signs it again. Wait for that transaction to confirm, or build a new one."
                .clone_into(&mut error.message);
        }
        Some("record_failed") => {
            "Klimper could not save its record of this transaction, so nothing was sent. \
             Try again."
                .clone_into(&mut error.message);
        }
        Some("unknown_partial") => {
            "This trade has Klimper sign a contract it cannot read (an NFT, a vault or \
             another wrapped coin), so it cannot check what the other side gets. Klimper \
             does not sign it."
                .clone_into(&mut error.message);
        }
        Some("options_per_request") => {
            "This request sends more options out of the wallet than Klimper allows this \
             website in one request. The user can change the option limit in the \
             website's settings in Klimper."
                .clone_into(&mut error.message);
        }
        Some("options_per_day") => {
            "This request would send more options out of the wallet today than Klimper \
             allows this website per day. Try again tomorrow (UTC), or the user can change \
             the option limit in the website's settings in Klimper."
                .clone_into(&mut error.message);
        }
        _ => {}
    }
    error
}

/// CHIP-0002 `signCoinSpends`, without the `chip0002_` prefix.
const SIGN_COIN_SPENDS: &str = "signCoinSpends";

/// CHIP-0002 `signMessage`, without the `chip0002_` prefix: checked against the record too.
const SIGN_MESSAGE: &str = "signMessage";

/// The coins whose signatures an answer hands to the website (xchonnect spec 11.2) that
/// wallet-kit does not report itself: every coin of the user's in a signed offer Klimper
/// built (a loan request). A `signCoinSpends` answer and every bundle Klimper broadcasts are
/// recorded through its broadcaster ([`SageBroadcaster`], wallet-kit's
/// `Broadcaster::record`) before the signature leaves.
pub fn record_released(
    method: &str,
    result: &str,
    ownership: &kit::Ownership,
    record: &SignedCoinsHandle,
) -> std::result::Result<(), RpcError> {
    let spends = if execute::is_wallet_built(method) {
        serde_json::from_str::<Value>(result)
            .ok()
            .and_then(|value| Some(value.get("offer")?.as_str()?.to_owned()))
            .and_then(|offer| chia_wallet_sdk::driver::decode_offer(&offer).ok())
            .map(|bundle| bundle.coin_spends)
    } else {
        None
    };
    let Some(spends) = spends else {
        return Ok(());
    };
    record
        .release(signed_coins::user_coins(&spends, ownership))
        .map_err(|error| {
            tracing::error!("xchonnect: the record of signed coins could not be written: {error}");
            rpc_error(
                codes::INVALID_PARAMS,
                "the wallet could not run this request",
            )
        })
}

/// A request whose spends, if any, the website built: refused before the user is asked if
/// the signature would not fix a spend's conditions ([`sign_guard`], TASK-141), then
/// answered by wallet-kit.
pub fn handle_dapp_built(
    method: &str,
    params: &str,
    context: &RequestContext<'_>,
    host: &Host<'_>,
) -> std::result::Result<String, RpcError> {
    sign_guard::check_request(method, params, context.keys, &context.network)?;
    kit::handle_with_host(method, params, context, host)
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
            methods_revision: 0,
            own_key: None,
        }
    }

    /// No connection is upgraded to the option methods, whatever it may do: they lock funds
    /// until a date, so the user turns them on for a website (TASK-140).
    #[test]
    fn an_older_connection_never_gets_the_option_methods_by_itself() {
        use crate::execute::options::OPTION_METHODS;
        use crate::execute::{CHIA_SEND, CHIA_TAKE_OFFER, upgrade_methods};
        let mut old = record();
        old.permissions.methods = vec![CHIA_SEND.to_owned(), CHIA_TAKE_OFFER.to_owned()];
        upgrade_methods(&mut old);
        assert!(
            OPTION_METHODS
                .iter()
                .all(|m| !old.permissions.methods.iter().any(|g| g == m))
        );
        assert!(
            OPTION_METHODS
                .iter()
                .all(|m| crate::execute::is_wallet_built(m))
        );
    }

    /// A connection paired before `chia_takeOffer` existed gets it once, next to its
    /// `chia_send`; one whose `chia_send` was turned off gets nothing; a take the user turns
    /// off afterwards stays off (TASK-137).
    #[test]
    fn an_older_connection_may_take_offers_once_if_it_may_send() {
        use crate::execute::{CHIA_SEND, CHIA_TAKE_OFFER, METHODS_REVISION, upgrade_methods};
        let mut old = record();
        old.permissions.methods = vec!["signCoinSpends".to_owned(), CHIA_SEND.to_owned()];
        assert!(upgrade_methods(&mut old));
        assert_eq!(old.methods_revision, METHODS_REVISION);
        assert_eq!(
            old.permissions.methods,
            [
                "signCoinSpends",
                CHIA_SEND,
                CHIA_TAKE_OFFER,
                crate::execute::CHIA_CREATE_OFFER
            ]
        );
        assert!(!upgrade_methods(&mut old), "only once");

        old.permissions.methods.retain(|m| m != CHIA_TAKE_OFFER);
        assert!(!upgrade_methods(&mut old));
        assert!(!old.permissions.methods.iter().any(|m| m == CHIA_TAKE_OFFER));

        let mut no_send = record();
        no_send.permissions.methods = vec!["signCoinSpends".to_owned()];
        assert!(!upgrade_methods(&mut no_send));
        assert_eq!(no_send.permissions.methods, ["signCoinSpends"]);
        assert_eq!(no_send.methods_revision, METHODS_REVISION);
    }

    /// A connection that already had `chia_takeOffer` (revision 1) is given `chia_createOffer`
    /// once, if it may send; a take the user turned off stays off (TASK-142).
    #[test]
    fn a_revision_one_connection_may_make_offers_once_if_it_may_send() {
        use crate::execute::{CHIA_CREATE_OFFER, CHIA_SEND, CHIA_TAKE_OFFER, upgrade_methods};
        let mut old = record();
        old.methods_revision = 1;
        old.permissions.methods = vec![CHIA_SEND.to_owned()];
        assert!(upgrade_methods(&mut old));
        assert_eq!(old.permissions.methods, [CHIA_SEND, CHIA_CREATE_OFFER]);
        assert!(!old.permissions.methods.iter().any(|m| m == CHIA_TAKE_OFFER));
        assert!(!upgrade_methods(&mut old), "only once");
        assert!(crate::execute::is_wallet_built(CHIA_CREATE_OFFER));
    }

    /// A record stored before the revision was kept reads as revision 0.
    #[test]
    fn a_record_without_a_methods_revision_reads_as_zero() {
        let mut json = serde_json::to_value(record()).unwrap();
        json.as_object_mut().unwrap().remove("methods_revision");
        let back: SessionRecord = serde_json::from_value(json).unwrap();
        assert_eq!(back.methods_revision, 0);
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

    /// The round-5 refusals reach the website in words it can show, code and reason kept.
    #[test]
    fn the_new_refusals_are_explained() {
        for (code, reason, words) in [
            (codes::UNAUTHORIZED, "unknown_partial", "cannot read"),
            (codes::LIMIT_EXCEEDED, "options_per_request", "option limit"),
            (codes::LIMIT_EXCEEDED, "options_per_day", "option limit"),
        ] {
            let data = serde_json::json!({ "reason": reason }).to_string();
            let error = explain(RpcError {
                code,
                message: "x".to_owned(),
                data: Some(data.clone()),
            });
            assert!(error.message.contains(words), "{reason}: {}", error.message);
            assert_eq!((error.code, error.data), (code, Some(data)));
        }
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
