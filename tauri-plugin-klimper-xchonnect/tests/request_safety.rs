// SPDX-License-Identifier: AGPL-3.0-or-later
//! The safety properties a dApp request has to satisfy, driven through Klimper's own
//! approval bridge and limit store (clapandpay TASK-079 AC #5).
//!
//! The spends are the ones `xchonnect-wallet-kit`'s own fixtures use — a standard send
//! and an atomic two-party XCH swap through offer settlement payments — rebuilt here
//! because those fixtures are `#[cfg(test)]` inside that crate. What is under test is the
//! wiring: that an approval really has to come back from the webview before anything is
//! signed, that a refusal reaches the user as a CHIP-0002 error, and that the refusals
//! which must happen *before* any prompt really do — among them Klimper's own, of a spend
//! whose conditions the signature would not fix (TASK-141).
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
use chia_sdk_driver::{Cat, CatSpend, Layer, SettlementLayer, Spend, SpendContext, StandardLayer};
use chia_sdk_test::{BlsPair, Simulator};
use chia_sdk_types::Conditions;
use clvm_traits::{FromClvm, ToClvm};
use clvm_utils::tree_hash;
use clvmr::{Allocator, NodePtr};
use serde_json::{Value, json};
use xchonnect_core::crypto::sha256_parts;
use xchonnect_core::rpc::codes;
use xchonnect_wallet_kit::{
    self as kit, AssetId, AssetLimit, Broadcast, Broadcaster, DailySpend, DappPermissions, Host,
    LimitStore, Network, Ownership, RequestContext, RpcError, SignedCoins, Signer, SignerError,
};

