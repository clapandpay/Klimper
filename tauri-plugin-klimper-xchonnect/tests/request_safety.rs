// SPDX-License-Identifier: AGPL-3.0-or-later
//! The safety properties a dApp request has to satisfy, driven through Klimper's own
//! approval bridge and limit store (clapandpay TASK-079 AC #5).
//!
//! The spends are the ones `xchonnect-wallet-kit`'s own fixtures use — a standard send
//! and an atomic two-party XCH swap through offer settlement payments — rebuilt here
//! because those fixtures are `#[cfg(test)]` inside that crate. What is under test is the
//! wiring: that an approval really has to come back from the webview before anything is
//! signed, that a refusal reaches the user as a CHIP-0002 error, and that the three
//! refusals which must happen *before* any prompt really do.
// Test-only relaxations: the harness is deliberately a few structs with closures in them.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::type_complexity,
    clippy::unused_self,
    clippy::needless_pass_by_value
)]

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chia_bls::{PublicKey, SecretKey, Signature};
use chia_protocol::{Bytes32, Coin, CoinSpend, Program, SpendBundle};
use chia_puzzle_types::Memos;
use chia_puzzle_types::offer::{NotarizedPayment, Payment, SettlementPaymentsSolution};
use chia_puzzles::SETTLEMENT_PAYMENT_HASH;
use chia_sdk_driver::{Layer, SettlementLayer, SpendContext, StandardLayer};
use chia_sdk_test::{BlsPair, Simulator};
use chia_sdk_types::Conditions;
use clvm_traits::{FromClvm, ToClvm};
use clvm_utils::tree_hash;
use clvmr::Allocator;
use serde_json::{Value, json};
use xchonnect_core::crypto::sha256_parts;
use xchonnect_core::rpc::codes;
use xchonnect_wallet_kit::{
    self as kit, AssetId, AssetLimit, Broadcast, Broadcaster, DailySpend, DappPermissions, Host,
    LimitStore, Network, Ownership, RequestContext, RpcError, Signer, SignerError,
};

use tauri_plugin_klimper_xchonnect::approval::{Approval, ApprovalBridge, ApprovalKind, Emitter};
use tauri_plugin_klimper_xchonnect::error::Result as KitResult;
use tauri_plugin_klimper_xchonnect::requests::{Progress, ProgressSender, RequestMeta, UiApprover};
use tauri_plugin_klimper_xchonnect::store::{
    MemorySecrets, PersistentLimits, SecretStore, SessionStore, StoredPermissions,
};

const NOW: u64 = 1_790_000_000;
const DAPP: &str = "pengui.xyz";

// ---------------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------------

/// Answers every approval with `approve`, and records what was shown.
struct Webview {
    bridge: Arc<ApprovalBridge>,
    shown: Arc<Mutex<Vec<Approval>>>,
    stop: Arc<Mutex<bool>>,
}

impl Webview {
    fn new(approve: bool) -> Self {
        let shown: Arc<Mutex<Vec<Approval>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = shown.clone();
        let emit: Emitter = Arc::new(move |event, payload| {
            if event.ends_with("approval")
                && let Ok(approval) = serde_json::from_value::<Approval>(payload)
            {
                sink.lock().unwrap().push(approval);
            }
        });
        let bridge = Arc::new(ApprovalBridge::new(emit));
        let stop = Arc::new(Mutex::new(false));
        // A background "user": answers anything that starts waiting.
        {
            let bridge = bridge.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !*stop.lock().unwrap() {
                    for approval in bridge.outstanding() {
                        let _ = bridge.resolve(&approval.id, approve);
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            });
        }
        Webview {
            bridge,
            shown,
            stop,
        }
    }

    fn prompts(&self) -> Vec<Approval> {
        self.shown.lock().unwrap().clone()
    }
}

impl Drop for Webview {
    fn drop(&mut self) {
        *self.stop.lock().unwrap() = true;
    }
}

/// Counts signatures and batches, and can be made faulty (`.faulty` returns a signature
/// by an unrelated key) or cancelled (`.cancel`: the user dismissed the authentication).
struct CountingSigner {
    secret: SecretKey,
    public: PublicKey,
    calls: AtomicUsize,
    batches: AtomicUsize,
    faulty: bool,
    cancel: bool,
}

