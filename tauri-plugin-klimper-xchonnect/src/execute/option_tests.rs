// SPDX-License-Identifier: AGPL-3.0-or-later
//! Option contracts end to end (TASK-140): Sage wallets on one simulated chain write, send,
//! use and claim back options, and request, take, repay and default on loans, through
//! Klimper's own path for each method — the key-free build with Sage's option code, the
//! cross-check against Sage's summary, wallet-kit's submit with a verified intent kind, one
//! approval — and the chain must accept every bundle that comes out. The refusals are run
//! on the same path: no permission, over the limit, the wrong network, too late, too early,
//! someone else's collateral, and a website's mint with a fake collateral.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};

use chia_bls::PublicKey;
use serde_json::Value;
use xchonnect_wallet_kit::permissions::{AssetLimit, DappPermissions};

use super::options::*;
use super::sage_sim::SimWallet;
use super::take_tests::{
    Kept, Limits, TakerKeys, issue_amount, know_cats, sage_summary, take, wallet_keys,
};
use super::*;

/// A wallet's keys and what wallet-kit counts as its own.
struct Who {
    signer: TakerKeys,
    keys: HashSet<PublicKey>,
    ownership: kit::Ownership,
}

async fn who(wallet: &SimWallet) -> Who {
    let (signer, keys, ownership) = wallet_keys(wallet).await;
    Who {
        signer,
        keys,
        ownership,
    }
}

/// Answers the approval and keeps the prompt exactly as the screen would receive it.
struct Seen {
    approves: bool,
    prompt: RefCell<Option<Value>>,
}

impl Seen {
    fn new(approves: bool) -> Self {
        Seen {
            approves,
            prompt: RefCell::new(None),
        }
    }
}

impl kit::Approver for Seen {
    fn approve(&self, prompt: &kit::Prompt<'_>) -> bool {
        *self.prompt.borrow_mut() = Some(serde_json::to_value(prompt).unwrap());
        self.approves
    }
}

fn granted(method: &str) -> DappPermissions {
    DappPermissions {
        methods: vec![method.to_owned()],
        exposed_keys: vec![],
        limits: BTreeMap::new(),
    }
}

/// How a request was answered.
struct Run {
    result: Result<String, RpcError>,
    prompt: Option<Value>,
    sent: Option<SpendBundle>,
    loan: Option<(LoanView, bool)>,
}

impl Run {
    fn ok(&self) -> &str {
        match &self.result {
            Ok(answer) => answer,
            Err(error) => panic!("refused: {error:?}"),
        }
    }

    fn asked(&self) -> bool {
        self.prompt.is_some()
    }

    fn reason(&self) -> String {
        match &self.result {
            Ok(answer) => panic!("not refused: {answer}"),
            Err(error) => format!("{} {}", error.code, error.data.clone().unwrap_or_default()),
        }
    }

    fn summary(&self) -> &Value {
        &self.prompt.as_ref().expect("never asked")["summary"]
    }
}

/// What a test varies about the request context.
struct Setup {
    permissions: DappPermissions,
    session_chain_id: &'static str,
    now: u64,
}

impl Setup {
    fn new(method: &str, now: u64) -> Self {
        Setup {
            permissions: granted(method),
            session_chain_id: "testnet11",
            now,
        }
    }
}

/// Run `submit` with a context for `w`, a user who approves, and a broadcaster that keeps
/// the bundle.
fn run(
    w: &Who,
    setup: &Setup,
    submit: impl FnOnce(
        &RequestContext<'_>,
        &dyn kit::Signer,
        &dyn kit::Approver,
        &dyn Broadcaster,
        &dyn Fn(&LoanView, bool),
    ) -> Result<String, RpcError>,
) -> Run {
    let limits = Limits::default();
    let ctx = RequestContext {
        dapp: "options.test",
        network: kit::Network::Testnet11,
        session_chain_id: setup.session_chain_id,
        permissions: &setup.permissions,
        allow_agg_sig_unsafe: false,
        allow_unknown_contracts: false,
        ownership: &w.ownership,
        keys: &w.keys,
        limits: &limits,
        now: setup.now,
        chain: None,
    };
    let seen = Seen::new(true);
    let kept = Kept::default();
    let loan = Cell::new(None);
    let result = submit(&ctx, &w.signer, &seen, &kept, &|view, offer| {
        loan.set(Some((*view, offer)));
    });
    Run {
        result,
        prompt: seen.prompt.into_inner(),
        sent: kept.0.into_inner(),
        loan: loan.get(),
    }
}

async fn chain_time(wallet: &SimWallet) -> u64 {
    wallet.sim.lock().await.next_timestamp()
}

async fn pass_time(wallet: &SimWallet, seconds: u64) {
    wallet.sim.lock().await.pass_time(seconds);
}

/// Send what was approved to the chain and wait for the wallet to see it.
async fn settle(wallet: &mut SimWallet, run: &Run) {
    let bundle = run.sent.clone().expect("nothing was sent");
    wallet
        .push_bundle(bundle)
        .await
        .expect("the chain refused the bundle");
    wallet.wait_for_coins().await;
}