use tauri_plugin_klimper_xchonnect::approval::{Approval, ApprovalBridge, ApprovalKind, Emitter};
use tauri_plugin_klimper_xchonnect::error::Result as KitResult;
use tauri_plugin_klimper_xchonnect::requests::{
    Progress, ProgressSender, RequestMeta, UiApprover, handle_dapp_built,
};
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
    /// The user allowed `AGG_SIG_UNSAFE` for the website.
    allow_unsafe: bool,
    /// Where the approver reports progress, as the session loop would receive it.
    progress: Option<ProgressSender>,
    /// The wallet's record of signed coins the requests are checked against.
    signed: SignedCoins,
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
            allow_unsafe: false,
            progress: None,
            signed: SignedCoins::default(),
        }
    }

    fn meta(&self, method: &str) -> RequestMeta {
        RequestMeta {
            session_id: "s1".to_owned(),
            dapp: DAPP.to_owned(),
            method: method.to_owned(),
            request_id: "r1".to_owned(),
            // No deadline of its own: `NOW` is a fixed date, and the bridge checks a
            // deadline against the real clock (its own test covers that).
            expires_at: 0,
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
        let limits = PersistentLimits::new(self.store.clone(), 1, DAPP.to_owned(), "s1".to_owned());
        let context = RequestContext {
            dapp: DAPP,
            network: Network::Testnet11,
            session_chain_id: "testnet11",
            permissions: &self.permissions,
            allow_agg_sig_unsafe: self.allow_unsafe,
            allow_unknown_contracts: false,
            ownership: &self.ownership,
            keys: &self.keys,
            limits: &limits,
            now: NOW,
            chain: None,
            signed_coins: &self.signed,
        };
        let host = Host {
            signer: &signer,
            approver: &approver,
            broadcaster: Some(chain),
        };
        // What `requests::run` calls for a request the dApp built.
        let outcome = handle_dapp_built(method, params, &context, &host);
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

/// A broadcaster that is a simulated chain: what it accepts, a full node would. It keeps the
/// wallet's record of signed coins as Klimper's broadcaster does (`SignedCoins::add` before
/// the bundle goes out, also when the chain then refuses it).
#[derive(Default)]
struct SimChain {
    sim: Mutex<Simulator>,
    accepted: Mutex<Vec<SpendBundle>>,
    recorded: Mutex<SignedCoins>,
    /// The record cannot be written (a locked keyring).
    unwritable: bool,
}

impl SimChain {
    fn on(sim: Simulator) -> Self {
        SimChain {
            sim: Mutex::new(sim),
            ..SimChain::default()
        }
    }
}

impl Broadcaster for SimChain {
    fn record(&self, record: &kit::BroadcastRecord) -> Result<(), String> {
        if self.unwritable {
            return Err("the keyring is locked".to_owned());
        }
        self.recorded.lock().unwrap().add(record);
        Ok(())
    }

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
    let limits = PersistentLimits::new(harness.store.clone(), 1, DAPP.to_owned(), "s1".to_owned());
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

    let limits = PersistentLimits::new(harness.store.clone(), 1, DAPP.to_owned(), "s1".to_owned());
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
    let limits = PersistentLimits::new(harness.store.clone(), 1, DAPP.to_owned(), "s1".to_owned());
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
        chain: None,
        signed_coins: &xchonnect_wallet_kit::SignedCoins::default(),
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

/// A coin the wallet owns under a key the dApp was not given gets the answer a stranger's
/// coin gets: no prompt and the same code (TASK-122). `requests::run` hands wallet-kit only
/// the exposed keys for dApp-built requests, which is what `keys` stands for here.
#[test]
fn a_hidden_key_answers_like_a_strangers() {
    let who = BlsPair::new(1);
    let hidden = BlsPair::new(2);
    let stranger = BlsPair::new(3);
    let mut harness = Harness::new(who.clone(), true);
    harness
        .ownership
        .p2_puzzle_hashes
        .insert(hidden.puzzle_hash);

    let mine_but_hidden = send(&mut Simulator::new(), &hidden, 500);
    let theirs = send(&mut Simulator::new(), &stranger, 500);
    let (a, _) = harness.call("signCoinSpends", &params_for(&mine_but_hidden, false));
    let (b, _) = harness.call("signCoinSpends", &params_for(&theirs, false));
    assert_eq!(a.unwrap_err().code, b.unwrap_err().code);
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
        options: 2,
    };
    // One backing store, two `SessionStore` objects: an app that restarted.
    let shared: Arc<MemorySecrets> = Arc::new(secrets);
    let first = Arc::new(SessionStore::open(&path, Box::new(Shared(shared.clone()))).unwrap());
    PersistentLimits::new(first, 1, DAPP.to_owned(), "s1".to_owned())
        .save(&record)
        .unwrap();
    let second = Arc::new(SessionStore::open(&path, Box::new(Shared(shared))).unwrap());
    let loaded = PersistentLimits::new(second, 1, DAPP.to_owned(), "s1".to_owned())
        .load()
        .unwrap();
    assert_eq!(loaded.spent.get(&AssetId::Xch), Some(&700));
    assert_eq!(loaded.day, NOW / 86_400);
    assert_eq!(loaded.options, 2);
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
    let limits = PersistentLimits::new(harness.store.clone(), 1, DAPP.to_owned(), "s1".to_owned());
    assert!(limits.load().unwrap().spent.is_empty());
}

/// Stored permissions are what the context is built from, so a method the user removed is
/// refused after a restart.
#[test]
fn stored_permissions_limit_the_methods() {
    let who = BlsPair::new(1);
    let mut harness = Harness::new(who.clone(), true);
    let mut stored = StoredPermissions::from_kit(&harness.permissions).unwrap();
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
    let limits = PersistentLimits::new(harness.store.clone(), 1, DAPP.to_owned(), "s1".to_owned());
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
        chain: None,
        signed_coins: &xchonnect_wallet_kit::SignedCoins::default(),
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

// ---------------------------------------------------------------------------------------
// Spends whose conditions the signature does not fix (TASK-141)
// ---------------------------------------------------------------------------------------

/// A standard spend of a coin of `who` with `delegated` as its delegated puzzle and
/// `solution` as the delegated puzzle's solution; the solution pays `who` back.
fn delegated_spend(
    who: &BlsPair,
    delegated: &[u8],
    solution: impl Fn(&mut SpendContext, NodePtr) -> NodePtr,
) -> CoinSpend {
    let mut ctx = SpendContext::new();
    let coin = Coin::new(Bytes32::new([4; 32]), who.puzzle_hash, 1000);
    let program = ctx.alloc(&Program::from(delegated.to_vec())).unwrap();
    let conditions = ctx
        .alloc(&Conditions::new().create_coin(who.puzzle_hash, 1000, Memos::None))
        .unwrap();
    let solution = solution(&mut ctx, conditions);
    let spend = StandardLayer::new(who.pk)
        .delegated_inner_spend(&mut ctx, Spend::new(program, solution))
        .unwrap();
    ctx.spend(coin, spend).unwrap();
    ctx.take().remove(0)
}

/// `xchonnect_submitCoinSpends` under its `chip0002_` alias (spec 9.1).
const SUBMIT_ALIAS: &str = "chip0002_xchonnect_submitCoinSpends";

fn assert_unsigned_conditions(harness: &Harness, method: &str, spends: &[CoinSpend]) {
    // A well-formed spend wallet-kit can run: the refusal is not about anything else.
    let summary = kit::simulate(spends, &harness.ownership, kit::simulate::DEFAULT_MAX_COST);
    assert!(summary.is_ok(), "{method}: {summary:?}");
    let (outcome, calls) = harness.call(method, &params_for(spends, false));
    let error = outcome.unwrap_err();
    assert_eq!(error.code, codes::UNAUTHORIZED, "{method}");
    assert_eq!(
        error.data.as_deref(),
        Some(r#"{"reason":"unsigned_conditions"}"#),
        "{method}"
    );
    assert_eq!(calls, 0, "{method}: nothing was signed");
    assert!(
        harness.webview.prompts().is_empty(),
        "{method}: the user was never asked"
    );
}

/// The delegated puzzle `1` returns its solution as the conditions, so the signature, which
/// covers only the delegated puzzle, does not fix them: refused before any prompt, whether
/// the dApp asks for a signature or for the wallet to submit.
#[test]
fn a_delegated_puzzle_that_returns_its_solution_is_refused_without_a_prompt() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let spend = delegated_spend(&who, &[0x01], |_, conditions| conditions);
    for method in [
        "signCoinSpends",
        "chip0002_signCoinSpends",
        kit::SUBMIT_COIN_SPENDS,
        SUBMIT_ALIAS,
    ] {
        assert_unsigned_conditions(&harness, method, std::slice::from_ref(&spend));
    }
    // One such spend among honest ones refuses the whole request.
    let mut spends = send(&mut Simulator::new(), &who, 500);
    spends.push(spend);
    assert_unsigned_conditions(&harness, "signCoinSpends", &spends);
}

/// Any other delegated puzzle that reads its solution: one that returns part of it, `(f 1)`,
/// and one that runs a program from it, `(a 2 3)`.
#[test]
fn other_delegated_puzzles_that_read_their_solution_are_refused() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let first = delegated_spend(&who, &[0xff, 0x05, 0xff, 0x01, 0x80], |ctx, conditions| {
        ctx.alloc(&(conditions, ())).unwrap()
    });
    let apply = delegated_spend(
        &who,
        &[0xff, 0x02, 0xff, 0x02, 0xff, 0x03, 0x80],
        |ctx, conditions| {
            let quoted = ctx.alloc(&(1, conditions)).unwrap();
            ctx.alloc(&(quoted, ())).unwrap()
        },
    );
    for spend in [first, apply] {
        assert_unsigned_conditions(&harness, "signCoinSpends", &[spend]);
    }
}

/// The same inside a CAT: the inner standard spend is held to the same rule.
#[test]
fn a_cat_whose_inner_spend_reads_its_solution_is_refused() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let mut ctx = SpendContext::new();
    let coin = Coin::new(Bytes32::new([5; 32]), who.puzzle_hash, 1000);
    let (_, cats) = Cat::single_issuance(
        &mut ctx,
        coin.coin_id(),
        None,
        1000,
        Conditions::new().create_coin(who.puzzle_hash, 1000, Memos::None),
    )
    .unwrap();
    let one = ctx.alloc(&Program::from(vec![0x01])).unwrap();
    let conditions = ctx
        .alloc(&Conditions::new().create_coin(who.puzzle_hash, 1000, Memos::None))
        .unwrap();
    let inner = StandardLayer::new(who.pk)
        .delegated_inner_spend(&mut ctx, Spend::new(one, conditions))
        .unwrap();
    // Only the CAT spend: the issuance itself is not part of the request.
    let _ = ctx.take();
    Cat::spend_all(&mut ctx, &[CatSpend::new(cats[0], inner)]).unwrap();
    assert_unsigned_conditions(&harness, "signCoinSpends", &ctx.take());
}

/// The honest shape — quoted conditions — is signed as before and the chain accepts it,
/// through both methods.
#[test]
fn quoted_conditions_are_signed_as_before() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let mut sim = Simulator::new();
    let spends = send(&mut sim, &who, 500);
    let (outcome, _) = harness.call("signCoinSpends", &params_for(&spends, false));
    sim.new_transaction(SpendBundle::new(spends, signature_of(&outcome.unwrap())))
        .unwrap();

    let mut sim = Simulator::new();
    let spends = send(&mut sim, &who, 500);
    let chain = SimChain::on(sim);
    let (outcome, _, _) = harness.run(kit::SUBMIT_COIN_SPENDS, &params_for(&spends, false), &chain);
    assert!(outcome.is_ok());
    assert_eq!(chain.accepted.lock().unwrap().len(), 1);
    assert_eq!(harness.webview.prompts().len(), 2);
}