impl CountingSigner {
    fn new(who: &BlsPair) -> Self {
        CountingSigner {
            secret: who.sk.clone(),
            public: who.pk,
            calls: AtomicUsize::new(0),
            batches: AtomicUsize::new(0),
            faulty: false,
            cancel: false,
        }
    }
}

impl Signer for CountingSigner {
    fn sign_all(&self, requests: &[(&PublicKey, &[u8])]) -> Result<Vec<Signature>, SignerError> {
        self.batches.fetch_add(1, Ordering::SeqCst);
        if self.cancel {
            return Err(SignerError::Cancelled);
        }
        requests
            .iter()
            .map(|(pk, msg)| self.sign(pk, msg))
            .collect()
    }

    fn sign(&self, public_key: &PublicKey, message: &[u8]) -> Result<Signature, SignerError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if *public_key != self.public {
            return Err(SignerError::KeyUnavailable);
        }
        if self.faulty {
            return Ok(chia_bls::sign(&BlsPair::new(99).sk, message));
        }
        Ok(chia_bls::sign(&self.secret, message))
    }
}

struct Harness {
    webview: Webview,
    store: Arc<SessionStore>,
    _dir: tempfile::TempDir,
    permissions: DappPermissions,
    ownership: Ownership,
    keys: HashSet<PublicKey>,
    who: BlsPair,
    faulty: bool,
    /// Where the approver reports progress, as the session loop would receive it.
    progress: Option<ProgressSender>,
}

impl Harness {
    fn new(who: BlsPair, approve: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Arc::new(SessionStore::open(dir.path(), Box::new(MemorySecrets::default())).unwrap());
        Harness {
            webview: Webview::new(approve),
            store,
            _dir: dir,
            permissions: DappPermissions::new_default(who.pk),
            ownership: Ownership {
                p2_puzzle_hashes: [who.puzzle_hash].into_iter().collect(),
            },
            keys: [who.pk].into_iter().collect(),
            who,
            faulty: false,
            progress: None,
        }
    }

    fn meta(&self, method: &str) -> RequestMeta {
        RequestMeta {
            session_id: "s1".to_owned(),
            dapp: DAPP.to_owned(),
            method: method.to_owned(),
            request_id: "r1".to_owned(),
            expires_at: NOW + 60,
            progress: self.progress.clone(),
        }
    }

    /// Run one request the way `requests::answer` does: the same approver and the same
    /// persistent limit store, through `handle_with_host` with a broadcaster that is a
    /// simulated chain. Returns the outcome, signatures made and signing batches.
    fn run(
        &self,
        method: &str,
        params: &str,
        chain: &SimChain,
    ) -> (Result<String, RpcError>, usize, usize) {
        let approver = UiApprover::new(self.webview.bridge.clone(), self.meta(method));
        let mut signer = CountingSigner::new(&self.who);
        signer.faulty = self.faulty;
        let limits = PersistentLimits::new(self.store.clone(), 1, "s1".to_owned());
        let context = RequestContext {
            dapp: DAPP,
            network: Network::Testnet11,
            session_chain_id: "testnet11",
            permissions: &self.permissions,
            allow_agg_sig_unsafe: false,
            allow_unknown_contracts: false,
            ownership: &self.ownership,
            keys: &self.keys,
            limits: &limits,
            now: NOW,
        };
        let host = Host {
            signer: &signer,
            approver: &approver,
            broadcaster: Some(chain),
        };
        let outcome = kit::handle_with_host(method, params, &context, &host);
        (
            outcome,
            signer.calls.load(Ordering::SeqCst),
            signer.batches.load(Ordering::SeqCst),
        )
    }

    fn call(&self, method: &str, params: &str) -> (Result<String, RpcError>, usize) {
        let (outcome, calls, _) = self.run(method, params, &SimChain::default());
        (outcome, calls)
    }
}

/// A broadcaster that is a simulated chain: what it accepts, a full node would.
#[derive(Default)]
struct SimChain {
    sim: Mutex<Simulator>,
    accepted: Mutex<Vec<SpendBundle>>,
}

impl SimChain {
    fn on(sim: Simulator) -> Self {
        SimChain {
            sim: Mutex::new(sim),
            accepted: Mutex::default(),
        }
    }
}

