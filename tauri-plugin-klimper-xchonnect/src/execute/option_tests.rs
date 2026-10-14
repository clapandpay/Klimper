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
        options: kit::DEFAULT_OPTION_LIMIT,
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
    /// The wallet's record of signed coins (spec 11.2).
    signed: kit::SignedCoins,
    /// Options that already left the user today (`DailySpend::options`).
    options_today: u128,
}

impl Setup {
    fn new(method: &str, now: u64) -> Self {
        Setup {
            permissions: granted(method),
            session_chain_id: "testnet11",
            now,
            signed: kit::SignedCoins::default(),
            options_today: 0,
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
    *limits.0.borrow_mut() = kit::DailySpend {
        day: setup.now / 86_400,
        options: setup.options_today,
        ..kit::DailySpend::default()
    };
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
        signed_coins: &setup.signed,
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
    let now = chain_time(from).await;
    send_option_with(
        from,
        launcher_id,
        to,
        Setup::new(CHIA_TRANSFER_OPTION, now),
        0,
    )
    .await
}

/// [`send_option`] under `setup` (its permissions and today's option count).
async fn send_option_with(
    from: &mut SimWallet,
    launcher_id: Bytes32,
    to: Bytes32,
    setup: Setup,
    fee: u64,
) -> Run {
    let w = who(from).await;
    let (spends, amount) = build_transfer(&from.wallet, launcher_id, to, fee)
        .await
        .unwrap()
        .expect("refused");
    let summary = sage_summary(from, &[], &spends).await;
    run(&w, &setup, |ctx, signer, approver, broadcaster, _| {
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
            fee,
        )
    })
}

/// Use `launcher_id` with `holder`'s wallet, as `chia_exerciseOption` does, at `now`.
async fn exercise(holder: &SimWallet, launcher_id: Bytes32, cats: &[Bytes32], now: u64) -> Run {
    exercise_paying(holder, launcher_id, cats, now, 0).await
}

/// [`exercise`] with Klimper's own `fee`.
async fn exercise_paying(
    holder: &SimWallet,
    launcher_id: Bytes32,
    cats: &[Bytes32],
    now: u64,
    fee: u64,
) -> Run {
    let w = who(holder).await;
    let spends = match build_exercise(&holder.wallet, launcher_id, fee, now)
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
            submit_exercise(ctx, signer, approver, broadcaster, &summary, spends, fee)
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
                    wallet_fee: 0,
                    signatures_released: true,
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
    // The website's fee is read (and must be well formed), never used: the fee is
    // Klimper's own.
    assert_eq!(params.fee, crate::execute::own_fee());
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
/// Until then Bob's own wallet takes the offer (Sage's take as Sage builds it, signed by
/// Sage) so the borrower's repayment and the lender's claim stay covered.
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

    let (spends, maker_signature) = build_take(&loan.bob.wallet, offer, 0).await.unwrap();
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
    // Its signatures go to the lender: released, held by direct assertions alone.
    assert_eq!(prompt["linking"], "direct");
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

/// With a fee Klimper chose (`wallet_fee`), Sage's exercise is still sealed, and the fee is
/// paid (xchonnect rc.5, H1: the fee credit is at most the wallet's own fee).
///
/// Ignored: Sage's `exercise_options` with a fee spends one of the wallet's XCH coins twice
/// in the bundle it builds (here a 1000-mojo coin), which wallet-kit refuses as an invalid
/// request (4000, `coinSpends`) and the chain would refuse too. Sage is not changed here;
/// run with `--ignored` to reproduce.
#[tokio::test]
#[ignore = "Sage's exercise_options with a fee spends one coin twice"]
async fn an_option_is_used_with_klimpers_fee() {
    let mut alice = SimWallet::new(5_000).await;
    let strike = xch_strike_cat(&mut alice).await;
    let AssetId::Cat(strike_cat) = strike.asset else {
        unreachable!()
    };
    let (launcher_id, _) = written(&mut alice, xch(1_000), strike, &[]).await;
    let now = chain_time(&alice).await;
    let used = exercise_paying(&alice, launcher_id, &[strike_cat], now, 100).await;
    used.ok();
    let prompt = used.prompt.as_ref().unwrap();
    assert_eq!(prompt["linking"], "sealed");
    assert_eq!(prompt["summary"]["reserve_fee"], 100);
    settle(&mut alice, &used).await;
}

/// Sage's mint with a fee Klimper chose (`wallet_fee`) is sealed, and the chain takes it.
#[tokio::test]
async fn an_option_is_written_with_klimpers_fee() {
    let mut alice = SimWallet::new(5_000).await;
    let now = chain_time(&alice).await;
    let params = mint_params(
        xch(1_000),
        cat(Bytes32::new([0x58; 32]), 10),
        now + TERM,
        100,
    );
    let (run, _) = write(&alice, &params, &[], &Setup::new(CHIA_MINT_OPTION, now)).await;
    run.ok();
    let prompt = run.prompt.as_ref().unwrap();
    assert_eq!(prompt["linking"], "sealed");
    assert_eq!(prompt["summary"]["reserve_fee"], 100);
    settle(&mut alice, &run).await;
}

/// Options leaving the user have a limit of their own (spec 11.1 item 5): one per request
/// and five a day by default, refused before the user is asked (4029).
#[tokio::test]
async fn sending_options_is_limited() {
    let mut alice = SimWallet::new(5_000).await;
    let bob = alice.next(2_000).await;
    let (launcher_id, _) = written(
        &mut alice,
        xch(1_000),
        cat(Bytes32::new([0x57; 32]), 10),
        &[],
    )
    .await;
    let bob_ph = bob.wallet.change_p2_puzzle_hash().await.unwrap();
    let now = chain_time(&alice).await;

    // The day's five already left: refused.
    let mut setup = Setup::new(CHIA_TRANSFER_OPTION, now);
    assert_eq!(setup.permissions.options, kit::DEFAULT_OPTION_LIMIT);
    setup.options_today = kit::DEFAULT_OPTIONS_PER_DAY;
    let refused = send_option_with(&mut alice, launcher_id, bob_ph, setup, 0).await;
    assert!(!refused.asked());
    assert_eq!(
        refused.reason(),
        format!(
            "{} {}",
            codes::LIMIT_EXCEEDED,
            r#"{"reason":"options_per_day"}"#
        )
    );
    let explained = crate::requests::explain(refused.result.unwrap_err());
    assert!(
        explained.message.contains("option limit"),
        "{}",
        explained.message
    );

    // No option may leave in one request.
    let mut setup = Setup::new(CHIA_TRANSFER_OPTION, now);
    setup.permissions.options.per_request = Some(0);
    let refused = send_option_with(&mut alice, launcher_id, bob_ph, setup, 0).await;
    assert!(!refused.asked());
    assert!(
        refused.reason().contains("options_per_request"),
        "{}",
        refused.reason()
    );

    // Under the default, one goes, sealed, with Klimper's own fee.
    let sent = send_option_with(
        &mut alice,
        launcher_id,
        bob_ph,
        Setup::new(CHIA_TRANSFER_OPTION, now),
        100,
    )
    .await;
    sent.ok();
    let prompt = sent.prompt.as_ref().unwrap();
    assert_eq!(prompt["linking"], "sealed");
    assert_eq!(prompt["summary"]["reserve_fee"], 100);
    // The chain takes it.
    alice
        .push_bundle(sent.sent.clone().unwrap())
        .await
        .expect("the chain refused the bundle");
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

// ---------------------------------------------------------------------------------------
// The record of signed coins (xchonnect spec 11.2)
// ---------------------------------------------------------------------------------------

use crate::signed_coins::{Recording, SignedCoinsHandle, spent_coins};
use crate::store::{MemorySecrets, SecretStore, SessionStore};

/// An empty record of signed coins, in a session store of its own.
fn record_of_signed() -> (tempfile::TempDir, SignedCoinsHandle) {
    let dir = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(
        crate::store::SessionStore::open(
            dir.path(),
            Box::new(crate::store::MemorySecrets::default()),
        )
        .unwrap(),
    );
    (dir, SignedCoinsHandle::new(store, 1, "testnet11"))
}

/// `signCoinSpends` params for `spends`, as a website sends them.
fn sign_params(spends: &[CoinSpend]) -> String {
    let spends: Vec<Value> = spends
        .iter()
        .map(|cs| {
            serde_json::json!({
                "coin": {
                    "parent_coin_info": hex0x(cs.coin.parent_coin_info),
                    "puzzle_hash": hex0x(cs.coin.puzzle_hash),
                    "amount": cs.coin.amount,
                },
                "puzzle_reveal": hex0x(cs.puzzle_reveal.as_ref()),
                "solution": hex0x(cs.solution.as_ref()),
            })
        })
        .collect();
    serde_json::json!({ "coinSpends": spends }).to_string()
}

/// Sage's mint of a CAT-backed option (collateral and launch from different coins, linked by
/// `ASSERT_CONCURRENT_SPEND`) holds only while sealed. Once a website holds a signature over
/// one of those coins (a `signCoinSpends` answered earlier), the same request is released:
/// only direct links count, the launch is not guaranteed and `option.mint` does not hold.
#[tokio::test]
async fn coins_signed_for_a_website_make_a_later_wallet_built_request_strict() {
    let mut alice = SimWallet::new(5_000).await;
    let collateral = issue_amount(&mut alice, 1_000).await;
    let now = chain_time(&alice).await;
    let params = mint_params(cat(collateral, 1_000), xch(300), now + TERM, 0);
    let w = who(&alice).await;
    let (_dir, record) = record_of_signed();

    // Sealed, with an empty record: Sage's mint passes as Sage builds it.
    let minted = build_sage_mint(&alice.wallet, &params)
        .await
        .unwrap()
        .unwrap();
    let summary = sage_summary(&alice, &[collateral], &minted.spends).await;
    let spends = minted.spends.clone();
    let sealed = run(&w, &Setup::new(CHIA_MINT_OPTION, now), |ctx, s, a, b, _| {
        submit_mint(ctx, s, a, b, &summary, minted, 0)
    });
    sealed.ok();
    assert_eq!(sealed.prompt.as_ref().unwrap()["linking"], "sealed");

    // A website had the user sign spends of the same coins: wallet-kit reports the coins
    // to Klimper's record before the signature is returned.
    let answered = run(&w, &Setup::new("signCoinSpends", now), |ctx, s, a, b, _| {
        let recording = Recording::new(b, &record);
        kit::handle_with_host(
            "signCoinSpends",
            &sign_params(&spends),
            ctx,
            &kit::Host {
                signer: s,
                approver: a,
                broadcaster: Some(&recording),
            },
        )
    });
    answered.ok();
    let signed = record.load().unwrap().to_kit().unwrap();
    assert!(signed.sealed.is_empty());
    let user_coins = crate::signed_coins::user_coins(&spends, &w.ownership);
    assert!(!user_coins.is_empty());
    assert!(user_coins.iter().all(|c| signed.released.contains(c)));

    let minted = build_sage_mint(&alice.wallet, &params)
        .await
        .unwrap()
        .unwrap();
    let summary = sage_summary(&alice, &[collateral], &minted.spends).await;
    let mut setup = Setup::new(CHIA_MINT_OPTION, now);
    setup.signed = signed;
    let strict = run(&w, &setup, |ctx, s, a, b, _| {
        submit_mint(ctx, s, a, b, &summary, minted, 0)
    });
    assert!(!strict.asked());
    assert!(
        strict.reason().contains("intent_mismatch"),
        "{}",
        strict.reason()
    );
}

/// Every BLS signature `keys` make for `spends`, per coin: the key and the exact message.
fn our_signatures(
    spends: &[CoinSpend],
    keys: &HashSet<PublicKey>,
) -> Vec<(Bytes32, PublicKey, Vec<u8>)> {
    use chia_wallet_sdk::signer::RequiredSignature;
    let constants = crate::execute::agg_sig_constants(&kit::Network::Testnet11);
    let mut allocator = chia_wallet_sdk::prelude::Allocator::new();
    let mut out = Vec::new();
    for cs in spends {
        for required in RequiredSignature::from_coin_spend(&mut allocator, cs, &constants).unwrap()
        {
            if let RequiredSignature::Bls(bls) = required
                && keys.contains(&bls.public_key)
            {
                out.push((cs.coin.coin_id(), bls.public_key, bls.message()));
            }
        }
    }
    out
}

/// The coins the record holds sealed signatures for (by bundle).
fn held(signed: &kit::SignedCoins) -> HashSet<Bytes32> {
    signed.sealed_bundles.values().flatten().copied().collect()
}

/// Refuses every bundle, as a network that does not take it.
struct Refusing;

impl Broadcaster for Refusing {
    fn record(&self, _record: &kit::BroadcastRecord) -> Result<(), String> {
        Ok(())
    }
    fn submit(&self, _bundle: &SpendBundle) -> Result<kit::Broadcast, String> {
        Err("refused by the network".to_owned())
    }
}

/// A sealed bundle Klimper broadcasts is recorded before it leaves: every coin released, and
/// per coin of the user's the exact signatures made for it (their aggregate is the bundle's
/// signature). Its bytes are kept and can be sent again unchanged. Its signatures are never
/// made again: the same spends are refused in a wallet-built request, a `signCoinSpends` and a
/// website's bundle (`sealed_signature`), before anything is shown. Once its coins are spent
/// on chain they leave the released coins and the kept bundle is dropped; its sealed
/// signatures stay (Klimper never prunes them), and the next mint is sealed again.
#[tokio::test]
async fn a_sealed_bundles_signatures_are_recorded_and_never_made_again() {
    let mut alice = SimWallet::new(5_000).await;
    let now = chain_time(&alice).await;
    let params = mint_params(xch(1_000), cat(Bytes32::new([0x58; 32]), 10), now + TERM, 0);
    let w = who(&alice).await;
    let (_dir, record) = record_of_signed();

    let minted = build_sage_mint(&alice.wallet, &params)
        .await
        .unwrap()
        .unwrap();
    let summary = sage_summary(&alice, &[], &minted.spends).await;
    let again = minted.clone();
    let first = run(&w, &Setup::new(CHIA_MINT_OPTION, now), |ctx, s, a, b, _| {
        let recording = Recording::new(b, &record);
        submit_mint(ctx, s, a, &recording, &summary, minted, 0)
    });
    first.ok();
    assert_eq!(first.prompt.as_ref().unwrap()["linking"], "sealed");
    let bundle = first.sent.clone().unwrap();
    let signed = record.load().unwrap().to_kit().unwrap();
    let signed_coin_count = held(&signed).len();
    for cs in &bundle.coin_spends {
        assert!(signed.released.contains(&cs.coin.coin_id()));
    }
    let ours = our_signatures(&bundle.coin_spends, &w.keys);
    let user_coins = crate::signed_coins::user_coins(&bundle.coin_spends, &w.ownership);
    assert!(!user_coins.is_empty());
    assert!(user_coins.iter().all(|c| held(&signed).contains(c)));
    for (coin, key, message) in &ours {
        assert!(signed.sealed[&kit::sealed_digest(key, message)].contains(&bundle.name()));
        assert!(signed.sealed_bundles[&bundle.name()].contains(coin));
    }
    assert_eq!(
        signed.sealed.values().map(HashSet::len).sum::<usize>(),
        ours.len()
    );
    let signatures: Vec<chia_bls::Signature> = ours
        .iter()
        .map(|(_, key, message)| kit::Signer::sign(&w.signer, key, message).unwrap())
        .collect();
    assert_eq!(
        chia_bls::aggregate(&signatures),
        bundle.aggregated_signature
    );

    // Kept, and sent again byte for byte without signing anything.
    let resent = Kept::default();
    let answer = crate::signed_coins::resend(&record, &bundle.name(), |b| resent.submit(b))
        .unwrap()
        .expect("the bundle was not kept");
    assert!(answer.is_ok());
    assert_eq!(resent.0.into_inner(), Some(bundle.clone()));

    // The same mint again, exactly as built: refused before it is shown.
    let spends = again.spends.clone();
    let mut setup = Setup::new(CHIA_MINT_OPTION, now);
    setup.signed = signed.clone();
    let refused = run(&w, &setup, |ctx, s, a, b, _| {
        submit_mint(ctx, s, a, b, &summary, again, 0)
    });
    assert!(!refused.asked());
    assert!(
        refused.reason().contains("4001") && refused.reason().contains("sealed_signature"),
        "{}",
        refused.reason()
    );
    assert!(refused.sent.is_none());

    // A website asks for the same signatures, alone or as its own bundle.
    let mut setup = Setup::new("signCoinSpends", now);
    setup.signed = signed.clone();
    let alone = run(&w, &setup, |ctx, s, a, _, _| {
        kit::handle("signCoinSpends", &sign_params(&spends), ctx, s, a)
    });
    assert!(!alone.asked());
    assert!(
        alone.reason().contains("sealed_signature"),
        "{}",
        alone.reason()
    );
    let mut setup = Setup::new(kit::SUBMIT_COIN_SPENDS, now);
    setup.signed = signed;
    let website = run(&w, &setup, |ctx, s, a, b, _| {
        kit::submit(
            SubmitRequest {
                method: kit::SUBMIT_COIN_SPENDS,
                spends,
                other_signature: None,
                intent: None,
                wallet_built: false,
                wallet_fee: 0,
                signatures_released: true,
            },
            ctx,
            s,
            a,
            b,
        )
        .map(|s| s.to_json())
    });
    assert!(!website.asked());
    assert!(
        website.reason().contains("sealed_signature"),
        "{}",
        website.reason()
    );
    assert!(website.sent.is_none());
    let explained = crate::requests::explain(website.result.unwrap_err());
    assert!(explained.message.contains("never signs it again"));

    // The bundle settles. A coin leaves the record only once the wallet's own synced
    // database has its spend `CONFIRMATIONS` blocks below the peak; the launcher and the eve,
    // which the database does not hold, stay (no peer is asked).
    settle(&mut alice, &first).await;
    let coins = record.coins().unwrap();
    let ids: Vec<String> = coins.iter().map(hex::encode).collect();
    let rows = alice.wallet.db.coins_by_ids(&ids).await.unwrap();
    let known: Vec<Bytes32> = rows.iter().map(|row| row.coin.coin_id()).collect();
    assert!(!known.is_empty());
    let spent_at = rows
        .iter()
        .map(|row| row.spent_height.expect("the bundle did not settle"))
        .max()
        .unwrap();
    let peak = |height: u32| {
        let db = alice.wallet.db.clone();
        async move {
            db.insert_block(height, Bytes32::new([0xbb; 32]), None, true)
                .await
                .unwrap();
        }
    };
    let synced = alice.wallet.db.latest_peak().await.unwrap().map(|(h, _)| h);
    if synced.is_none_or(|h| h < spent_at + crate::signed_coins::CONFIRMATIONS - 1) {
        peak(spent_at + crate::signed_coins::CONFIRMATIONS - 1).await;
        let shallow = spent_coins(&alice.wallet, &coins).await;
        assert!(
            shallow.is_empty(),
            "pruned before {} confirmations",
            crate::signed_coins::CONFIRMATIONS
        );
    }
    peak(spent_at + crate::signed_coins::CONFIRMATIONS).await;
    let spent = spent_coins(&alice.wallet, &coins).await;
    assert_eq!(spent, known.iter().copied().collect());
    // What the database does not hold (the launcher, the eve, a coin created and spent
    // inside the bundle) stays recorded.
    let unknown: HashSet<Bytes32> = coins
        .iter()
        .filter(|c| !known.contains(c))
        .copied()
        .collect();
    let pruned = record.prune(&spent).unwrap().to_kit().unwrap();
    assert_eq!(pruned.released, unknown);
    // Sealed signatures are never pruned.
    assert_eq!(held(&pruned).len(), signed_coin_count);
    for (_, key, message) in &ours {
        assert!(pruned.is_sealed(key, message));
    }
    assert!(record.sealed_bundles().unwrap().bundles.is_empty());
    assert!(
        crate::signed_coins::resend(&record, &bundle.name(), resent_never)
            .unwrap()
            .is_none()
    );

    // With the record pruned, the next mint is sealed again.
    let next = build_sage_mint(&alice.wallet, &params)
        .await
        .unwrap()
        .unwrap();
    let summary = sage_summary(&alice, &[], &next.spends).await;
    let mut setup = Setup::new(CHIA_MINT_OPTION, now);
    setup.signed = record.load().unwrap().to_kit().unwrap();
    let sealed = run(&w, &setup, |ctx, s, a, b, _| {
        submit_mint(ctx, s, a, b, &summary, next, 0)
    });
    sealed.ok();
    assert_eq!(sealed.prompt.as_ref().unwrap()["linking"], "sealed");
}

fn resent_never(_bundle: &SpendBundle) -> Result<kit::Broadcast, String> {
    panic!("nothing is kept to send")
}

/// A sealed bundle the network refuses is recorded all the same (it may have reached other
/// nodes): its signatures are never made again, and it is kept to send again.
#[tokio::test]
async fn a_sealed_bundle_the_network_refuses_is_recorded() {
    let alice = SimWallet::new(5_000).await;
    let now = chain_time(&alice).await;
    let params = mint_params(xch(1_000), cat(Bytes32::new([0x58; 32]), 10), now + TERM, 0);
    let w = who(&alice).await;
    let (_dir, record) = record_of_signed();
    let minted = build_sage_mint(&alice.wallet, &params)
        .await
        .unwrap()
        .unwrap();
    let summary = sage_summary(&alice, &[], &minted.spends).await;
    let spends = minted.spends.clone();
    let refused = run(&w, &Setup::new(CHIA_MINT_OPTION, now), |ctx, s, a, _, _| {
        let recording = Recording::new(&Refusing, &record);
        submit_mint(ctx, s, a, &recording, &summary, minted, 0)
    });
    assert!(refused.asked());
    assert!(
        refused.reason().contains("broadcast_failed"),
        "{}",
        refused.reason()
    );
    let signed = record.load().unwrap().to_kit().unwrap();
    let ours = our_signatures(&spends, &w.keys);
    assert!(!ours.is_empty());
    for (_, key, message) in &ours {
        assert!(signed.is_sealed(key, message));
    }
    assert_eq!(record.sealed_bundles().unwrap().bundles.len(), 1);
}

/// Fails every write, as a keyring that is locked or full.
#[derive(Debug, Default)]
struct Unwritable(MemorySecrets);

impl SecretStore for Unwritable {
    fn put(&self, _key: &str, _bytes: &[u8]) -> crate::error::Result<()> {
        Err(crate::error::Error::internal("the keyring is locked"))
    }
    fn get(&self, key: &str) -> crate::error::Result<Option<Vec<u8>>> {
        self.0.get(key)
    }
    fn delete(&self, key: &str) -> crate::error::Result<()> {
        self.0.delete(key)
    }
}

/// When the record cannot be written, nothing is broadcast (4000 `record_failed`).
#[tokio::test]
async fn a_record_that_cannot_be_written_sends_nothing() {
    let alice = SimWallet::new(5_000).await;
    let now = chain_time(&alice).await;
    let params = mint_params(xch(1_000), cat(Bytes32::new([0x58; 32]), 10), now + TERM, 0);
    let w = who(&alice).await;
    let dir = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(
        SessionStore::open(dir.path(), Box::new(Unwritable::default())).unwrap(),
    );
    let record = SignedCoinsHandle::new(store, 1, "testnet11");
    let minted = build_sage_mint(&alice.wallet, &params)
        .await
        .unwrap()
        .unwrap();
    let summary = sage_summary(&alice, &[], &minted.spends).await;
    let failed = run(&w, &Setup::new(CHIA_MINT_OPTION, now), |ctx, s, a, b, _| {
        let recording = Recording::new(b, &record);
        submit_mint(ctx, s, a, &recording, &summary, minted, 0)
    });
    assert!(
        failed.reason().starts_with("4000") && failed.reason().contains("record_failed"),
        "{}",
        failed.reason()
    );
    assert!(failed.sent.is_none(), "broadcast without a record");
    let explained = crate::requests::explain(failed.result.unwrap_err());
    assert!(explained.message.contains("nothing was sent"));
}

/// What Sage signs through its own screens is recorded too (spec 11.2): the coins of an
/// offer Sage made, read from Sage's database, unchanged.
#[tokio::test]
async fn the_coins_of_an_offer_sage_made_count_as_released() {
    let loan = Box::pin(loan()).await;
    let w = who(&loan.alice).await;
    let offer = chia_wallet_sdk::driver::decode_offer(&loan.offer).unwrap();
    let ours = crate::signed_coins::user_coins(&offer.coin_spends, &w.ownership);
    assert!(
        crate::signed_coins::signed_by_sage(&loan.alice.wallet)
            .await
            .unwrap()
            .iter()
            .all(|c| !ours.contains(c))
    );
    // As if the user had made this offer in Sage's own offer screen.
    let mut tx = loan.alice.wallet.db.tx().await.unwrap();
    tx.insert_offer(sage_database::OfferRow {
        offer_id: offer.name(),
        encoded_offer: loan.offer.clone(),
        expiration_height: None,
        expiration_timestamp: None,
        fee: 0,
        status: sage_database::OfferStatus::Active,
        inserted_timestamp: 0,
    })
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let by_sage = crate::signed_coins::signed_by_sage(&loan.alice.wallet)
        .await
        .unwrap();
    let held: Vec<Bytes32> = ours
        .iter()
        .filter(|c| by_sage.contains(c))
        .copied()
        .collect();
    assert!(!held.is_empty(), "{ours:?} {by_sage:?}");
    let (_dir, record) = record_of_signed();
    record.release(by_sage).unwrap();
    assert!(held.iter().all(|c| {
        record
            .load()
            .unwrap()
            .to_kit()
            .unwrap()
            .released
            .contains(c)
    }));
}

/// Only a coin seen spent leaves the record: a released coin that is still unspent stays.
#[tokio::test]
async fn an_unspent_released_coin_stays_in_the_record() {
    let alice = SimWallet::new(5_000).await;
    let coin = alice.wallet.db.selectable_xch_coins().await.unwrap()[0];
    let (_dir, record) = record_of_signed();
    record.release(vec![coin.coin_id()]).unwrap();
    let spent = spent_coins(&alice.wallet, &record.load().unwrap().coins()).await;
    assert!(spent.is_empty());
    let kept = record.prune(&spent).unwrap();
    assert!(kept.to_kit().unwrap().released.contains(&coin.coin_id()));
}

/// A loan request's signed offer goes to the website: every coin of the borrower's in it is
/// released before the answer leaves (spec 11.2), none is in flight (nothing was sent).
#[tokio::test]
async fn a_loan_requests_offer_releases_the_borrowers_coins() {
    let loan = Box::pin(loan()).await;
    let w = who(&loan.alice).await;
    let (_dir, record) = record_of_signed();
    crate::requests::record_released(CHIA_MINT_OPTION, loan.request.ok(), &w.ownership, &record)
        .unwrap();
    let signed = record.load().unwrap().to_kit().unwrap();
    let offer = chia_wallet_sdk::driver::decode_offer(&loan.offer).unwrap();
    let borrower = crate::signed_coins::user_coins(&offer.coin_spends, &w.ownership);
    assert!(!borrower.is_empty());
    assert!(borrower.iter().all(|c| signed.released.contains(c)));
    assert!(signed.sealed.is_empty());
}
