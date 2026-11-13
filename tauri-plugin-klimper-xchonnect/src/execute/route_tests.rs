// SPDX-License-Identifier: AGPL-3.0-or-later
//! Taking a route end to end (TASK-168): a whole offer and fills of partial offers, each made
//! by its own Sage wallet on a simulated chain, taken through `chia_takeRoute`'s own path
//! ([`run`]: the parameters, the legs' checks, the lookup on the chain, the key-free build,
//! Sage's summary, the cross-check and wallet-kit's submit). The bundle goes to the chain and
//! must settle, all of it, with every balance as the legs said; and every refusal is one the
//! user is never asked about.

use std::cell::{Cell, RefCell};

use chia_sdk_next::PartialOfferAsset;
use chia_wallet_sdk::driver::encode_offer;
use sage_wallet::{Offered, Requested};
use serde_json::json;
use tauri_plugin_klimper_partial_offers::{PartialOffer, Terms, wallet as partial_wallet};
use xchonnect_wallet_kit::permissions::{AssetLimit, DappPermissions};

use super::option_tests::{chain_time, pass_time};
use super::sage_sim::SimWallet;
use super::take_route::{
    BuiltRoute, CHIA_TAKE_ROUTE, EXPIRES_AFTER, MAX_FILL_FEES, MAX_LEGS, RouteView,
    allowed_unknown, build_route, check_fills, checked_offers, counts_as_failed_build, read_fill,
    route_params, submit_route,
};
use super::take_tests::{Kept, Limits, issue_amount, know_cats, reason, sage_summary, wallet_keys};
use super::*;
use crate::trade::{Leg, TradeOverview};

/// Three makers and a taker on one chain.
///
/// * `whole`: 1000 of CAT `a` for 400 mojos, a whole offer.
/// * `cat_partial`: 1000 of CAT `b` for 500 mojos, a partial offer (its coin is a CAT).
/// * `xch_partial`: 600 mojos for 1200 of the taker's CAT `t`, a partial offer that offers
///   XCH (its coin is the bare partial puzzle) and requires a fee of 10 for each fill.
struct Market {
    second: SimWallet,
    third: SimWallet,
    taker: SimWallet,
    a: Bytes32,
    b: Bytes32,
    t: Bytes32,
    whole: SpendBundle,
    cat_partial: PartialOffer,
    xch_partial: PartialOffer,
}

impl Market {
    fn cats(&self) -> [Bytes32; 3] {
        [self.a, self.b, self.t]
    }
}

async fn partial(maker: &SimWallet, terms: Terms) -> PartialOffer {
    partial_wallet::make(&maker.partial_ctx().await, &terms)
        .await
        .expect("the partial offer was not made")
}

async fn cat_for_xch(maker: &SimWallet, cat: Bytes32, expiration: Option<u64>) -> PartialOffer {
    partial(
        maker,
        Terms {
            offered: PartialOfferAsset::Cat {
                asset_id: cat,
                hidden_puzzle_hash: None,
            },
            offered_amount: 1_000,
            requested: PartialOfferAsset::Xch,
            requested_amount: 500,
            min_fill_requested: 50,
            expiration,
            required_fee: None,
            maker_puzzle_hash: maker.wallet.change_p2_puzzle_hash().await.unwrap(),
        },
    )
    .await
}

async fn xch_for_cat(maker: &SimWallet, cat: Bytes32, required_fee: u64) -> PartialOffer {
    partial(
        maker,
        Terms {
            offered: PartialOfferAsset::Xch,
            offered_amount: 600,
            requested: PartialOfferAsset::Cat {
                asset_id: cat,
                hidden_puzzle_hash: None,
            },
            requested_amount: 1_200,
            min_fill_requested: 100,
            expiration: None,
            required_fee: Some(required_fee),
            maker_puzzle_hash: maker.wallet.change_p2_puzzle_hash().await.unwrap(),
        },
    )
    .await
}