impl Broadcaster for SimChain {
    fn submit(&self, bundle: &SpendBundle) -> Result<Broadcast, String> {
        self.sim
            .lock()
            .unwrap()
            .new_transaction(bundle.clone())
            .map_err(|e| e.to_string())?;
        self.accepted.lock().unwrap().push(bundle.clone());
        Ok(Broadcast {
            transaction_id: format!("0x{}", hex::encode(bundle.name())),
            status: "pending".to_owned(),
        })
    }
}

/// A backend sharing one `MemorySecrets`, so two `SessionStore` objects can be opened
/// over the same bytes — an app that restarted.
#[derive(Debug)]
struct Shared(Arc<MemorySecrets>);

impl SecretStore for Shared {
    fn put(&self, key: &str, bytes: &[u8]) -> KitResult<()> {
        self.0.put(key, bytes)
    }
    fn get(&self, key: &str) -> KitResult<Option<Vec<u8>>> {
        self.0.get(key)
    }
    fn delete(&self, key: &str) -> KitResult<()> {
        self.0.delete(key)
    }
}

fn signature_of(result: &str) -> Signature {
    let text: String = serde_json::from_str(result).unwrap();
    let bytes = hex::decode(text.trim_start_matches("0x")).unwrap();
    Signature::from_bytes(&bytes.try_into().unwrap()).unwrap()
}

// ---------------------------------------------------------------------------------------
// Spend builders (the wallet-kit fixtures, rebuilt)
// ---------------------------------------------------------------------------------------

fn params_for(spends: &[CoinSpend], partial: bool) -> String {
    let arr: Vec<Value> = spends
        .iter()
        .map(|cs| {
            json!({
                "coin": {
                    "parent_coin_info": format!("0x{}", hex::encode(cs.coin.parent_coin_info)),
                    "puzzle_hash": format!("0x{}", hex::encode(cs.coin.puzzle_hash)),
                    "amount": cs.coin.amount,
                },
                "puzzle_reveal": format!("0x{}", hex::encode(cs.puzzle_reveal.as_ref())),
                "solution": format!("0x{}", hex::encode(cs.solution.as_ref())),
            })
        })
        .collect();
    json!({ "coinSpends": arr, "partialSign": partial }).to_string()
}

fn send(sim: &mut Simulator, from: &BlsPair, amount: u64) -> Vec<CoinSpend> {
    let coin = sim.new_coin(from.puzzle_hash, amount);
    let mut ctx = SpendContext::new();
    let conditions =
        Conditions::new().create_coin(BlsPair::new(2).puzzle_hash, amount, Memos::None);
    StandardLayer::new(from.pk)
        .spend(&mut ctx, coin, conditions)
        .unwrap();
    ctx.take()
}

fn anyone_can_spend(parent: u8, amount: u64, conditions: Conditions) -> CoinSpend {
    let mut a = Allocator::new();
    let puzzle = Program::from(vec![0x01]);
    let node = puzzle.to_clvm(&mut a).unwrap();
    let puzzle_hash = Bytes32::from(tree_hash(&a, node));
    let solution = conditions.to_clvm(&mut a).unwrap();
    CoinSpend::new(
        Coin::new(Bytes32::new([parent; 32]), puzzle_hash, amount),
        puzzle,
        Program::from_clvm(&a, solution).unwrap(),
    )
}

fn settlement(
    ctx: &mut SpendContext,
    parent: Bytes32,
    amount: u64,
    payment: NotarizedPayment,
) -> CoinSpend {
    let puzzle = SettlementLayer.construct_puzzle(ctx).unwrap();
    let solution = SettlementLayer
        .construct_solution(ctx, SettlementPaymentsSolution::new(vec![payment]))
        .unwrap();
    CoinSpend::new(
        Coin::new(parent, Bytes32::from(SETTLEMENT_PAYMENT_HASH), amount),
        ctx.serialize(&puzzle).unwrap(),
        ctx.serialize(&solution).unwrap(),
    )
}

fn announcement(ctx: &mut SpendContext, payment: &NotarizedPayment) -> Bytes32 {
    let node = ctx.alloc(payment).unwrap();
    let message = ctx.tree_hash(node).to_bytes();
    Bytes32::from(sha256_parts(&[
        SETTLEMENT_PAYMENT_HASH.as_slice(),
        &message,
    ]))
}