/// The `chip0002_` alias of the submit method is the same method: an honest spend is
/// signed and broadcast through it.
#[test]
fn the_submit_alias_submits_quoted_conditions() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let mut sim = Simulator::new();
    let spends = send(&mut sim, &who, 500);
    let chain = SimChain::on(sim);
    let (outcome, calls, _) = harness.run(SUBMIT_ALIAS, &params_for(&spends, false), &chain);
    assert!(outcome.unwrap().contains("transactionId"));
    assert!(calls > 0);
    assert_eq!(chain.accepted.lock().unwrap().len(), 1);
    assert_eq!(harness.webview.prompts().len(), 1);
}

// ---------------------------------------------------------------------------------------
// AGG_SIG_UNSAFE messages that could stand in for another signature (TASK-141)
// ---------------------------------------------------------------------------------------

/// A spend of `who`'s coin, quoted conditions, asking for `who`'s `AGG_SIG_UNSAFE`
/// signature of `message`.
fn unsafe_spend(sim: &mut Simulator, who: &BlsPair, message: Vec<u8>) -> Vec<CoinSpend> {
    let coin = sim.new_coin(who.puzzle_hash, 1000);
    let mut ctx = SpendContext::new();
    StandardLayer::new(who.pk)
        .spend(
            &mut ctx,
            coin,
            Conditions::new()
                .create_coin(who.puzzle_hash, 1000, Memos::None)
                .agg_sig_unsafe(who.pk, message.into()),
        )
        .unwrap();
    ctx.take()
}