async fn market() -> Market {
    let mut first = SimWallet::new(10_000).await;
    let mut taker = first.next(6_000).await;
    let mut second = taker.next(3_000).await;
    let third = second.next(3_000).await;

    let a = issue_amount(&mut first, 1_000).await;
    let b = issue_amount(&mut second, 1_000).await;
    let t = issue_amount(&mut taker, 2_000).await;

    let whole = first
        .wallet
        .make_offer(
            Offered {
                cats: indexmap::indexmap! { a => 1_000 },
                ..Default::default()
            },
            Requested {
                xch: 400,
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    let whole = first
        .wallet
        .sign_transaction(whole, &first.agg_sig, first.master_sk.clone(), true)
        .await
        .unwrap();
    let cat_partial = cat_for_xch(&second, b, None).await;
    let xch_partial = xch_for_cat(&third, t, 10).await;
    know_cats(&taker, &[a, b, t]).await;
    Market {
        second,
        third,
        taker,
        a,
        b,
        t,
        whole,
        cat_partial,
        xch_partial,
    }
}

/// The request a website sends.
fn request(offers: &[&SpendBundle], fills: &[(&PartialOffer, u64)]) -> String {
    let offers: Vec<String> = offers
        .iter()
        .map(|offer| encode_offer(offer).unwrap())
        .collect();
    let fills: Vec<Value> = fills
        .iter()
        .map(|(offer, amount)| json!({ "offer": offer.to_text().unwrap(), "amount": amount }))
        .collect();
    json!({ "offers": offers, "fills": fills }).to_string()
}

/// The same with the fills' texts given as they are, for texts the driver would not write.
fn request_texts(offers: &[&SpendBundle], fills: &[(String, u64)]) -> String {
    let offers: Vec<String> = offers
        .iter()
        .map(|offer| encode_offer(offer).unwrap())
        .collect();
    let fills: Vec<Value> = fills
        .iter()
        .map(|(offer, amount)| json!({ "offer": offer, "amount": amount }))
        .collect();
    json!({ "offers": offers, "fills": fills }).to_string()
}

/// Answers the approval and remembers what the screen would have shown.
struct Screen {
    approves: bool,
    legs: Cell<usize>,
    asked: Cell<bool>,
    trade: RefCell<Option<TradeOverview>>,
    linking: RefCell<Option<String>>,
    unknown: RefCell<Vec<String>>,
}

impl kit::Approver for Screen {
    fn approve(&self, prompt: &kit::Prompt<'_>) -> bool {
        self.asked.set(true);
        let payload = serde_json::to_value(prompt).unwrap_or_default();
        *self.linking.borrow_mut() = payload
            .get("linking")
            .and_then(Value::as_str)
            .map(str::to_owned);
        *self.unknown.borrow_mut() = payload["summary"]["unknown_puzzles"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|hash| hash.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        *self.trade.borrow_mut() = crate::trade::of_prompt(prompt, Some(self.legs.get()));
        self.approves
    }
}

/// What a route ended in.
struct Run {
    result: Result<Value, RpcError>,
    asked: bool,
    sent: Option<SpendBundle>,
    trade: Option<TradeOverview>,
    linking: Option<String>,
    /// The unknown puzzles wallet-kit listed on the approval.
    unknown: Vec<String>,
    view: Option<RouteView>,
}

impl Run {
    fn refused(error: RpcError) -> Self {
        Run {
            result: Err(error),
            asked: false,
            sent: None,
            trade: None,
            linking: None,
            unknown: Vec::new(),
            view: None,
        }
    }

    fn reason(&self) -> String {
        reason(self.result.as_ref().expect_err("the route was not refused"))
    }
}

fn route_permissions() -> DappPermissions {
    DappPermissions {
        methods: vec![CHIA_TAKE_ROUTE.to_owned()],
        exposed_keys: vec![],
        limits: BTreeMap::new(),
        options: kit::DEFAULT_OPTION_LIMIT,
    }
}

struct With<'a> {
    fee: u64,
    permissions: &'a DappPermissions,
    approves: bool,
    /// Changes the built route before it is checked and submitted.
    tamper: &'a dyn Fn(&mut BuiltRoute),
}

/// Take a route with the taker's wallet the way `take_route` does after it has Sage's
/// state: the same functions in the same order.
async fn run_with(taker: &SimWallet, cats: &[Bytes32], params: &str, with: With<'_>) -> Run {
    let now = chain_time(taker).await;
    let (signer, public_keys, ownership) = wallet_keys(taker).await;
    let limits = Limits::default();
    let ctx = RequestContext {
        dapp: "pengui.test",
        network: kit::Network::Testnet11,
        session_chain_id: "testnet11",
        permissions: with.permissions,
        allow_agg_sig_unsafe: false,
        allow_unknown_contracts: false,
        ownership: &ownership,
        keys: &public_keys,
        limits: &limits,
        now,
        chain: None,
        signed_coins: &kit::SignedCoins::default(),
    };
    let params = match route_params(params) {
        Ok(params) => params,
        Err(error) => return Run::refused(error),
    };
    let offers = match checked_offers(&params.offers, &ctx) {
        Ok(offers) => offers,
        Err(error) => return Run::refused(error),
    };
    let wallet = taker.partial_ctx().await;
    let mut fills = Vec::new();
    for param in &params.fills {
        match read_fill(&wallet, param, now)
            .await
            .expect("the chain could not be read")
        {
            Ok(fill) => fills.push(fill),
            Err(error) => return Run::refused(error),
        }
    }
    if let Err(error) = check_fills(&fills, &ctx) {
        return Run::refused(error);
    }
    let mut built = match build_route(
        &taker.wallet,
        &offers,
        &fills,
        with.fee,
        now + EXPIRES_AFTER,
    )
    .await
    .expect("Sage could not build the route")
    {
        Ok(built) => built,
        Err(error) => return Run::refused(error),
    };
    (with.tamper)(&mut built);
    let summary = sage_summary(taker, cats, &built.spends).await;
    let screen = Screen {
        approves: with.approves,
        legs: Cell::new(0),
        asked: Cell::new(false),
        trade: RefCell::new(None),
        linking: RefCell::new(None),
        unknown: RefCell::new(Vec::new()),
    };
    let kept = Kept::default();
    let view = RefCell::new(None);
    let result = submit_route(
        &ctx,
        &signer,
        &screen,
        &kept,
        &summary,
        built,
        with.fee,
        &|route| {
            screen.legs.set(route.offers + route.fills);
            *view.borrow_mut() = Some(route.clone());
        },
    );
    Run {
        result: result.map(|answer| serde_json::from_str(&answer).unwrap()),
        asked: screen.asked.get(),
        sent: kept.0.into_inner(),
        trade: screen.trade.into_inner(),
        linking: screen.linking.into_inner(),
        unknown: screen.unknown.into_inner(),
        view: view.into_inner(),
    }
}

async fn run(taker: &SimWallet, cats: &[Bytes32], params: &str, fee: u64) -> Run {
    run_with(
        taker,
        cats,
        params,
        With {
            fee,
            permissions: &route_permissions(),
            approves: true,
            tamper: &|_| {},
        },
    )
    .await
}

async fn settle(taker: &mut SimWallet, run: &Run) {
    taker
        .push_bundle(run.sent.clone().expect("nothing was sent"))
        .await
        .expect("the chain refused the route");
    taker.wait_for_coins().await;
}

async fn balances(wallet: &SimWallet, cats: &[Bytes32]) -> (u128, Vec<u128>) {
    let mut held = Vec::new();
    for cat in cats {
        held.push(wallet.wallet.db.cat_balance(*cat).await.unwrap());
    }
    (wallet.wallet.db.xch_balance().await.unwrap(), held)
}

fn leg(asset: AssetId, amount: u64) -> Leg {
    Leg {
        asset,
        amount: u128::from(amount),
    }
}

// ---------------------------------------------------------------------------------------
// Routes that settle
// ---------------------------------------------------------------------------------------

/// A whole offer, a first fill of a partial offer whose coin is a CAT and a first fill of
/// one whose coin is the bare partial puzzle, with a fee per fill and Klimper's own fee: one
/// approval, one bundle, every balance exact.
#[tokio::test]
async fn a_whole_offer_and_two_fills_settle_as_one_route() {
    let mut m = Box::pin(market()).await;
    let cats = m.cats();
    let (xch_before, _) = balances(&m.taker, &cats).await;

    let params = request(&[&m.whole], &[(&m.cat_partial, 250), (&m.xch_partial, 600)]);
    let done = run(&m.taker, &cats, &params, 7).await;

    assert!(done.asked);
    assert_eq!(done.linking.as_deref(), Some("sealed"));
    let answer = done.result.as_ref().expect("the route was refused");
    assert!(answer["transactionId"].as_str().unwrap().starts_with("0x"));
    // What each fill paid and bought, and what is left of each partial offer.
    let fills = answer["fills"].as_array().unwrap();
    assert_eq!(
        (fills[0]["paid"].as_str(), fills[0]["bought"].as_str()),
        (Some("250"), Some("500"))
    );
    assert_eq!(
        (fills[1]["paid"].as_str(), fills[1]["bought"].as_str()),
        (Some("600"), Some("300"))
    );
    assert!(
        fills
            .iter()
            .all(|fill| fill["nextOffer"].as_str().unwrap().starts_with("partial1"))
    );

    // The screen: three legs, the fee the fills require on its own line, an expiry, and the
    // one unknown puzzle is the partial coin Klimper verified.
    let view = done.view.clone().unwrap();
    assert_eq!((view.offers, view.fills, view.fill_fees), (1, 2, 10));
    assert_eq!(
        answer["expiresAt"].as_u64(),
        Some(view.expires_at),
        "the answer and the screen name the same expiry"
    );
    assert_eq!(
        view.verified_partial_puzzles,
        vec![hex::encode(m.xch_partial.coin().puzzle_hash)]
    );
    assert_eq!(done.unknown, view.verified_partial_puzzles);
    let trade = done.trade.clone().unwrap();
    assert_eq!(trade.offers, Some(3));
    // 400 + 250 paid, 300 bought back, the fills' fee of 10 inside what is paid.
    assert_eq!(
        trade.pays,
        vec![leg(AssetId::Xch, 360), leg(AssetId::Cat(m.t), 600)]
    );
    assert_eq!(
        trade.receives,
        vec![leg(AssetId::Cat(m.a), 1_000), leg(AssetId::Cat(m.b), 500)]
    );
    assert_eq!(trade.fee, 7);

    settle(&mut m.taker, &done).await;
    let (xch_after, held) = balances(&m.taker, &cats).await;
    assert_eq!(xch_after, xch_before - 400 - 250 + 300 - 10 - 7);
    assert_eq!(held, vec![1_000, 500, 1_400]);
}

/// A fill alone is a route too, and what is left is filled from the next offer once the
/// first has confirmed: the coin is on the chain then, the maker's spends are not sent
/// again, and no leg carries a signature.
#[tokio::test]
async fn what_a_fill_leaves_is_filled_from_the_next_offer() {
    let mut m = Box::pin(market()).await;
    let cats = m.cats();

    let first = run(&m.taker, &cats, &request(&[], &[(&m.cat_partial, 250)]), 0).await;
    let next = first.result.as_ref().unwrap()["fills"][0]["nextOffer"]
        .as_str()
        .unwrap()
        .to_owned();
    // One asset for one asset: the screen shows the price of the fill.
    let trade = first.trade.clone().unwrap();
    assert_eq!(trade.pays, vec![leg(AssetId::Xch, 250)]);
    assert_eq!(trade.receives, vec![leg(AssetId::Cat(m.b), 500)]);
    assert!(trade.price.is_some());
    settle(&mut m.taker, &first).await;

    let next = PartialOffer::from_text(&next).unwrap();
    assert!(!next.is_first());
    let second = run(&m.taker, &cats, &request(&[], &[(&next, 250)]), 0).await;
    assert_eq!(second.linking.as_deref(), Some("sealed"));
    let fill = &second.result.as_ref().expect("the second fill was refused")["fills"][0];
    assert_eq!(fill["bought"].as_str(), Some("500"));
    // Everything is taken: nothing is left to offer.
    assert!(fill.get("nextOffer").is_none());
    settle(&mut m.taker, &second).await;
    assert_eq!(m.taker.wallet.db.cat_balance(m.b).await.unwrap(), 1_000);
    drop(m.second);
}

/// A first text whose coin is on the chain already (the maker sent their spends without a
/// fill): the maker's spends and signature in the text are dropped, not sent again.
#[tokio::test]
async fn a_first_text_whose_coin_is_on_the_chain_is_filled_without_the_makers_spends() {
    let mut m = Box::pin(market()).await;
    let cats = m.cats();
    let makers = m.cat_partial.maker_spends().clone();
    let inputs = m.cat_partial.input_coin_ids();
    m.second.push_bundle(makers).await.unwrap();
    m.second.wait_for_coins().await;

    let done = run(&m.taker, &cats, &request(&[], &[(&m.cat_partial, 250)]), 0).await;
    let sent = done.sent.clone().expect("nothing was sent");
    assert!(
        sent.coin_spends
            .iter()
            .all(|spend| !inputs.contains(&spend.coin.coin_id())),
        "the maker's spends were sent again"
    );
    settle(&mut m.taker, &done).await;
    assert_eq!(m.taker.wallet.db.cat_balance(m.b).await.unwrap(), 500);
}

/// A later part of a partial offer the user made themselves: the payment goes to the user,
/// the route is bound and settles. Nothing is paid for something else, so there is no trade
/// to show: the screen falls back to the plain list of what changes.
#[tokio::test]
async fn a_later_fill_of_the_users_own_partial_offer_settles() {
    let mut m = Box::pin(market()).await;
    let cats = m.cats();
    let now = chain_time(&m.taker).await;
    // 1000 of T for 500 mojos, made by the taker; the second wallet takes the first half.
    let own = cat_for_xch(&m.taker, m.t, None).await;
    know_cats(&m.second, &[m.t]).await;
    let ctx = m.second.partial_ctx().await;
    let (first, bundle) = partial_wallet::take(&ctx, &own.to_text().unwrap(), 250, 0, now)
        .await
        .expect("the first fill was refused");
    let spent: Vec<Bytes32> = bundle
        .coin_spends
        .iter()
        .map(|spend| spend.coin.coin_id())
        .collect();
    ctx.submit(bundle)
        .await
        .expect("the chain refused the fill");
    m.second.wait_for_coins().await;
    m.taker.watch_coins(spent).await;
    m.taker.wait_for_coins().await;

    let next = PartialOffer::from_text(&first.next_offer.unwrap()).unwrap();
    let done = run(&m.taker, &cats, &request(&[], &[(&next, 250)]), 0).await;
    assert!(done.asked, "{:?}", done.result);
    assert_eq!(done.linking.as_deref(), Some("sealed"));
    assert!(done.trade.is_none(), "paying oneself is not a trade");
    settle(&mut m.taker, &done).await;
    let (xch_after, held_after) = balances(&m.taker, &cats).await;
    // 4000 mojos after issuing T, 250 from the second wallet's fill; the user's own 250 go
    // out and come straight back. Of the 2000 T, the 500 the second wallet bought are gone.
    assert_eq!(xch_after, 4_250);
    assert_eq!(held_after[2], 1_500);
}

/// A route the user approved that reaches the chain after its expiry is refused there: who
/// saw the bundle cannot complete it later.
#[tokio::test]
async fn a_route_sent_after_its_expiry_is_refused_by_the_chain() {
    let m = Box::pin(market()).await;
    let cats = m.cats();
    let done = run(
        &m.taker,
        &cats,
        &request(&[&m.whole], &[(&m.cat_partial, 250)]),
        0,
    )
    .await;
    let bundle = done.sent.expect("nothing was sent");

    pass_time(&m.taker, EXPIRES_AFTER + 1).await;
    let refused = m.taker.sim.lock().await.new_transaction(bundle);
    let error = format!("{:?}", refused.expect_err("an expired route was accepted"));
    assert!(error.contains("AssertBeforeSecondsAbsolute"), "{error}");
}

/// A route the user rejects sends nothing.
#[tokio::test]
async fn a_rejected_route_sends_nothing() {
    let m = Box::pin(market()).await;
    let done = run_with(
        &m.taker,
        &m.cats(),
        &request(&[&m.whole], &[(&m.xch_partial, 600)]),
        With {
            fee: 0,
            permissions: &route_permissions(),
            approves: false,
            tamper: &|_| {},
        },
    )
    .await;
    assert!(done.asked);
    assert!(done.result.is_err());
    assert!(done.sent.is_none());
}

// ---------------------------------------------------------------------------------------
// Refusals, each before the user is asked
// ---------------------------------------------------------------------------------------

async fn refused(m: &Market, params: &str) -> String {
    let done = run(&m.taker, &m.cats(), params, 0).await;
    assert!(!done.asked, "the user was asked about a refused route");
    assert!(done.sent.is_none());
    done.reason()
}

#[tokio::test]
async fn a_partial_offer_someone_took_from_first_is_refused_as_changed() {
    let mut m = Box::pin(market()).await;
    let cats = m.cats();
    let first = run(&m.taker, &cats, &request(&[], &[(&m.cat_partial, 250)]), 0).await;
    settle(&mut m.taker, &first).await;
    // The same text again: its coin is spent now.
    let again = refused(&m, &request(&[], &[(&m.cat_partial, 250)])).await;
    assert!(again.contains("offer_changed"), "{again}");
}

#[tokio::test]
async fn a_fill_under_the_least_or_over_what_is_left_is_refused() {
    let m = Box::pin(market()).await;
    let under = refused(&m, &request(&[], &[(&m.cat_partial, 49)])).await;
    assert!(under.contains("below_min_fill"), "{under}");
    let over = refused(&m, &request(&[], &[(&m.cat_partial, 501)])).await;
    assert!(over.contains("above_max_fill"), "{over}");
}

#[tokio::test]
async fn a_leg_twice_is_refused() {
    let m = Box::pin(market()).await;
    let fill_twice = refused(
        &m,
        &request(&[], &[(&m.cat_partial, 100), (&m.cat_partial, 100)]),
    )
    .await;
    assert!(fill_twice.contains("leg_twice"), "{fill_twice}");
    let offer_twice = refused(&m, &request(&[&m.whole, &m.whole], &[])).await;
    assert!(offer_twice.contains("leg_twice"), "{offer_twice}");
}

#[tokio::test]
async fn fees_per_fill_above_the_cap_are_refused() {
    let m = Box::pin(market()).await;
    let greedy = xch_for_cat(&m.third, m.t, MAX_FILL_FEES + 1).await;
    let reason = refused(&m, &request(&[], &[(&greedy, 600)])).await;
    assert!(reason.contains("fill_fee_too_high"), "{reason}");
}

#[tokio::test]
async fn a_partial_offer_that_expires_before_the_route_does_is_refused() {
    let m = Box::pin(market()).await;
    let now = chain_time(&m.taker).await;
    let soon = cat_for_xch(&m.second, m.b, Some(now + EXPIRES_AFTER)).await;
    let reason = refused(&m, &request(&[], &[(&soon, 250)])).await;
    assert!(reason.contains("offer_expired"), "{reason}");
}

#[tokio::test]
async fn a_whole_offer_with_a_forged_signature_is_refused() {
    let m = Box::pin(market()).await;
    let mut forged = m.whole.clone();
    forged.aggregated_signature =
        chia_bls::sign(&chia_bls::SecretKey::from_seed(&[9; 32]), b"forged");
    let reason = refused(&m, &request(&[&forged], &[(&m.cat_partial, 250)])).await;
    assert!(reason.contains("offer_signature"), "{reason}");
}

/// A first fill carries the maker's signed spends: a text whose signature does not cover
/// them is refused like an offer's.
#[tokio::test]
async fn a_first_fill_with_a_forged_signature_is_refused() {
    let m = Box::pin(market()).await;
    let mut bundle = m.cat_partial.to_spend_bundle().unwrap();
    bundle.aggregated_signature =
        chia_bls::sign(&chia_bls::SecretKey::from_seed(&[9; 32]), b"forged");
    let forged = PartialOffer::from_spend_bundle(&bundle).unwrap();
    let reason = refused(&m, &request(&[], &[(&forged, 250)])).await;
    assert!(reason.contains("offer_signature"), "{reason}");
}

/// A website's whole offer may hold only the maker's own plain spends: one that spends a
/// settlement coin (which could claim a fill's payout before the user does) is refused.
#[tokio::test]
async fn a_whole_offer_that_spends_a_settlement_coin_is_refused() {
    let m = Box::pin(market()).await;
    let mut offer = m.whole.clone();
    let mut claim = offer
        .coin_spends
        .iter()
        .find(|spend| spend.coin.parent_coin_info == Bytes32::default())
        .cloned()
        .unwrap();
    claim.coin.parent_coin_info = Bytes32::new([7; 32]);
    offer.coin_spends.push(claim);
    let reason = refused(&m, &request(&[&offer], &[(&m.cat_partial, 250)])).await;
    assert!(reason.contains("unsupported_asset"), "{reason}");
}

/// A first offer the maker cancelled before any fill, by spending a coin it is made from:
/// its coin will never exist.
#[tokio::test]
async fn a_first_offer_whose_maker_spent_its_coins_is_refused_as_changed() {
    let mut m = Box::pin(market()).await;
    let home = m.second.wallet.change_p2_puzzle_hash().await.unwrap();
    let moved = m
        .second
        .wallet
        .send_cat(m.b, vec![(home, 1_000)], 0, true, vec![], None)
        .await
        .unwrap();
    m.second.transact(moved).await;
    m.second.wait_for_coins().await;
    let reason = refused(&m, &request(&[], &[(&m.cat_partial, 250)])).await;
    assert!(reason.contains("offer_changed"), "{reason}");
}

/// The lineage proof comes from the text: one that is not the coin's parent's is refused,
/// since the fill could never confirm.
#[tokio::test]
async fn a_text_with_a_wrong_lineage_proof_is_refused() {
    let mut m = Box::pin(market()).await;
    let cats = m.cats();
    let first = run(&m.taker, &cats, &request(&[], &[(&m.cat_partial, 250)]), 0).await;
    let next = first.result.as_ref().unwrap()["fills"][0]["nextOffer"]
        .as_str()
        .unwrap()
        .to_owned();
    settle(&mut m.taker, &first).await;

    let mut coin = *PartialOffer::from_text(&next).unwrap().offer();
    coin.lineage_proof.as_mut().unwrap().parent_amount += 1;
    let wrong = PartialOffer::from_coin(coin).unwrap();
    let reason = refused(&m, &request(&[], &[(&wrong, 250)])).await;
    assert!(reason.contains("fills"), "{reason}");
}

/// The maker's spends a first fill carries are the website's: held to what an offer's are.
#[tokio::test]
async fn a_first_fill_with_a_makers_coin_twice_or_too_many_spends_is_refused() {
    let m = Box::pin(market()).await;
    let bundle = m.cat_partial.to_spend_bundle().unwrap();
    let maker = bundle
        .coin_spends
        .iter()
        .find(|spend| {
            m.cat_partial
                .input_coin_ids()
                .contains(&spend.coin.coin_id())
        })
        .cloned()
        .unwrap();

    let mut twice = bundle.clone();
    twice.coin_spends.push(maker.clone());
    let text = chia_sdk_next::encode_partial_offer(&twice).unwrap();
    let reason = refused(&m, &request_texts(&[], &[(text, 250)])).await;
    assert!(reason.contains("offer_duplicate_coin"), "{reason}");

    let mut many = bundle;
    many.coin_spends
        .extend(std::iter::repeat_n(maker, MAX_OFFER_SPENDS));
    let text = chia_sdk_next::encode_partial_offer(&many).unwrap();
    let reason = refused(&m, &request_texts(&[], &[(text, 250)])).await;
    assert!(reason.contains("offer_too_large"), "{reason}");
}

/// A route trades XCH and CATs that are not revocable: a revocable CAT on either side of a
/// fill or of a whole offer is refused by name (wallet-kit cannot bind a payout inside one).
#[tokio::test]
async fn a_revocable_cat_in_a_leg_is_refused() {
    let mut m = Box::pin(market()).await;
    let issuer = Bytes32::new([5; 32]);
    let third_home = m.third.wallet.change_p2_puzzle_hash().await.unwrap();

    // A fill that asks for a revocable CAT.
    let asks_revocable = partial(
        &m.third,
        Terms {
            offered: PartialOfferAsset::Xch,
            offered_amount: 600,
            requested: PartialOfferAsset::Cat {
                asset_id: m.t,
                hidden_puzzle_hash: Some(issuer),
            },
            requested_amount: 1_200,
            min_fill_requested: 100,
            expiration: None,
            required_fee: None,
            maker_puzzle_hash: third_home,
        },
    )
    .await;
    let reason = refused(&m, &request(&[], &[(&asks_revocable, 600)])).await;
    assert!(reason.contains("unsupported_asset"), "{reason}");

    // A whole offer that asks for one.
    let asks = m
        .third
        .wallet
        .make_offer(
            Offered {
                xch: 100,
                ..Default::default()
            },
            Requested {
                cats: indexmap::indexmap! {
                    m.t => sage_wallet::RequestedCat { amount: 200, hidden_puzzle_hash: Some(issuer) }
                },
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    let asks = m
        .third
        .wallet
        .sign_transaction(asks, &m.third.agg_sig, m.third.master_sk.clone(), true)
        .await
        .unwrap();
    let reason = refused(&m, &request(&[&asks], &[])).await;
    assert!(reason.contains("unsupported_asset"), "{reason}");

    // A whole offer and a fill that give one.
    let (spends, revocable) = m
        .third
        .wallet
        .issue_cat_with_hidden_puzzle_hash(1_000, 0, None, Some(issuer))
        .await
        .unwrap();
    m.third.transact(spends).await;
    m.third.wait_for_coins().await;
    let gives = m
        .third
        .wallet
        .make_offer(
            Offered {
                cats: indexmap::indexmap! { revocable => 1_000 },
                ..Default::default()
            },
            Requested {
                xch: 100,
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    let gives = m
        .third
        .wallet
        .sign_transaction(gives, &m.third.agg_sig, m.third.master_sk.clone(), true)
        .await
        .unwrap();
    let reason = refused(&m, &request(&[&gives], &[])).await;
    assert!(reason.contains("unsupported_asset"), "{reason}");
}

/// A website's leg may not spend a settlement coin inside a CAT either.
#[tokio::test]
async fn a_whole_offer_that_spends_a_cat_settlement_coin_is_refused() {
    let m = Box::pin(market()).await;
    // An offer that asks for a CAT has a placeholder inside that CAT.
    let asks = m
        .third
        .wallet
        .make_offer(
            Offered {
                xch: 100,
                ..Default::default()
            },
            Requested {
                cats: indexmap::indexmap! {
                    m.t => sage_wallet::RequestedCat { amount: 200, hidden_puzzle_hash: None }
                },
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    let mut asks = m
        .third
        .wallet
        .sign_transaction(asks, &m.third.agg_sig, m.third.master_sk.clone(), true)
        .await
        .unwrap();
    let mut claim = asks
        .coin_spends
        .iter()
        .find(|spend| spend.coin.parent_coin_info == Bytes32::default())
        .cloned()
        .unwrap();
    claim.coin.parent_coin_info = Bytes32::new([7; 32]);
    asks.coin_spends.push(claim);
    let done = run(&m.taker, &m.cats(), &request(&[&asks], &[]), 0).await;
    assert!(!done.asked);
    assert!(done.result.is_err());
    assert!(done.sent.is_none());
}

#[tokio::test]
async fn a_route_over_the_websites_limit_is_refused() {
    let m = Box::pin(market()).await;
    let mut permissions = route_permissions();
    permissions.limits.insert(
        AssetId::Xch,
        AssetLimit {
            // The fill alone pays 250: the limit counts the fill, not the partial offer.
            per_request: Some(249),
            per_day: None,
        },
    );
    let done = run_with(
        &m.taker,
        &m.cats(),
        &request(&[], &[(&m.cat_partial, 250)]),
        With {
            fee: 0,
            permissions: &permissions,
            approves: true,
            tamper: &|_| {},
        },
    )
    .await;
    assert!(!done.asked);
    assert!(done.result.is_err());

    permissions.limits.insert(
        AssetId::Xch,
        AssetLimit {
            per_request: Some(250),
            per_day: None,
        },
    );
    let done = run_with(
        &m.taker,
        &m.cats(),
        &request(&[], &[(&m.cat_partial, 250)]),
        With {
            fee: 0,
            permissions: &permissions,
            approves: true,
            tamper: &|_| {},
        },
    )
    .await;
    assert!(done.asked, "{:?}", done.result);
}

/// A website without the permission is refused by wallet-kit before the user is asked.
#[tokio::test]
async fn a_website_without_the_permission_is_refused() {
    let m = Box::pin(market()).await;
    let mut permissions = route_permissions();
    permissions.methods = vec![CHIA_TAKE_OFFER.to_owned()];
    let done = run_with(
        &m.taker,
        &m.cats(),
        &request(&[&m.whole], &[]),
        With {
            fee: 0,
            permissions: &permissions,
            approves: true,
            tamper: &|_| {},
        },
    )
    .await;
    assert!(!done.asked);
    assert!(done.result.is_err());
}

/// The one unknown puzzle let through is a partial coin of the route, spent exactly as the
/// wallet built it. Anything else unknown refuses, whatever wallet-kit would be told.
#[tokio::test]
async fn an_unknown_puzzle_that_is_not_the_wallets_own_fill_is_refused() {
    let m = Box::pin(market()).await;
    let params = request(&[], &[(&m.xch_partial, 600)]);
    // The wallet no longer vouches for the coin.
    let unvouched = run_with(
        &m.taker,
        &m.cats(),
        &params,
        With {
            fee: 0,
            permissions: &route_permissions(),
            approves: true,
            tamper: &|built| {
                for (_, bare) in &mut built.partial_spends {
                    *bare = false;
                }
            },
        },
    )
    .await;
    assert!(!unvouched.asked);
    assert!(unvouched.reason().contains("unknown_contract"));

    // The spend in the route is not the one the wallet built.
    let swapped = run_with(
        &m.taker,
        &m.cats(),
        &params,
        With {
            fee: 0,
            permissions: &route_permissions(),
            approves: true,
            tamper: &|built| {
                for (spend, _) in &mut built.partial_spends {
                    spend.solution = Program::default();
                }
            },
        },
    )
    .await;
    assert!(!swapped.asked);
    assert!(swapped.reason().contains("unknown_contract"));
}

/// A first fill of the user's own partial offer would have the wallet sign the maker's side
/// too, with nothing binding it: refused before the user is asked.
#[tokio::test]
async fn a_first_fill_of_the_users_own_partial_offer_is_refused() {
    let m = Box::pin(market()).await;
    let own = cat_for_xch(&m.taker, m.t, None).await;
    let done = run(&m.taker, &m.cats(), &request(&[], &[(&own, 250)]), 0).await;
    assert!(!done.asked, "{:?}", done.result);
    assert!(
        done.reason().contains("unbound_partial"),
        "{}",
        done.reason()
    );
    assert!(done.sent.is_none());
}

/// A whole offer the user made themselves is the same: the wallet would sign both sides.
#[tokio::test]
async fn a_whole_offer_of_the_users_own_is_refused() {
    let m = Box::pin(market()).await;
    let own = m
        .taker
        .wallet
        .make_offer(
            Offered {
                cats: indexmap::indexmap! { m.t => 100 },
                ..Default::default()
            },
            Requested {
                xch: 50,
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
    let own = m
        .taker
        .wallet
        .sign_transaction(own, &m.taker.agg_sig, m.taker.master_sk.clone(), true)
        .await
        .unwrap();
    let done = run(&m.taker, &m.cats(), &request(&[&own], &[]), 0).await;
    assert!(!done.asked, "{:?}", done.result);
    assert!(
        done.reason().contains("unbound_partial"),
        "{}",
        done.reason()
    );
    assert!(done.sent.is_none());
}

// ---------------------------------------------------------------------------------------
// The one unknown puzzle, on its own
// ---------------------------------------------------------------------------------------

fn spend_of(byte: u8) -> CoinSpend {
    CoinSpend::new(
        Coin::new(Bytes32::new([byte; 32]), Bytes32::new([byte; 32]), 1),
        Program::default(),
        Program::default(),
    )
}

fn info_of(spend: &CoinSpend, kind: kit::SpendKind, owned: bool) -> kit::simulate::SpendInfo {
    kit::simulate::SpendInfo {
        coin_id: hex::encode(spend.coin.coin_id()),
        puzzle_hash: hex::encode(spend.coin.puzzle_hash),
        amount: spend.coin.amount,
        kind,
        owned,
        agg_sigs: vec![],
        cost: 0,
    }
}

/// A route of the user's spend (`1`) and the partial coins `bare` (flagged) and `wrapped`.
fn route_of(bare: &[u8], wrapped: &[u8], others: &[u8]) -> BuiltRoute {
    let partial_spends: Vec<(CoinSpend, bool)> = bare
        .iter()
        .map(|byte| (spend_of(*byte), true))
        .chain(wrapped.iter().map(|byte| (spend_of(*byte), false)))
        .collect();
    let mut spends: Vec<CoinSpend> = partial_spends.iter().map(|(s, _)| s.clone()).collect();
    spends.extend(others.iter().map(|byte| spend_of(*byte)));
    BuiltRoute {
        spends,
        legs_signature: Signature::default(),
        leg_coins: HashSet::new(),
        partial_spends,
        fills: Vec::new(),
        view: RouteView {
            offers: 0,
            fills: bare.len() + wrapped.len(),
            fill_fees: 0,
            expires_at: 1,
            verified_partial_puzzles: Vec::new(),
        },
    }
}

fn summary_of(spends: Vec<kit::simulate::SpendInfo>, unknown: &[u8]) -> Summary {
    let mut summary = Summary::default();
    summary.spends = spends;
    summary.unknown_puzzles = unknown
        .iter()
        .map(|byte| hex::encode([*byte; 32]))
        .collect();
    summary
}

#[test]
fn only_the_routes_own_bare_partial_coins_may_be_unknown() {
    use kit::SpendKind::{Standard, Unknown};
    let refused = |built: &BuiltRoute, summary: &Summary| {
        reason(&allowed_unknown(built, summary).unwrap_err()).contains("unknown_contract")
    };

    // Nothing unknown: wallet-kit is not told to let anything through.
    let plain = route_of(&[], &[3], &[1]);
    let summary = summary_of(vec![info_of(&spend_of(1), Standard, true)], &[]);
    assert_eq!(allowed_unknown(&plain, &summary).ok(), Some(false));

    // Two bare partial coins of the route, both unknown: let through.
    let two = route_of(&[2, 4], &[], &[1]);
    let both = summary_of(
        vec![
            info_of(&spend_of(2), Unknown, false),
            info_of(&spend_of(4), Unknown, false),
            info_of(&spend_of(1), Standard, true),
        ],
        &[2, 4],
    );
    assert_eq!(allowed_unknown(&two, &both).ok(), Some(true));

    // An unknown spend that is no partial coin of the route.
    let stranger = summary_of(
        vec![
            info_of(&spend_of(2), Unknown, false),
            info_of(&spend_of(9), Unknown, false),
        ],
        &[2, 9],
    );
    assert!(refused(&route_of(&[2], &[], &[9]), &stranger));

    // A partial coin of the route the wallet did not build as a bare one.
    let wrapped = summary_of(vec![info_of(&spend_of(3), Unknown, false)], &[3]);
    assert!(refused(&route_of(&[2], &[3], &[]), &wrapped));

    // A bare partial coin that is the user's own coin.
    let owned = summary_of(vec![info_of(&spend_of(2), Unknown, true)], &[2]);
    assert!(refused(&route_of(&[2], &[], &[]), &owned));

    // A puzzle wallet-kit lists as unknown that no allowed spend has (a spend the user signs
    // without wallet-kit recognising it is listed there too).
    let listed = summary_of(vec![info_of(&spend_of(2), Unknown, false)], &[2, 8]);
    assert!(refused(&route_of(&[2], &[], &[]), &listed));

    // The spend in the route twice.
    let mut twice = route_of(&[2], &[], &[]);
    twice.spends.push(spend_of(2));
    let once = summary_of(vec![info_of(&spend_of(2), Unknown, false)], &[2]);
    assert!(refused(&twice, &once));
}

// ---------------------------------------------------------------------------------------
// What counts as a failed build
// ---------------------------------------------------------------------------------------

#[test]
fn the_chains_news_about_a_partial_offer_is_not_a_failed_build() {
    let with = |reason: &str| error(codes::INVALID_PARAMS, "refused", Some(reason));
    for news in [
        "offer_changed",
        "offer_expired",
        "below_min_fill",
        "above_max_fill",
        "fill_fee_too_high",
        "chain_unavailable",
    ] {
        assert!(!counts_as_failed_build(&with(news)), "{news}");
    }
    for failed in [
        "offer",
        "offer_signature",
        "offer_duplicate_coin",
        "offer_too_large",
        "nothing_requested",
        "unknown_contract",
        "unsupported_asset",
        "leg_twice",
        "fills",
        "build_failed",
    ] {
        assert!(counts_as_failed_build(&with(failed)), "{failed}");
    }
    // A refusal without a reason counts: nothing is let through by leaving it out.
    assert!(counts_as_failed_build(&error(
        codes::INVALID_PARAMS,
        "refused",
        None
    )));
}

// ---------------------------------------------------------------------------------------
// The request's parameters
// ---------------------------------------------------------------------------------------

#[test]
fn the_parameters_are_read_strictly() {
    let reason_of = |params: Value| reason(&route_params(&params.to_string()).unwrap_err());
    // No leg at all.
    assert!(reason_of(json!({})).contains("offers"));
    assert!(reason_of(json!({ "offers": [], "fills": [] })).contains("offers"));
    // Each list holds its own kind of text.
    assert!(reason_of(json!({ "offers": ["partial1abc"] })).contains("offers"));
    assert!(
        reason_of(json!({ "fills": [{ "offer": "offer1abc", "amount": 5 }] })).contains("fills")
    );
    // An amount is a positive number of mojos, as a number or a string.
    assert!(
        reason_of(json!({ "fills": [{ "offer": "partial1abc", "amount": 0 }] })).contains("fills")
    );
    assert!(reason_of(json!({ "fills": [{ "offer": "partial1abc" }] })).contains("fills"));
    assert!(
        reason_of(json!({ "fills": [{ "offer": "partial1abc", "amount": -1 }] })).contains("fills")
    );
    assert!(reason_of(json!({ "offers": "offer1abc" })).contains("offers"));
    // Not more legs than a route may have.
    let many: Vec<String> = (0..=MAX_LEGS).map(|i| format!("offer1{i}")).collect();
    assert!(reason_of(json!({ "offers": many })).contains("too_many_legs"));

    let read = route_params(
        &json!({
            "offers": [" offer1abc "],
            "fills": [{ "offer": "partial1abc", "amount": "250" }],
            "fee": 99
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(read.offers, vec!["offer1abc".to_owned()]);
    assert_eq!(
        (read.fills[0].offer.as_str(), read.fills[0].amount),
        ("partial1abc", 250)
    );
    // The website's fee is read and ignored: the fee is Klimper's own.
    assert_eq!(read.fee, own_fee());
}

#[test]
fn a_route_is_built_by_the_wallet() {
    assert!(is_wallet_built(CHIA_TAKE_ROUTE));
}