/// Alice gives 1000 and is paid 500; Bob gives 500 and is paid 1000. `bind_*` controls
/// whether each side asserts the payment it expects.
struct Swap {
    alice: BlsPair,
    bob: BlsPair,
    spends: Vec<CoinSpend>,
}

fn swap(sim: &mut Simulator, bind_alice: bool, bind_bob: bool) -> Swap {
    let (alice, bob) = (BlsPair::new(11), BlsPair::new(12));
    let a = sim.new_coin(alice.puzzle_hash, 1000);
    let b = sim.new_coin(bob.puzzle_hash, 500);
    let settlement_ph = Bytes32::from(SETTLEMENT_PAYMENT_HASH);
    let np_alice = NotarizedPayment::new(
        a.coin_id(),
        vec![Payment::new(alice.puzzle_hash, 500, Memos::None)],
    );
    let np_bob = NotarizedPayment::new(
        b.coin_id(),
        vec![Payment::new(bob.puzzle_hash, 1000, Memos::None)],
    );
    let mut ctx = SpendContext::new();
    let ann_alice = announcement(&mut ctx, &np_alice);
    let ann_bob = announcement(&mut ctx, &np_bob);
    let mut ca = Conditions::new().create_coin(settlement_ph, 1000, Memos::None);
    if bind_alice {
        ca = ca.assert_puzzle_announcement(ann_alice);
    }
    let mut cb = Conditions::new().create_coin(settlement_ph, 500, Memos::None);
    if bind_bob {
        cb = cb.assert_puzzle_announcement(ann_bob);
    }
    StandardLayer::new(alice.pk).spend(&mut ctx, a, ca).unwrap();
    StandardLayer::new(bob.pk).spend(&mut ctx, b, cb).unwrap();
    let mut spends = ctx.take();
    spends.push(settlement(&mut ctx, a.coin_id(), 1000, np_bob));
    spends.push(settlement(&mut ctx, b.coin_id(), 500, np_alice));
    Swap { alice, bob, spends }
}

// ---------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------

/// An approved request produces a signature the chain accepts, the user saw the simulated
/// loss rather than a dApp label, each signature was confirmed separately, and the spend
/// was recorded against the daily limit.
#[test]
fn an_approved_spend_is_signed_and_recorded() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let mut sim = Simulator::new();
    let spends = send(&mut sim, &who, 500);

    let (outcome, calls) = harness.call("chip0002_signCoinSpends", &params_for(&spends, false));
    let signature = signature_of(&outcome.unwrap());
    sim.new_transaction(SpendBundle::new(spends, signature))
        .unwrap();
    assert_eq!(calls, 1);

    let prompts = harness.webview.prompts();
    // One prompt for the whole request; no confirmation per signature.
    assert_eq!(prompts.len(), 1);
    assert!(matches!(prompts[0].kind, ApprovalKind::Request { .. }));
    let ApprovalKind::Request { ref prompt } = prompts[0].kind;
    let shown = prompt.to_string();
    assert!(shown.contains("\"net\":-500"), "{shown}");

    // The loss was committed to the persistent limit store.
    let limits = PersistentLimits::new(harness.store.clone(), 1, "s1".to_owned());
    assert_eq!(limits.load().unwrap().spent.get(&AssetId::Xch), Some(&500));
}

/// A rejection reaches the dApp as the CHIP-0002 user rejection, and the signer is never
/// called.
#[test]
fn a_rejected_request_signs_nothing() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), false);
    let spends = send(&mut Simulator::new(), &who, 500);
    let (outcome, calls) = harness.call("signCoinSpends", &params_for(&spends, false));
    assert_eq!(outcome.unwrap_err().code, codes::USER_REJECTED);
    assert_eq!(calls, 0, "the signer was never reached");

    let limits = PersistentLimits::new(harness.store.clone(), 1, "s1".to_owned());
    assert!(limits.load().unwrap().spent.is_empty());
}