const TERM: u64 = 10_000;

fn xch(amount: u64) -> Part {
    Part {
        asset: AssetId::Xch,
        amount,
    }
}

fn cat(asset_id: Bytes32, amount: u64) -> Part {
    Part {
        asset: AssetId::Cat(asset_id),
        amount,
    }
}

fn mint_params(collateral: Part, strike: Part, expires_at: u64, fee: u64) -> MintParams {
    MintParams {
        collateral,
        strike,
        expires_at,
        creator_address: None,
        loan: None,
        fee,
    }
}

/// Write an option with `wallet`, as `chia_mintOption` does after reading its params.
async fn write(
    wallet: &SimWallet,
    params: &MintParams,
    cats: &[Bytes32],
    setup: &Setup,
) -> (Run, Bytes32) {
    let w = who(wallet).await;
    let minted = build_sage_mint(&wallet.wallet, params)
        .await
        .unwrap()
        .expect("no option built");
    let launcher_id = minted.launcher_id;
    let summary = sage_summary(wallet, cats, &minted.spends).await;
    let run = run(&w, setup, |ctx, signer, approver, broadcaster, _| {
        submit_mint(
            ctx,
            signer,
            approver,
            broadcaster,
            &summary,
            minted,
            params.fee,
        )
    });
    (run, launcher_id)
}

/// The option effect on the prompt, the one with `action`.
fn effect<'a>(run: &'a Run, action: &str) -> &'a Value {
    run.summary()["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == action)
        .unwrap_or_else(|| panic!("no {action} effect in {}", run.summary()["options"]))
}

/// The collateral locked by a mint is an option output, never a payment to someone.
fn assert_collateral_is_locked(run: &Run, amount: u64) {
    let outputs = run.summary()["outputs"].as_array().unwrap();
    let collateral: Vec<&Value> = outputs
        .iter()
        .filter(|o| o["option"]["role"] == "collateral")
        .collect();
    assert_eq!(collateral.len(), 1, "{outputs:?}");
    assert_eq!(collateral[0]["amount"], amount);
    assert!(collateral[0]["recipient"].is_null());
    assert!(
        outputs
            .iter()
            .filter(|o| o["to_user"] == false)
            .all(|o| !o["option"].is_null()),
        "a payment to someone: {outputs:?}"
    );
}

