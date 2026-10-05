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
    self as kit, AssetId, AssetLimit, DailySpend, DappPermissions, LimitStore, Network, Ownership,
    RequestContext, RpcError, Signer, SignerError,
};

use tauri_plugin_klimper_xchonnect::approval::{Approval, ApprovalBridge, ApprovalKind, Emitter};
use tauri_plugin_klimper_xchonnect::error::Result as KitResult;
use tauri_plugin_klimper_xchonnect::requests::{RequestMeta, UiApprover};
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

/// Counts calls and can be made faulty: `.faulty` returns a signature by an unrelated key.
struct CountingSigner {
    secret: SecretKey,
    public: PublicKey,
    calls: AtomicUsize,
    faulty: bool,
    gate: Arc<dyn Fn(usize, &PublicKey) -> bool + Send + Sync>,
}

impl Signer for CountingSigner {
    fn sign(&self, public_key: &PublicKey, message: &[u8]) -> Result<Signature, SignerError> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if !(self.gate)(index, public_key) {
            return Err(SignerError::Cancelled);
        }
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
        }
    }

    fn meta(&self, method: &str) -> RequestMeta {
        RequestMeta {
            session_id: "s1".to_owned(),
            dapp: DAPP.to_owned(),
            method: method.to_owned(),
            expires_at: NOW + 60,
        }
    }

    /// Run one request the way `requests::answer` does: the same approver, the same
    /// per-signature gate, the same persistent limit store.
    fn call(&self, method: &str, params: &str) -> (Result<String, RpcError>, usize) {
        let plan_len = Arc::new(AtomicUsize::new(0));
        let approver = UiApprover::new(
            self.webview.bridge.clone(),
            self.meta(method),
            plan_len.clone(),
        );
        let bridge = self.webview.bridge.clone();
        let meta = self.meta(method);
        let gate_len = plan_len.clone();
        let gate = Arc::new(move |index: usize, public_key: &PublicKey| {
            bridge.ask(Approval {
                id: format!("sig-{index}-{}", rand_suffix()),
                session_id: meta.session_id.clone(),
                dapp: meta.dapp.clone(),
                method: meta.method.clone(),
                expires_at: meta.expires_at,
                kind: ApprovalKind::Signature {
                    index,
                    total: gate_len.load(Ordering::SeqCst),
                    public_key: format!("0x{}", hex::encode(public_key.to_bytes())),
                },
            })
        });
        let signer = CountingSigner {
            secret: self.who.sk.clone(),
            public: self.who.pk,
            calls: AtomicUsize::new(0),
            faulty: self.faulty,
            gate,
        };
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
        let outcome = kit::handle(method, params, &context, &signer, &approver);
        (outcome, signer.calls.load(Ordering::SeqCst))
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

fn rand_suffix() -> String {
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::SeqCst).to_string()
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
    // The request prompt, then one confirmation for the single signature.
    assert!(matches!(prompts[0].kind, ApprovalKind::Request { .. }));
    let ApprovalKind::Request { ref prompt } = prompts[0].kind else {
        panic!("expected a request prompt")
    };
    let shown = prompt.to_string();
    assert!(shown.contains("\"net\":-500"), "{shown}");
    assert!(matches!(
        prompts[1].kind,
        ApprovalKind::Signature {
            index: 1,
            total: 1,
            ..
        }
    ));

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

/// Declining one individual signature cancels the whole request, even though the request
/// as a whole was approved.
#[test]
fn declining_a_single_signature_cancels_the_request() {
    let who = BlsPair::new(1);
    let mut harness = Harness::new(who.clone(), true);
    // A gate that always refuses, whatever the webview says about the request.
    harness.permissions = DappPermissions::new_default(who.pk);
    let spends = send(&mut Simulator::new(), &who, 500);

    let plan_len = Arc::new(AtomicUsize::new(0));
    let approver = UiApprover::new(
        harness.webview.bridge.clone(),
        harness.meta("signCoinSpends"),
        plan_len,
    );
    let signer = CountingSigner {
        secret: who.sk.clone(),
        public: who.pk,
        calls: AtomicUsize::new(0),
        faulty: false,
        gate: Arc::new(|_, _| false),
    };
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
    assert!(limits.load().unwrap().spent.is_empty(), "nothing committed");
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
        let ApprovalKind::Request { ref prompt } = prompts[0].kind else {
            panic!("expected a request prompt")
        };
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
    let plan_len = Arc::new(AtomicUsize::new(0));
    let approver = UiApprover::new(
        harness.webview.bridge.clone(),
        harness.meta("signCoinSpends"),
        plan_len,
    );
    let signer = CountingSigner {
        secret: who.sk.clone(),
        public: who.pk,
        calls: AtomicUsize::new(0),
        faulty: false,
        gate: Arc::new(|_, _| true),
    };
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