/// Dismissing the authentication cancels the whole request: nothing is signed or recorded,
/// even though the request itself was approved.
#[test]
fn a_cancelled_authentication_cancels_the_request() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let spends = send(&mut Simulator::new(), &who, 500);

    let approver = UiApprover::new(
        harness.webview.bridge.clone(),
        harness.meta("signCoinSpends"),
    );
    let mut signer = CountingSigner::new(&who);
    signer.cancel = true;
    let limits = PersistentLimits::new(harness.store.clone(), 1, "s1".to_owned());
    let context = RequestContext {
        dapp: DAPP,
        network: Network::Testnet11,
        session_chain_id: "testnet11",
        permissions: &harness.permissions,
        allow_agg_sig_unsafe: false,
        allow_unknown_contracts: false,
        ownership: &harness.ownership,
        keys: &harness.keys,
        limits: &limits,
        now: NOW,
    };
    let outcome = kit::handle(
        "signCoinSpends",
        &params_for(&spends, false),
        &context,
        &signer,
        &approver,
    );
    assert_eq!(outcome.unwrap_err().code, codes::USER_REJECTED);
    assert_eq!(signer.calls.load(Ordering::SeqCst), 0, "nothing signed");
    assert!(limits.load().unwrap().spent.is_empty(), "nothing committed");
}

/// A five-coin send is one prompt and one signing batch — one key unlock — not one per
/// coin (the 1 + 5 prompts of the first testnet runs).
#[test]
fn a_five_coin_request_is_one_prompt_and_one_batch() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let mut sim = Simulator::new();
    let mut spends = send(&mut sim, &who, 500);
    for _ in 0..4 {
        let coin = sim.new_coin(who.puzzle_hash, 100);
        let mut ctx = SpendContext::new();
        StandardLayer::new(who.pk)
            .spend(&mut ctx, coin, Conditions::new())
            .unwrap();
        spends.extend(ctx.take());
    }
    let (outcome, calls, batches) = harness.run(
        "signCoinSpends",
        &params_for(&spends, false),
        &SimChain::default(),
    );
    let signature = signature_of(&outcome.unwrap());
    sim.new_transaction(SpendBundle::new(spends, signature))
        .unwrap();
    assert_eq!(harness.webview.prompts().len(), 1);
    assert_eq!((calls, batches), (5, 1));
}

/// `xchonnect_submitCoinSpends`: Klimper signs once and broadcasts; the chain accepts it,
/// the dApp gets the transaction id, and the screen was told what the wallet will do.
#[test]
fn a_submitted_bundle_is_broadcast_by_the_wallet() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let mut sim = Simulator::new();
    let spends = send(&mut sim, &who, 500);
    let chain = SimChain::on(sim);
    let (outcome, _, batches) =
        harness.run(kit::SUBMIT_COIN_SPENDS, &params_for(&spends, false), &chain);
    let result: Value = serde_json::from_str(&outcome.unwrap()).unwrap();
    assert_eq!(result["status"], "pending");
    assert!(result["transactionId"].as_str().unwrap().starts_with("0x"));
    assert_eq!(chain.accepted.lock().unwrap().len(), 1);
    assert_eq!(batches, 1);

    let prompts = harness.webview.prompts();
    assert_eq!(prompts.len(), 1);
    let ApprovalKind::Request { ref prompt } = prompts[0].kind;
    assert_eq!(prompt["type"], "submit_coin_spends");
    assert_eq!(prompt["wallet_built"], false);
}

/// The dApp is told where its request is (`rpc.status`): shown once the prompt is on
/// screen, approved once the user said yes, and nothing more for a rejection.
#[test]
fn progress_is_reported_as_the_user_decides() {
    let who = BlsPair::new(1);
    for (approve, expected) in [
        (true, vec![Progress::Shown, Progress::Approved]),
        (false, vec![Progress::Shown]),
    ] {
        let mut harness = Harness::new(who.clone(), approve);
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        harness.progress = Some(sender);
        let spends = send(&mut Simulator::new(), &who, 500);
        let _ = harness.call("signCoinSpends", &params_for(&spends, false));
        let mut seen = Vec::new();
        while let Ok(step) = receiver.try_recv() {
            seen.push(step);
        }
        assert_eq!(seen, expected, "approve: {approve}");
    }
}