#[tokio::test]
async fn writing_an_xch_option_with_a_cat_strike_locks_the_collateral_and_settles() {
    let mut alice = SimWallet::new(5_000).await;
    let strike_cat = Bytes32::new([0x51; 32]);
    let now = chain_time(&alice).await;
    let params = mint_params(xch(1_000), cat(strike_cat, 500), now + TERM, 7);
    let (run, launcher_id) = write(&alice, &params, &[], &Setup::new(CHIA_MINT_OPTION, now)).await;
    let answer: Value = serde_json::from_str(run.ok()).unwrap();
    assert_eq!(answer["launcherId"], hex0x(launcher_id));
    let mint = effect(&run, "mint");
    assert_eq!(mint["creator_is_user"], true);
    assert_eq!(mint["owner_is_user"], true);
    assert_eq!(mint["funded_by_user"], true);
    assert_eq!(mint["guaranteed"], true);
    assert_eq!(mint["terms"]["collateral"]["amount"], 1_000);
    assert_eq!(mint["terms"]["strike"]["asset"]["type"], "cat");
    assert_eq!(mint["terms"]["expiration_seconds"], now + TERM);
    assert_eq!(
        run.prompt.as_ref().unwrap()["intent"]["kind_verified"],
        true
    );
    assert_eq!(
        run.prompt.as_ref().unwrap()["intent"]["kind"],
        "option.mint"
    );
    assert!(
        run.summary()["unknown_puzzles"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_collateral_is_locked(&run, 1_000);
    settle(&mut alice, &run).await;
    assert!(
        alice
            .wallet
            .db
            .spendable_option(launcher_id)
            .await
            .unwrap()
            .is_some()
    );
    // What leaves the wallet's coins for certain: 1000 locked, the option's mojo, the fee.
    let xch_net = run.summary()["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["asset"]["type"] == "xch")
        .unwrap()["net"]
        .clone();
    assert_eq!(xch_net, -1_008);
    // Sage still counts the locked collateral: the writer may claim it back.
    assert_eq!(alice.wallet.db.xch_balance().await.unwrap(), 5_000 - 1 - 7);
}

/// EXPECTED TO FAIL until the xchonnect follow-up to rc.5 f25f044 (B1: a wallet-built mint
/// whose collateral and launch come from different coins, linked by Sage's
/// `Relation::AssertConcurrent`, is not a guaranteed launch, so `option.mint` does not hold).
#[should_panic(expected = "intent_mismatch")]
#[tokio::test]
async fn writing_a_cat_option_with_an_xch_strike_settles() {
    let mut alice = SimWallet::new(5_000).await;
    let collateral = issue_amount(&mut alice, 1_000).await;
    let now = chain_time(&alice).await;
    let params = mint_params(cat(collateral, 1_000), xch(300), now + TERM, 0);
    let (run, launcher_id) = write(
        &alice,
        &params,
        &[collateral],
        &Setup::new(CHIA_MINT_OPTION, now),
    )
    .await;
    run.ok();
    assert_eq!(
        effect(&run, "mint")["terms"]["collateral"]["asset"]["type"],
        "cat"
    );
    assert_collateral_is_locked(&run, 1_000);
    settle(&mut alice, &run).await;
    assert!(
        alice
            .wallet
            .db
            .spendable_option(launcher_id)
            .await
            .unwrap()
            .is_some()
    );
    // Sage still counts the locked collateral: the writer may claim it back.
    assert_eq!(
        alice.wallet.db.cat_balance(collateral).await.unwrap(),
        1_000
    );
}

/// Write an option and wait for it on the chain: the writer holds it.
async fn written(
    alice: &mut SimWallet,
    collateral: Part,
    strike: Part,
    cats: &[Bytes32],
) -> (Bytes32, u64) {
    let now = chain_time(alice).await;
    let params = mint_params(collateral, strike, now + TERM, 0);
    let (run, launcher_id) = write(alice, &params, cats, &Setup::new(CHIA_MINT_OPTION, now)).await;
    run.ok();
    settle(alice, &run).await;
    (launcher_id, now + TERM)
}

/// Send `launcher_id` from `from` to `to_ph`, as `chia_transferOption` does.
async fn send_option(from: &mut SimWallet, launcher_id: Bytes32, to: Bytes32) -> Run {
    let w = who(from).await;
    let now = chain_time(from).await;
    let (spends, amount) = build_transfer(&from.wallet, launcher_id, to, 0)
        .await
        .unwrap()
        .expect("refused");
    let summary = sage_summary(from, &[], &spends).await;
    run(
        &w,
        &Setup::new(CHIA_TRANSFER_OPTION, now),
        |ctx, signer, approver, broadcaster, _| {
            submit_transfer(
                ctx,
                signer,
                approver,
                broadcaster,
                &summary,
                spends,
                Part {
                    asset: AssetId::Option(launcher_id),
                    amount,
                },
                to,
                0,
            )
        },
    )
}

/// Use `launcher_id` with `holder`'s wallet, as `chia_exerciseOption` does, at `now`.
async fn exercise(holder: &SimWallet, launcher_id: Bytes32, cats: &[Bytes32], now: u64) -> Run {
    let w = who(holder).await;
    let spends = match build_exercise(&holder.wallet, launcher_id, 0, now)
        .await
        .unwrap()
    {
        Ok(spends) => spends,
        Err(refusal) => {
            return Run {
                result: Err(refusal_rpc(refusal)),
                prompt: None,
                sent: None,
                loan: None,
            };
        }
    };
    let summary = sage_summary(holder, cats, &spends).await;
    run(
        &w,
        &Setup::new(CHIA_EXERCISE_OPTION, now),
        |ctx, signer, approver, broadcaster, _| {
            submit_exercise(ctx, signer, approver, broadcaster, &summary, spends, 0)
        },
    )
}

/// Claim back the collateral of `launcher_id` with `creator`'s wallet, as
/// `chia_claimOption` does, at `now`.
async fn claim(
    creator: &SimWallet,
    launcher_id: Bytes32,
    cats: &[Bytes32],
    now: u64,
    fee: u64,
) -> Run {
    let w = who(creator).await;
    let peer = creator.peer().await;
    let built = build_claim(
        &creator.wallet,
        Some(&peer),
        &w.ownership.p2_puzzle_hashes,
        launcher_id,
        fee,
        now,
    )
    .await
    .unwrap();
    let spends = match built {
        Ok(spends) => spends,
        Err(refusal) => {
            return Run {
                result: Err(refusal_rpc(refusal)),
                prompt: None,
                sent: None,
                loan: None,
            };
        }
    };
    let summary = sage_summary(creator, cats, &spends).await;
    run(
        &w,
        &Setup::new(CHIA_CLAIM_OPTION, now),
        |ctx, signer, approver, broadcaster, _| {
            submit_claim(ctx, signer, approver, broadcaster, &summary, spends, fee)
        },
    )
}

/// EXPECTED TO FAIL until the xchonnect follow-up to rc.5 f25f044 (B1: a wallet-built mint
/// whose collateral and launch come from different coins, linked by Sage's
/// `Relation::AssertConcurrent`, is not a guaranteed launch, so `option.mint` does not hold).
/// (Past the mint, its exercise also meets B2.)
#[should_panic(expected = "intent_mismatch")]
#[tokio::test]
async fn a_sent_option_is_used_by_its_new_holder_before_the_expiry() {
    let mut alice = SimWallet::new(5_000).await;
    let mut bob = alice.next(2_000).await;
    let collateral = issue_amount(&mut alice, 1_000).await;
    let (launcher_id, _) =
        written(&mut alice, cat(collateral, 1_000), xch(300), &[collateral]).await;

    let bob_ph = bob.wallet.change_p2_puzzle_hash().await.unwrap();
    let sent = send_option(&mut alice, launcher_id, bob_ph).await;
    sent.ok();
    let transfer = effect(&sent, "transfer");
    assert_eq!(transfer["from_user"], true);
    assert_eq!(transfer["to"], hex::encode(bob_ph));
    assert_eq!(
        sent.prompt.as_ref().unwrap()["intent"]["kind_verified"],
        true
    );
    settle(&mut alice, &sent).await;
    bob.wait_for_puzzles().await;
    assert!(
        bob.wallet
            .db
            .spendable_option(launcher_id)
            .await
            .unwrap()
            .is_some()
    );

    let now = chain_time(&bob).await;
    let used = exercise(&bob, launcher_id, &[collateral], now).await;
    used.ok();
    let effect = effect(&used, "exercise");
    assert_eq!(effect["by_user"], true);
    assert_eq!(effect["terms"]["strike"]["amount"], 300);
    let prompt = used.prompt.as_ref().unwrap();
    assert_eq!(prompt["binding"]["all_bound"], true);
    assert_eq!(prompt["intent"]["kind_verified"], true);
    settle(&mut bob, &used).await;
    assert_eq!(bob.wallet.db.cat_balance(collateral).await.unwrap(), 1_000);
}

#[tokio::test]
async fn an_option_is_not_used_after_its_expiry() {
    let mut alice = SimWallet::new(5_000).await;
    let (launcher_id, expiry) = written(
        &mut alice,
        xch(1_000),
        cat(Bytes32::new([0x52; 32]), 10),
        &[],
    )
    .await;
    let late = exercise(&alice, launcher_id, &[], expiry).await;
    assert!(!late.asked());
    assert!(
        late.reason().contains("option_expired"),
        "{}",
        late.reason()
    );
}

#[tokio::test]
async fn the_writer_claims_the_collateral_back_after_the_expiry() {
    let mut alice = SimWallet::new(5_000).await;
    let (launcher_id, expiry) = written(
        &mut alice,
        xch(1_000),
        cat(Bytes32::new([0x53; 32]), 10),
        &[],
    )
    .await;

    // Too early: refused before anything is built.
    let early = claim(&alice, launcher_id, &[], expiry - 1, 0).await;
    assert!(!early.asked());
    assert!(
        early.reason().contains("option_not_expired"),
        "{}",
        early.reason()
    );

    pass_time(&alice, 2 * TERM).await;
    let now = chain_time(&alice).await;
    let before = alice.wallet.db.xch_balance().await.unwrap();
    let back = claim(&alice, launcher_id, &[], now, 5).await;
    back.ok();
    let clawback = effect(&back, "clawback");
    assert_eq!(clawback["creator_is_user"], true);
    assert_eq!(clawback["collateral"]["amount"], 1_000);
    assert_eq!(
        back.prompt.as_ref().unwrap()["intent"]["kind_verified"],
        true
    );
    settle(&mut alice, &back).await;
    // Sage counted the collateral already: only the fee goes.
    assert_eq!(alice.wallet.db.xch_balance().await.unwrap(), before - 5);
}

/// EXPECTED TO FAIL until the xchonnect follow-up to rc.5 f25f044 (B1: a wallet-built mint
/// whose collateral and launch come from different coins, linked by Sage's
/// `Relation::AssertConcurrent`, is not a guaranteed launch, so `option.mint` does not hold).
#[should_panic(expected = "intent_mismatch")]
#[tokio::test]
async fn a_cat_collateral_comes_back_after_the_expiry() {
    let mut alice = SimWallet::new(5_000).await;
    let collateral = issue_amount(&mut alice, 1_000).await;
    let (launcher_id, _) =
        written(&mut alice, cat(collateral, 1_000), xch(300), &[collateral]).await;
    pass_time(&alice, 2 * TERM).await;
    let now = chain_time(&alice).await;
    let back = claim(&alice, launcher_id, &[collateral], now, 0).await;
    back.ok();
    settle(&mut alice, &back).await;
    assert_eq!(
        alice.wallet.db.cat_balance(collateral).await.unwrap(),
        1_000
    );
}

#[tokio::test]
async fn someone_elses_collateral_is_not_claimed() {
    let mut alice = SimWallet::new(5_000).await;
    let mut bob = alice.next(1_000).await;
    let (launcher_id, _) = written(
        &mut alice,
        xch(1_000),
        cat(Bytes32::new([0x54; 32]), 10),
        &[],
    )
    .await;
    let bob_ph = bob.wallet.change_p2_puzzle_hash().await.unwrap();
    let sent = send_option(&mut alice, launcher_id, bob_ph).await;
    settle(&mut alice, &sent).await;
    bob.wait_for_puzzles().await;
    pass_time(&alice, 2 * TERM).await;
    let now = chain_time(&alice).await;
    // Bob holds the option, but Alice wrote it: the collateral is hers to claim.
    let refused = claim(&bob, launcher_id, &[], now, 0).await;
    assert!(!refused.asked());
    assert!(
        refused.reason().contains("not_creator"),
        "{}",
        refused.reason()
    );
}

// ---------------------------------------------------------------------------------------
// Refusals before the prompt
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_mint_is_refused_without_the_permission_over_the_limit_and_on_the_wrong_network() {
    let alice = SimWallet::new(5_000).await;
    let now = chain_time(&alice).await;
    let params = mint_params(xch(1_000), cat(Bytes32::new([0x55; 32]), 10), now + TERM, 0);

    let mut setup = Setup::new(CHIA_MINT_OPTION, now);
    setup.permissions.methods = vec![CHIA_SEND.to_owned()];
    let (run, _) = write(&alice, &params, &[], &setup).await;
    assert!(!run.asked());
    assert!(
        run.reason().starts_with(&codes::UNAUTHORIZED.to_string()),
        "{}",
        run.reason()
    );

    // The collateral counts against the XCH limit like a payment.
    let mut setup = Setup::new(CHIA_MINT_OPTION, now);
    setup.permissions.limits.insert(
        AssetId::Xch,
        AssetLimit {
            per_request: Some(999),
            per_day: None,
        },
    );
    let (run, _) = write(&alice, &params, &[], &setup).await;
    assert!(!run.asked());
    assert!(run.result.is_err());

    let mut setup = Setup::new(CHIA_MINT_OPTION, now);
    setup.session_chain_id = "mainnet";
    let (run, _) = write(&alice, &params, &[], &setup).await;
    assert!(!run.asked());
    assert!(run.reason().contains("wrong_network"), "{}", run.reason());
}

/// A website's own mint (`xchonnect_submitCoinSpends`) whose collateral goes to a puzzle hash
/// that is not the option's: nothing about it verifies, so it stays an unknown contract and is
/// refused before the user is asked.
#[tokio::test]
async fn a_website_mint_with_a_fake_collateral_stays_unknown_and_is_refused() {
    let alice = SimWallet::new(5_000).await;
    let now = chain_time(&alice).await;
    let params = mint_params(xch(1_000), cat(Bytes32::new([0x56; 32]), 10), now + TERM, 0);
    let minted = build_sage_mint(&alice.wallet, &params)
        .await
        .unwrap()
        .unwrap();
    let w = who(&alice).await;
    let fake = Bytes32::new([0x66; 32]);
    let mut allocator = chia_wallet_sdk::prelude::Allocator::new();
    let mut spends = minted.spends;
    let mut moved = false;
    for spend in &mut spends {
        moved |= edit_user_conditions(&mut allocator, spend, |conditions| {
            for condition in conditions.iter_mut() {
                if let chia_wallet_sdk::prelude::Condition::CreateCoin(cc) = condition
                    && cc.amount == 1_000
                {
                    cc.puzzle_hash = fake;
                }
            }
        });
    }
    assert!(moved);
    let summary = kit::simulate(&spends, &w.ownership, kit::simulate::DEFAULT_MAX_COST).unwrap();
    assert!(summary.options.is_empty());
    assert!(!summary.unknown_puzzles.is_empty());
    let run = run(
        &w,
        &Setup::new(kit::SUBMIT_COIN_SPENDS, now),
        |ctx, signer, approver, broadcaster, _| {
            kit::submit(
                SubmitRequest {
                    method: kit::SUBMIT_COIN_SPENDS,
                    spends,
                    other_signature: None,
                    intent: None,
                    wallet_built: false,
                },
                ctx,
                signer,
                approver,
                broadcaster,
            )
            .map(|s| s.to_json())
        },
    );
    assert!(!run.asked());
    assert!(
        run.reason()
            .starts_with(&codes::UNSUPPORTED_CONTENT.to_string()),
        "{}",
        run.reason()
    );
}

#[test]
fn mint_params_are_read_strictly() {
    let now = 1_000_000;
    let good = r#"{"underlying":{"amount":"1000"},"strike":{"assetId":"0x5151515151515151515151515151515151515151515151515151515151515151","amount":500},"expiresAt":1100000,"fee":7}"#;
    let params = super::options::mint_params(good, now).unwrap();
    assert_eq!(params.collateral, xch(1_000));
    assert_eq!(params.strike, cat(Bytes32::new([0x51; 32]), 500));
    assert_eq!(params.fee, 7);
    for (bad, why) in [
        (
            r#"{"strike":{"amount":5},"expiresAt":1100000}"#,
            "underlying",
        ),
        (
            r#"{"underlying":{"amount":0},"strike":{"assetId":"0x51","amount":5},"expiresAt":1100000}"#,
            "underlying",
        ),
        (
            r#"{"underlying":{"amount":5},"strike":{"amount":5},"expiresAt":1100000}"#,
            "strike",
        ),
        (
            r#"{"underlying":{"amount":5},"strike":{"assetId":"zz","amount":5},"expiresAt":1100000}"#,
            "strike",
        ),
        (
            r#"{"underlying":{"amount":5},"strike":{"assetId":"0x5151515151515151515151515151515151515151515151515151515151515151","amount":5},"expiresAt":1000010}"#,
            "expiresAt",
        ),
        (
            r#"{"underlying":{"amount":5},"strike":{"assetId":"0x5151515151515151515151515151515151515151515151515151515151515151","amount":5},"expiresAt":1100000,"creatorAddress":5}"#,
            "creatorAddress",
        ),
    ] {
        let error = super::options::mint_params(bad, now).unwrap_err();
        assert!(error.data.unwrap_or_default().contains(why), "{bad}");
    }
}

fn refusal_rpc(refusal: Refusal) -> RpcError {
    // The same answer the website gets.
    let error = match refusal {
        Refusal::UnknownOption => "unknown_option",
        Refusal::Expired => "option_expired",
        Refusal::NotExpired => "option_not_expired",
        Refusal::NotCreator => "not_creator",
        Refusal::Fee => "fee",
    };
    RpcError {
        code: codes::UNAUTHORIZED,
        message: error.to_owned(),
        data: Some(format!(r#"{{"reason":"{error}"}}"#)),
    }
}

// ---------------------------------------------------------------------------------------
// Loans
// ---------------------------------------------------------------------------------------

/// A borrower (Alice) with a CAT to lock, a lender (Bob) with XCH, and Alice's signed loan
/// request: borrow 300 mojos, lock 1000 of the CAT, repay 315 mojos before the expiry.
struct Loan {
    alice: SimWallet,
    bob: SimWallet,
    collateral: Bytes32,
    offer: String,
    launcher_id: Bytes32,
    expiry: u64,
    request: Run,
}

async fn loan() -> Loan {
    let mut alice = SimWallet::new(3_000).await;
    let bob = alice.next(3_000).await;
    let collateral = issue_amount(&mut alice, 1_000).await;
    let lender = bob.wallet.change_p2_puzzle_hash().await.unwrap();
    let now = chain_time(&alice).await;
    let params = MintParams {
        creator_address: Some("lender".to_owned()),
        loan: Some(xch(300)),
        ..mint_params(cat(collateral, 1_000), xch(315), now + TERM, 0)
    };
    let built = build_mint(&alice.wallet, Some(lender), &params, Some(xch(300)))
        .await
        .unwrap()
        .expect("no loan request built");
    let launcher_id = built.launcher_id;
    let summary = sage_summary(&alice, &[collateral], &built.spends).await;
    let w = who(&alice).await;
    let request = run(
        &w,
        &Setup::new(CHIA_MINT_OPTION, now),
        |ctx, signer, approver, _, on_loan| {
            submit_loan_request(
                ctx,
                signer,
                approver,
                &summary,
                built,
                lender,
                xch(300),
                0,
                on_loan,
            )
        },
    );
    let answer: Value = serde_json::from_str(request.ok()).unwrap();
    let offer = answer["offer"].as_str().expect("no offer").to_owned();
    Loan {
        alice,
        bob,
        collateral,
        offer,
        launcher_id,
        expiry: now + TERM,
        request,
    }
}

/// Bob lends. Through `chia_takeOffer`'s path the take is refused before he is asked:
/// wallet-kit 0.1.0-rc.5 never verifies `loan.take` (xchonnect TASK-78, audit SA-H2), since
/// a borrower's funding spend can be swapped for one that keeps the money, and nothing in
/// the request ties the collateral to the lender's signature. Follow-up: the loan request
/// must carry an announcement the lender can bind to; then this take is Klimper's again.
/// Until then Bob's own wallet takes the offer (Sage's take, Klimper's `bind_user_spends`,
/// signed by Sage) so the borrower's repayment and the lender's claim stay covered.
async fn lend(loan: &mut Loan) {
    let offer = chia_wallet_sdk::driver::decode_offer(&loan.offer).unwrap();
    know_cats(&loan.bob, &[loan.collateral]).await;
    let outcome = take(
        &loan.bob,
        &[loan.collateral],
        &offer,
        0,
        &super::take_tests::take_permissions(),
        true,
    )
    .await;
    let error = outcome
        .result
        .expect_err("the lender's take was approved: loan.take must not verify (TASK-78)");
    assert!(
        error
            .data
            .as_deref()
            .unwrap_or("")
            .contains("intent_mismatch"),
        "{error:?}"
    );
    assert!(
        !outcome.asked,
        "the lender was asked about an unverified loan"
    );
    assert!(outcome.sent.is_none());

    let (_, _, ownership) = wallet_keys(&loan.bob).await;
    let (spends, maker_signature) = build_take(&loan.bob.wallet, offer, 0).await.unwrap();
    let spends = bind_user_spends(spends, &ownership).unwrap();
    let bundle = loan
        .bob
        .wallet
        .sign_transaction(
            SpendBundle::new(spends, maker_signature),
            &loan.bob.agg_sig,
            loan.bob.master_sk.clone(),
            true,
        )
        .await
        .unwrap();
    loan.bob
        .push_bundle(bundle)
        .await
        .expect("the chain refused the loan");
    loan.bob.wait_for_coins().await;
    // The borrower's wallet learns of the option it holds once it has synced its puzzle.
    loan.alice.wait_for_puzzles().await;
}

/// EXPECTED TO FAIL until the xchonnect follow-up to rc.5 f25f044 (B3: the loan request's
/// two coin sets are linked by `ASSERT_CONCURRENT_SPEND`, as Sage links them, so the launch
/// is not guaranteed and `loan.request` does not hold).
#[should_panic(expected = "intent_mismatch")]
#[tokio::test]
async fn a_loan_request_is_signed_as_an_offer_and_shows_the_loan() {
    let loan = Box::pin(loan()).await;
    // Nothing was sent: the request is an offer for the lender.
    let (view, makes_offer) = loan.request.loan.expect("no loan on the borrower's screen");
    assert!(makes_offer);
    assert_eq!(view.role, "borrower");
    assert_eq!(view.principal.amount, 300);
    let lender = loan.bob.wallet.change_p2_puzzle_hash().await.unwrap();
    assert_eq!(view.counterparty, lender);
    let prompt = loan.request.prompt.as_ref().unwrap();
    assert_eq!(prompt["intent"]["kind"], "loan.request");
    assert_eq!(prompt["intent"]["kind_verified"], true);
    let mint = effect(&loan.request, "mint");
    assert_eq!(mint["creator_is_user"], false);
    assert_eq!(mint["owner_is_user"], true);
    assert_eq!(mint["terms"]["creator_puzzle_hash"], hex::encode(lender));
    assert_eq!(mint["terms"]["strike"]["amount"], 315);
    assert_eq!(mint["launcher_id"], hex::encode(loan.launcher_id));
    assert!(loan.offer.starts_with("offer1"));
}

/// EXPECTED TO FAIL until the xchonnect follow-up to rc.5 f25f044 (B3: the loan request's
/// two coin sets are linked by `ASSERT_CONCURRENT_SPEND`, as Sage links them, so the launch
/// is not guaranteed and `loan.request` does not hold).
#[should_panic(expected = "intent_mismatch")]
#[tokio::test]
async fn a_loan_is_taken_by_the_lender_and_repaid_by_the_borrower() {
    let mut loan = Box::pin(loan()).await;
    let alice_xch = loan.alice.wallet.db.xch_balance().await.unwrap();
    let bob_xch = loan.bob.wallet.db.xch_balance().await.unwrap();
    Box::pin(lend(&mut loan)).await;
    // The principal arrived; the collateral is locked; the tie went to the lender.
    assert_eq!(
        loan.alice
            .wallet
            .db
            .cat_balance(loan.collateral)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        loan.bob.wallet.db.xch_balance().await.unwrap(),
        bob_xch - 300 + LOAN_TIE_MOJOS as u128
    );

    let now = chain_time(&loan.alice).await;
    let repaid = exercise(&loan.alice, loan.launcher_id, &[loan.collateral], now).await;
    repaid.ok();
    assert_eq!(effect(&repaid, "exercise")["creator_is_user"], false);
    settle(&mut loan.alice, &repaid).await;
    assert_eq!(
        loan.alice
            .wallet
            .db
            .cat_balance(loan.collateral)
            .await
            .unwrap(),
        1_000
    );
    // Borrowed 300, repaid 315, the tie; the option's mojo came back when it was melted.
    let after = loan.alice.wallet.db.xch_balance().await.unwrap();
    assert_eq!(after, alice_xch + 300 - 315 - u128::from(LOAN_TIE_MOJOS));
}

/// EXPECTED TO FAIL until the xchonnect follow-up to rc.5 f25f044 (B3: the loan request's
/// two coin sets are linked by `ASSERT_CONCURRENT_SPEND`, as Sage links them, so the launch
/// is not guaranteed and `loan.request` does not hold).
#[should_panic(expected = "intent_mismatch")]
#[tokio::test]
async fn a_loan_not_repaid_is_claimed_by_the_lender_after_the_expiry() {
    let mut loan = Box::pin(loan()).await;
    Box::pin(lend(&mut loan)).await;
    pass_time(&loan.bob, 2 * TERM).await;
    let now = chain_time(&loan.bob).await;
    assert!(now > loan.expiry);
    // Too late to repay.
    let late = exercise(&loan.alice, loan.launcher_id, &[loan.collateral], now).await;
    assert!(
        late.reason().contains("option_expired"),
        "{}",
        late.reason()
    );
    // The lender, who never held the option, reads it from the chain and claims.
    let claimed = claim(&loan.bob, loan.launcher_id, &[loan.collateral], now, 0).await;
    claimed.ok();
    assert_eq!(effect(&claimed, "clawback")["creator_is_user"], true);
    let collateral_coin = claimed.summary()["spends"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"]["type"] == "option_underlying")
        .expect("no collateral spend")["coin_id"]
        .as_str()
        .unwrap()
        .to_owned();
    settle(&mut loan.bob, &claimed).await;
    // On the chain: the collateral is spent, into a coin of the lender's.
    let collateral_coin =
        Bytes32::try_from(hex::decode(collateral_coin).unwrap().as_slice()).unwrap();
    let sim = loan.bob.sim.lock().await;
    assert!(
        sim.coin_state(collateral_coin)
            .unwrap()
            .spent_height
            .is_some()
    );
    let bob = loan.bob.wallet.change_p2_puzzle_hash().await.unwrap();
    let cat_ph = Bytes32::from(
        chia_wallet_sdk::driver::CatInfo::new(loan.collateral, None, bob).puzzle_hash(),
    );
    let claimed_coin = Coin::new(collateral_coin, cat_ph, 1_000);
    assert!(sim.coin_state(claimed_coin.coin_id()).is_some());
}

/// A take whose maker writes an option that is neither the user's to hold nor to claim back
/// buys the user nothing: refused before the user is asked. The summary is wallet-kit's own
/// for a stranger's mint (its option effects cannot be built by hand: `#[non_exhaustive]`).
#[tokio::test]
async fn a_take_that_writes_someone_elses_option_is_refused() {
    let alice = SimWallet::new(1_000).await;
    let stranger = alice.next(5_000).await;
    let now = chain_time(&stranger).await;
    let params = mint_params(xch(1_000), xch(500), now + TERM, 0);
    let minted = build_sage_mint(&stranger.wallet, &params)
        .await
        .unwrap()
        .expect("no option built");
    let w = who(&alice).await;
    let summary = kit::simulate(
        &minted.spends,
        &w.ownership,
        kit::simulate::DEFAULT_MAX_COST,
    )
    .unwrap();
    assert!(
        summary
            .options
            .iter()
            .any(|e| matches!(e, kit::option::OptionEffect::Mint { .. })),
        "no mint in {:?}",
        summary.options
    );
    let error = take_loan(&summary, &minted.spends, &w.ownership).unwrap_err();
    assert!(error.data.unwrap().contains("option_not_yours"));
}

/// Using an XCH-backed option with the writer's own strike CAT, with Sage's
/// `exercise_options` as it builds it (nothing added).
/// EXPECTED TO FAIL until the xchonnect follow-up to rc.5 f25f044 (B2: Sage's
/// `exercise_options` links its spends with `ASSERT_CONCURRENT_SPEND`, which the kit no
/// longer counts, so a wallet-built exercise is refused as `unbound_exercise`).
#[should_panic(expected = "unbound_exercise")]
#[tokio::test]
async fn an_xch_option_is_used_before_the_expiry() {
    let mut alice = SimWallet::new(5_000).await;
    let strike = xch_strike_cat(&mut alice).await;
    let AssetId::Cat(strike_cat) = strike.asset else {
        unreachable!()
    };
    let (launcher_id, _) = written(&mut alice, xch(1_000), strike, &[]).await;
    let now = chain_time(&alice).await;
    let used = exercise(&alice, launcher_id, &[strike_cat], now).await;
    used.ok();
    settle(&mut alice, &used).await;
}

/// A strike in a CAT the wallet holds, so the writer can use its own option.
async fn xch_strike_cat(wallet: &mut SimWallet) -> Part {
    cat(issue_amount(wallet, 500).await, 300)
}

/// Buying an option someone offers for XCH, with `chia_takeOffer`: the option sits at the
/// settlement puzzle in the maker's offer, Klimper ties the taker's spends to it, and the
/// option arrives.
#[tokio::test]
async fn an_offered_option_is_bought_with_xch() {
    let mut alice = SimWallet::new(5_000).await;
    let bob = alice.next(3_000).await;
    let (launcher_id, _) = written(
        &mut alice,
        xch(1_000),
        cat(Bytes32::new([0x57; 32]), 10),
        &[],
    )
    .await;
    let offer = alice
        .wallet
        .make_offer(
            sage_wallet::Offered {
                options: vec![launcher_id],
                ..Default::default()
            },
            sage_wallet::Requested {
                xch: 500,
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    let offer = alice
        .wallet
        .sign_transaction(offer, &alice.agg_sig, alice.master_sk.clone(), true)
        .await
        .unwrap();
    let outcome = take(
        &bob,
        &[],
        &offer,
        0,
        &super::take_tests::take_permissions(),
        true,
    )
    .await;
    if let Err(error) = &outcome.result {
        panic!("refused: {error:?}");
    }
    assert!(outcome.loan.is_none());
    let option_coin = alice
        .wallet
        .db
        .spendable_option(launcher_id)
        .await
        .unwrap()
        .unwrap()
        .coin
        .coin_id();
    let bundle = outcome.sent.expect("nothing sent");
    if let Err(error) = bob.sim.lock().await.new_transaction(bundle.clone()) {
        panic!("the chain refuses the take: {error:?}");
    }
    // On the chain: the offered option coin is spent (into Bob's).
    let spent = bob
        .sim
        .lock()
        .await
        .coin_state(option_coin)
        .and_then(|state| state.spent_height);
    assert!(spent.is_some(), "the take did not settle");
}