/// With `AGG_SIG_UNSAFE` allowed for the website, a message that ends with the network's
/// `AGG_SIG_ME` data, or that has the size of what `signMessage` signs, is still refused
/// before the user is asked, through every method.
#[test]
fn an_unsafe_message_shaped_like_another_signature_is_refused_without_a_prompt() {
    let who = BlsPair::new(1);
    let mut harness = Harness::new(who.clone(), true);
    harness.allow_unsafe = true;
    let mut coin_bound = vec![7; 40];
    coin_bound.extend_from_slice(
        chia_sdk_types::TESTNET11_CONSTANTS
            .agg_sig_me_additional_data
            .as_ref(),
    );
    let signed_message = kit::signed_message_hash(b"I agree").to_vec();
    for message in [coin_bound, signed_message] {
        let spends = unsafe_spend(&mut Simulator::new(), &who, message);
        for method in [
            "signCoinSpends",
            "chip0002_signCoinSpends",
            kit::SUBMIT_COIN_SPENDS,
            SUBMIT_ALIAS,
        ] {
            let (outcome, calls) = harness.call(method, &params_for(&spends, false));
            let error = outcome.unwrap_err();
            assert_eq!(error.code, codes::UNAUTHORIZED, "{method}");
            assert_eq!(
                error.data.as_deref(),
                Some(r#"{"reason":"unsafe_message"}"#),
                "{method}"
            );
            assert_eq!(calls, 0, "{method}: nothing was signed");
        }
    }
    assert!(harness.webview.prompts().is_empty());
}

/// Any other unsafe message is asked about and signed as before once the user allowed
/// `AGG_SIG_UNSAFE`, and still refused by wallet-kit when they did not.
#[test]
fn an_honest_unsafe_message_is_signed_when_allowed() {
    let who = BlsPair::new(1);
    let mut harness = Harness::new(who.clone(), true);
    let mut sim = Simulator::new();
    let spends = unsafe_spend(&mut sim, &who, b"hello, dApp".to_vec());
    let (outcome, calls) = harness.call("signCoinSpends", &params_for(&spends, false));
    assert_eq!(
        outcome.unwrap_err().data.as_deref(),
        Some(r#"{"reason":"agg_sig_unsafe"}"#)
    );
    assert_eq!(calls, 0);

    harness.allow_unsafe = true;
    let (outcome, calls) = harness.call("signCoinSpends", &params_for(&spends, false));
    assert!(calls > 0);
    assert_eq!(harness.webview.prompts().len(), 1);
    // A valid signature: the chain accepts the spend with it.
    sim.new_transaction(SpendBundle::new(spends, signature_of(&outcome.unwrap())))
        .unwrap();
}

// ---------------------------------------------------------------------------------------
// The record of signed coins (xchonnect spec 11.2)
// ---------------------------------------------------------------------------------------

/// One spend of `coin` paying `amount` to someone else.
fn spend_of(coin: Coin, who: &BlsPair, amount: u64) -> Vec<CoinSpend> {
    let mut ctx = SpendContext::new();
    let conditions =
        Conditions::new().create_coin(BlsPair::new(2).puzzle_hash, amount, Memos::None);
    StandardLayer::new(who.pk)
        .spend(&mut ctx, coin, conditions)
        .unwrap();
    ctx.take()
}

/// The record of a sealed bundle that held `spends`: the digests of what `who` signed in it,
/// and the coins it signed for.
fn sealed_record(spends: &[CoinSpend], who: &BlsPair) -> SignedCoins {
    use chia_wallet_sdk::signer::{AggSigConstants, RequiredSignature};
    use chia_wallet_sdk::types::TESTNET11_CONSTANTS;
    let constants = AggSigConstants::new(TESTNET11_CONSTANTS.agg_sig_me_additional_data);
    let mut allocator = Allocator::new();
    let mut record = kit::BroadcastRecord::default();
    let mut bundle = kit::SealedBundle {
        id: Bytes32::new([0x5e; 32]),
        ..Default::default()
    };
    for cs in spends {
        record.coins.push(cs.coin.coin_id());
        for required in RequiredSignature::from_coin_spend(&mut allocator, cs, &constants).unwrap()
        {
            if let RequiredSignature::Bls(bls) = required
                && bls.public_key == who.pk
            {
                bundle
                    .digests
                    .push(kit::sealed_digest(&bls.public_key, &bls.message()));
                bundle.coins.push(cs.coin.coin_id());
            }
        }
    }
    assert!(!bundle.digests.is_empty());
    record.sealed = Some(bundle);
    let mut signed = SignedCoins::default();
    signed.add(&record);
    signed
}

/// A signature recorded for a sealed bundle is never made again, for any request a website
/// sends (`signCoinSpends`, `xchonnect_submitCoinSpends`, `signMessage`), before anything is
/// shown; a different spend of the same coin, and another message, are signed as usual.
#[test]
fn a_sealed_signature_is_never_made_again_but_the_coin_can_be_spent_otherwise() {
    let who = BlsPair::new(1);
    let mut harness = Harness::new(who.clone(), true);
    let mut sim = Simulator::new();
    let coin = sim.new_coin(who.puzzle_hash, 1000);
    let sealed = spend_of(coin, &who, 500);
    harness.signed = sealed_record(&sealed, &who);
    let message = b"sign in to pengui".to_vec();
    let signed_hash = kit::signed_message_hash(&message);
    harness.signed.add(&kit::BroadcastRecord {
        coins: vec![],
        sealed: Some(kit::SealedBundle {
            id: Bytes32::new([0x5f; 32]),
            digests: vec![kit::sealed_digest(&who.pk, &signed_hash)],
            coins: vec![coin.coin_id()],
        }),
    });

    for method in [
        "signCoinSpends",
        "chip0002_signCoinSpends",
        kit::SUBMIT_COIN_SPENDS,
    ] {
        let chain = SimChain::on(Simulator::new());
        let (outcome, calls, _) = harness.run(method, &params_for(&sealed, false), &chain);
        let error = outcome.unwrap_err();
        assert_eq!(error.code, codes::UNAUTHORIZED, "{method}");
        assert_eq!(
            error.data.as_deref(),
            Some(r#"{"reason":"sealed_signature"}"#),
            "{method}"
        );
        assert_eq!(calls, 0, "{method}: nothing was signed");
        assert!(chain.accepted.lock().unwrap().is_empty());
    }
    let sign_message = |message: &[u8]| {
        json!({
            "message": format!("0x{}", hex::encode(message)),
            "publicKey": format!("0x{}", hex::encode(who.pk.to_bytes())),
        })
        .to_string()
    };
    let (outcome, calls) = harness.call("signMessage", &sign_message(&message));
    assert_eq!(
        outcome.unwrap_err().data.as_deref(),
        Some(r#"{"reason":"sealed_signature"}"#)
    );
    assert_eq!(calls, 0);
    assert!(harness.webview.prompts().is_empty());

    // Another spend of the same coin, and another message: signed, and valid on chain.
    let other = spend_of(coin, &who, 400);
    let (outcome, calls) = harness.call("signCoinSpends", &params_for(&other, false));
    assert_eq!(calls, 1);
    sim.new_transaction(SpendBundle::new(other, signature_of(&outcome.unwrap())))
        .unwrap();
    let (outcome, _) = harness.call("signMessage", &sign_message(b"another message"));
    assert!(outcome.is_ok());
}

/// The wallet records a website's bundle before it is broadcast, also when the network then
/// refuses it (it may have reached other nodes): its coins are released.
#[test]
fn a_refused_broadcast_is_recorded_all_the_same() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    // The coin exists nowhere the simulated chain knows of: it refuses the bundle.
    let spends = send(&mut Simulator::new(), &who, 500);
    let chain = SimChain::default();
    let (outcome, _, _) = harness.run(kit::SUBMIT_COIN_SPENDS, &params_for(&spends, false), &chain);
    let error = outcome.unwrap_err();
    assert_eq!(
        error.data.as_deref(),
        Some(r#"{"reason":"broadcast_failed"}"#)
    );
    let recorded = chain.recorded.lock().unwrap();
    assert!(recorded.released.contains(&spends[0].coin.coin_id()));
    // A website's bundle is released, never sealed.
    assert!(recorded.sealed.is_empty());
}

/// Approves after a pause: a prompt left open on the user's screen.
struct SlowUser;

impl kit::Approver for SlowUser {
    fn approve(&self, _prompt: &kit::Prompt<'_>) -> bool {
        std::thread::sleep(Duration::from_millis(300));
        true
    }
}

/// Coin-signing requests run one at a time, the record held from wallet-kit's check across
/// the approval prompt to what is recorded: a request B that arrives while a sealed request A
/// waits for its approval reads the record only after A recorded its signatures, and cannot
/// have A's message signed.
#[test]
fn a_request_that_arrives_during_a_sealed_prompt_cannot_sign_its_message() {
    use tauri_plugin_klimper_xchonnect::signed_coins::{
        Recording, SignedCoinsHandle, checked_record,
    };
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let record = SignedCoinsHandle::new(harness.store.clone(), 1, "testnet11");
    let mut sim = Simulator::new();
    let coin = sim.new_coin(who.puzzle_hash, 1000);
    let to = BlsPair::new(2).puzzle_hash;
    let mut ctx = SpendContext::new();
    StandardLayer::new(who.pk)
        .spend(
            &mut ctx,
            coin,
            Conditions::new()
                .create_coin(to, 400, Memos::None)
                .create_coin(who.puzzle_hash, 600, Memos::None),
        )
        .unwrap();
    let spends = ctx.take();
    let chain = SimChain::on(sim);
    let started = std::sync::Barrier::new(2);

    std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            let (_held, signed) = checked_record(&record, true, SignedCoinsHandle::load).unwrap();
            started.wait();
            let permissions = DappPermissions {
                methods: vec!["chia_send".to_owned()],
                exposed_keys: vec![who.pk],
                limits: BTreeMap::new(),
                options: kit::DEFAULT_OPTION_LIMIT,
            };
            let limits =
                PersistentLimits::new(harness.store.clone(), 1, DAPP.to_owned(), "s1".to_owned());
            let context = RequestContext {
                dapp: DAPP,
                network: Network::Testnet11,
                session_chain_id: "testnet11",
                permissions: &permissions,
                allow_agg_sig_unsafe: false,
                allow_unknown_contracts: false,
                ownership: &harness.ownership,
                keys: &harness.keys,
                limits: &limits,
                now: NOW,
                chain: None,
                signed_coins: &signed,
            };
            let intent = kit::Intent {
                kind: Some("send".to_owned()),
                recipients: Some(vec![kit::RecipientClaim {
                    asset: AssetId::Xch,
                    puzzle_hash: to,
                    amount: 400,
                }]),
                fee: Some(0),
                net: None,
            };
            let recording = Recording::new(&chain, &record);
            kit::submit(
                kit::SubmitRequest {
                    method: "chia_send",
                    spends: spends.clone(),
                    other_signature: None,
                    intent: Some(&intent),
                    wallet_built: true,
                    wallet_fee: 0,
                    signatures_released: false,
                },
                &context,
                &CountingSigner::new(&who),
                &SlowUser,
                &recording,
            )
            .map(|_| ())
        });
        let b = scope.spawn(|| {
            started.wait();
            // B arrives while A's prompt is open.
            let (_held, signed) = checked_record(&record, true, SignedCoinsHandle::load).unwrap();
            let mut late = Harness::new(who.clone(), true);
            late.signed = signed;
            late.call("signCoinSpends", &params_for(&spends, false))
        });
        a.join().unwrap().unwrap();
        let (outcome, calls) = b.join().unwrap();
        assert_eq!(
            outcome.unwrap_err().data.as_deref(),
            Some(r#"{"reason":"sealed_signature"}"#)
        );
        assert_eq!(calls, 0);
    });
    assert_eq!(chain.accepted.lock().unwrap().len(), 1);
    assert!(!record.load().unwrap().to_kit().unwrap().sealed.is_empty());
}

/// wallet-kit reports the coins of every `signCoinSpends` answer before the signature is
/// returned: they go into the record as released. When the record cannot be written, no
/// signature is returned (4000 `record_failed`) and nothing counts against the limit.
#[test]
fn a_signature_for_a_website_is_recorded_before_it_is_returned() {
    let who = BlsPair::new(1);
    let harness = Harness::new(who.clone(), true);
    let spends = send(&mut Simulator::new(), &who, 500);
    let chain = SimChain::default();
    let (outcome, _, _) = harness.run("signCoinSpends", &params_for(&spends, false), &chain);
    signature_of(&outcome.unwrap());
    assert!(
        chain
            .recorded
            .lock()
            .unwrap()
            .released
            .contains(&spends[0].coin.coin_id())
    );

    let harness = Harness::new(who.clone(), true);
    let locked = SimChain {
        unwritable: true,
        ..SimChain::default()
    };
    let (outcome, _, _) = harness.run(
        "chip0002_signCoinSpends",
        &params_for(&spends, false),
        &locked,
    );
    let error = tauri_plugin_klimper_xchonnect::requests::explain(outcome.unwrap_err());
    assert_eq!(error.code, codes::INVALID_PARAMS);
    assert_eq!(error.data.as_deref(), Some(r#"{"reason":"record_failed"}"#));
    assert!(error.message.contains("nothing was sent"));
    let limits = PersistentLimits::new(harness.store.clone(), 1, DAPP.to_owned(), "s1".to_owned());
    assert!(limits.load().unwrap().spent.is_empty());
}