/// Spec 11.2: a `partialSign` request whose user spends do not assert the counterparty's
/// payment is refused, and the user is not even asked. The bound version is signed, and
/// the prompt says what the signature depends on receiving.
#[test]
fn an_unbound_partial_spend_is_refused_without_a_prompt() {
    let mut sim = Simulator::new();
    let unbound = swap(&mut sim, false, true);
    let harness = Harness::new(unbound.alice.clone(), true);
    let (outcome, calls) = harness.call("signCoinSpends", &params_for(&unbound.spends, true));
    let error = outcome.unwrap_err();
    assert_eq!(error.code, codes::UNAUTHORIZED);
    assert_eq!(
        error.data.as_deref(),
        Some(r#"{"reason":"unbound_partial"}"#)
    );
    assert_eq!(calls, 0);
    assert!(
        harness.webview.prompts().is_empty(),
        "the user was never asked"
    );

    // Bound: both sides sign and the aggregate is a bundle the chain accepts.
    let mut sim = Simulator::new();
    let bound = swap(&mut sim, true, true);
    let sign_as = |who: BlsPair| {
        let harness = Harness::new(who, true);
        let (outcome, _) = harness.call("signCoinSpends", &params_for(&bound.spends, true));
        let prompts = harness.webview.prompts();
        let ApprovalKind::Request { ref prompt } = prompts[0].kind;
        (signature_of(&outcome.unwrap()), prompt.to_string())
    };
    let (sig_alice, prompt_alice) = sign_as(bound.alice.clone());
    let (sig_bob, prompt_bob) = sign_as(bound.bob.clone());
    assert!(
        prompt_alice.contains("\"all_bound\":true") && prompt_alice.contains("\"amount\":500"),
        "{prompt_alice}"
    );
    assert!(prompt_bob.contains("\"amount\":1000"), "{prompt_bob}");
    let mut aggregate = sig_alice;
    aggregate.aggregate(&sig_bob);
    sim.new_transaction(SpendBundle::new(bound.spends, aggregate))
        .unwrap();
}

/// `AGG_SIG_UNSAFE` is refused for a dApp that has no override, before any prompt — such
/// a signature is valid in any spend context.
#[test]
fn agg_sig_unsafe_is_refused_by_default() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let mut ctx = SpendContext::new();
    let coin = Coin::new(Bytes32::new([4; 32]), who.puzzle_hash, 1000);
    StandardLayer::new(who.pk)
        .spend(
            &mut ctx,
            coin,
            Conditions::new()
                .create_coin(who.puzzle_hash, 1000, Memos::None)
                .agg_sig_unsafe(who.pk, vec![1, 2, 3].into()),
        )
        .unwrap();
    let spends = ctx.take();
    let (outcome, calls) = harness.call("signCoinSpends", &params_for(&spends, false));
    let error = outcome.unwrap_err();
    assert_eq!(error.code, codes::UNAUTHORIZED);
    assert_eq!(
        error.data.as_deref(),
        Some(r#"{"reason":"agg_sig_unsafe"}"#)
    );
    assert_eq!(calls, 0);
    assert!(harness.webview.prompts().is_empty());
}

/// A spend of a puzzle the wallet does not recognise is refused with the CHIP-0002
/// unsupported-content code, and never prompted (spec 11.1 item 3).
#[test]
fn an_unknown_contract_is_refused() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let unknown = anyone_can_spend(0, 1, Conditions::new().agg_sig_me(who.pk, vec![1].into()));
    let (outcome, calls) = harness.call("signCoinSpends", &params_for(&[unknown], false));
    assert_eq!(outcome.unwrap_err().code, codes::UNSUPPORTED_CONTENT);
    assert_eq!(calls, 0);
    assert!(harness.webview.prompts().is_empty());
}

/// A per-request limit refuses before any prompt, and the daily limit survives a restart
/// of the store (TASK-079 AC #3).
#[test]
fn limits_refuse_before_prompting_and_survive_a_restart() {
    let who = BlsPair::new(1);
    let mut harness = Harness::new(who.clone(), true);
    harness.permissions.limits.insert(
        AssetId::Xch,
        AssetLimit {
            per_request: Some(100),
            per_day: None,
        },
    );
    let spends = send(&mut Simulator::new(), &who, 500);
    let (outcome, calls) = harness.call("signCoinSpends", &params_for(&spends, false));
    let error = outcome.unwrap_err();
    assert_eq!(error.code, codes::LIMIT_EXCEEDED);
    assert_eq!(error.data.as_deref(), Some(r#"{"reason":"per_request"}"#));
    assert_eq!(calls, 0);
    assert!(harness.webview.prompts().is_empty());

    // A day's total, written and then read by a freshly opened store.
    let secrets = MemorySecrets::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let record = DailySpend {
        day: NOW / 86_400,
        spent: [(AssetId::Xch, 700_u128)].into_iter().collect(),
    };
    // One backing store, two `SessionStore` objects: an app that restarted.
    let shared: Arc<MemorySecrets> = Arc::new(secrets);
    let first = Arc::new(SessionStore::open(&path, Box::new(Shared(shared.clone()))).unwrap());
    PersistentLimits::new(first, 1, "s1".to_owned())
        .save(&record)
        .unwrap();
    let second = Arc::new(SessionStore::open(&path, Box::new(Shared(shared))).unwrap());
    let loaded = PersistentLimits::new(second, 1, "s1".to_owned())
        .load()
        .unwrap();
    assert_eq!(loaded.spent.get(&AssetId::Xch), Some(&700));
    assert_eq!(loaded.day, NOW / 86_400);
}

/// A signature that does not verify for the key that was asked for is never returned.
#[test]
fn a_faulty_signer_produces_nothing() {
    let who = BlsPair::new(1);
    let mut harness = Harness::new(who.clone(), true);
    harness.faulty = true;
    let spends = send(&mut Simulator::new(), &who, 500);
    let (outcome, _) = harness.call("signCoinSpends", &params_for(&spends, false));
    assert_eq!(outcome.unwrap_err().code, codes::NO_SECRET_KEY);
    let limits = PersistentLimits::new(harness.store.clone(), 1, "s1".to_owned());
    assert!(limits.load().unwrap().spent.is_empty());
}

/// Stored permissions are what the context is built from, so a method the user removed is
/// refused after a restart.
#[test]
fn stored_permissions_limit_the_methods() {
    let who = BlsPair::new(1);
    let mut harness = Harness::new(who.clone(), true);
    let mut stored = StoredPermissions::from_kit(&harness.permissions);
    stored.methods = vec!["chainId".to_owned()];
    stored.limits = BTreeMap::new();
    harness.permissions = stored.to_kit();
    let spends = send(&mut Simulator::new(), &who, 500);
    let (outcome, calls) = harness.call("signCoinSpends", &params_for(&spends, false));
    assert_eq!(outcome.unwrap_err().code, codes::UNAUTHORIZED);
    assert_eq!(calls, 0);
    // `chainId` still works and needs no prompt.
    let (outcome, _) = harness.call("chainId", "{}");
    assert_eq!(outcome.unwrap(), "\"testnet11\"");
}

/// A session approved for another chain is refused. wallet-kit checks this inside the
/// signing policy; Klimper checks it again before any prompt (see
/// `requests::answer`).
#[test]
fn a_request_for_another_network_is_refused() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let spends = send(&mut Simulator::new(), &who, 500);
    let limits = PersistentLimits::new(harness.store.clone(), 1, "s1".to_owned());
    let approver = UiApprover::new(
        harness.webview.bridge.clone(),
        harness.meta("signCoinSpends"),
    );
    let signer = CountingSigner::new(&who);
    let context = RequestContext {
        dapp: DAPP,
        network: Network::Testnet11,
        // The session was approved for mainnet while the wallet is on testnet11.
        session_chain_id: "mainnet",
        permissions: &harness.permissions,
        allow_agg_sig_unsafe: false,
        allow_unknown_contracts: false,
        ownership: &harness.ownership,
        keys: &harness.keys,
        limits: &limits,
        now: NOW,
    };
    let outcome = kit::handle(
        "signCoinSpends",
        &params_for(&spends, false),
        &context,
        &signer,
        &approver,
    );
    let error = outcome.unwrap_err();
    assert_eq!(error.code, codes::UNAUTHORIZED);
    assert_eq!(error.data.as_deref(), Some(r#"{"reason":"wrong_network"}"#));
    assert_eq!(signer.calls.load(Ordering::SeqCst), 0);
    assert!(harness.webview.prompts().is_empty());
}
