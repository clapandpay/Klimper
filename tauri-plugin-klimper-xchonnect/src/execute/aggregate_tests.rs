// SPDX-License-Identifier: AGPL-3.0-or-later
//! Taking several makers' offers at once (TASK-139): two or three offers, each made and
//! signed by its own Sage wallet, joined into one offer and taken through Klimper's own
//! path (`take_tests::take`: the offer checks, the key-free build, the binding, Sage's
//! summary, the cross-check and wallet-kit's submit). The bundle goes to the simulated
//! chain and must settle, all of it, with every balance as the offers said.
//!
//! The offers are joined three ways: with Sage's `aggregate_offers` (spends together, the
//! asked-for placeholders first, signatures summed), by plain concatenation with the
//! signatures summed, and merged (one placeholder per asset carrying every maker's
//! notarized payments, as chia's `Offer.aggregate` writes it and Pengui sends by default).
//!
//! `export_aggregate_fixtures` (ignored; run by hand) writes
//! `tests/fixtures/aggregate_offers.json` for websites that aggregate offers themselves to
//! test against Sage's aggregation; `the_committed_fixtures_still_hold` keeps that file
//! honest.

use chia_wallet_sdk::driver::{decode_offer, encode_offer};
use sage_wallet::{Offered, Requested, RequestedCat, aggregate_offers};
use serde_json::json;

use super::sage_sim::SimWallet;
use super::take_tests::{
    Outcome, checked, issue_amount, know_cats, reason, take, take_permissions,
};
use super::*;
use crate::trade::{Leg, TradeOverview};

/// How the offers are joined into one.
#[derive(Clone, Copy, Debug)]
enum Join {
    /// Sage's `aggregate_offers`.
    Sage,
    /// Every offer's spends one after another, the signatures summed.
    Concat,
    /// One placeholder per requested asset carrying every maker's notarized payments, in
    /// order, then the makers' own spends, the signatures summed: as chia's
    /// `Offer.aggregate` writes it, and as Pengui's browser code sends by default
    /// (`aggregateOffers.ts`, `mergePlaceholders: true`).
    Merged,
}

/// Pengui's merged form: placeholders keyed by their coin's puzzle hash (one per asset, the
/// first one's coin and puzzle kept), their solutions' notarized payments concatenated as
/// CLVM, never re-encoded, so each maker's announcement still matches byte for byte.
fn merged(offers: &[SpendBundle]) -> SpendBundle {
    use chia_wallet_sdk::prelude::{Allocator, FromClvm, NodePtr, ToClvm};

    let mut allocator = Allocator::new();
    let mut placeholders: Vec<(CoinSpend, Vec<NodePtr>)> = Vec::new();
    let mut spends = Vec::new();
    let mut signature = Signature::default();
    for offer in offers {
        signature += &offer.aggregated_signature;
        for spend in &offer.coin_spends {
            if spend.coin.parent_coin_info != Bytes32::default() {
                spends.push(spend.clone());
                continue;
            }
            let solution = spend.solution.to_clvm(&mut allocator).unwrap();
            let items = Vec::<NodePtr>::from_clvm(&allocator, solution).unwrap();
            match placeholders
                .iter_mut()
                .find(|(first, _)| first.coin.puzzle_hash == spend.coin.puzzle_hash)
            {
                Some((_, payments)) => payments.extend(items),
                None => placeholders.push((spend.clone(), items)),
            }
        }
    }
    let mut coin_spends: Vec<CoinSpend> = placeholders
        .into_iter()
        .map(|(first, payments)| {
            let list = payments.to_clvm(&mut allocator).unwrap();
            CoinSpend::new(
                first.coin,
                first.puzzle_reveal,
                Program::from_clvm(&allocator, list).unwrap(),
            )
        })
        .collect();
    // Placeholders first, as `Offer.to_spend_bundle` orders them.
    coin_spends.extend(spends);
    SpendBundle::new(coin_spends, signature)
}

fn join(offers: &[SpendBundle], how: Join) -> SpendBundle {
    match how {
        Join::Sage => aggregate_offers(offers.to_vec()),
        Join::Concat => {
            let mut joined = SpendBundle::new(Vec::new(), Signature::default());
            for offer in offers {
                joined.coin_spends.extend(offer.coin_spends.iter().cloned());
                joined.aggregated_signature += &offer.aggregated_signature;
            }
            joined
        }
        Join::Merged => merged(offers),
    }
}

/// What the makers offer, and what the user pays for it.
#[derive(Clone, Copy, Debug)]
enum Market {
    /// The user buys one CAT: every maker sells 1000 of the same CAT for XCH, each at
    /// their own price.
    Buy,
    /// The user sells a CAT: every maker buys 1000 of it, each for their own amount of
    /// XCH.
    Sell,
    /// CATs for a CAT: every maker gives 1000 of a CAT of their own for 1000 of the
    /// user's, so the user pays one CAT and receives as many as there are makers.
    Swap,
}

/// The makers' signed offers and a taker on the same chain.
struct Book {
    taker: SimWallet,
    offers: Vec<SpendBundle>,
    /// Every CAT the take touches (for Sage's summary).
    cats: Vec<Bytes32>,
    /// What the user pays and receives once every offer is taken, the fee not included.
    pays: Vec<Leg>,
    receives: Vec<Leg>,
}

/// XCH each maker asks, or gives, in [`Market::Buy`] and [`Market::Sell`]:
/// three different prices.
const PRICES: [u64; 3] = [400, 500, 650];

async fn signed_offer(maker: &SimWallet, offered: Offered, requested: Requested) -> SpendBundle {
    let offer = maker
        .wallet
        .make_offer(offered, requested, None)
        .await
        .unwrap();
    maker
        .wallet
        .sign_transaction(offer, &maker.agg_sig, maker.master_sk.clone(), true)
        .await
        .unwrap()
}

fn asks_cat(asset_id: Bytes32, amount: u64) -> Requested {
    Requested {
        cats: indexmap::indexmap! { asset_id => RequestedCat { amount, hidden_puzzle_hash: None } },
        ..Default::default()
    }
}

/// `makers` wallets, each with 3000 mojos (the first, which may issue every maker's CAT,
/// 10000), and a taker with 5000 between them on one chain.
async fn wallets(makers: usize) -> (Vec<SimWallet>, SimWallet) {
    let first = SimWallet::new(10_000).await;
    let taker = first.next(5_000).await;
    let mut all = vec![first];
    let mut last = taker.next(3_000).await;
    for _ in 2..makers {
        let next = last.next(3_000).await;
        all.push(last);
        last = next;
    }
    if makers > 1 {
        all.push(last);
    }
    (all, taker)
}

async fn book(market: Market, makers: usize, maker_fee: u64) -> Book {
    let (mut makers_w, mut taker) = wallets(makers).await;
    let n = u64::try_from(makers).unwrap();
    let mut offers = Vec::new();
    let leg = |asset, amount: u64| Leg {
        asset,
        amount: u128::from(amount),
    };
    // Only the first maker pays a fee of their own: a mix, as in a real book.
    let fee_of = |index: usize| if index == 0 { maker_fee } else { 0 };
    match market {
        Market::Buy => {
            // The first maker issues the CAT and hands 1000 to each of the others.
            let a = issue_amount(&mut makers_w[0], 1_000 * n).await;
            let mut to = Vec::new();
            for maker in &makers_w[1..] {
                know_cats(maker, &[a]).await;
                to.push((maker.wallet.change_p2_puzzle_hash().await.unwrap(), 1_000));
            }
            if !to.is_empty() {
                let spends = makers_w[0]
                    .wallet
                    .send_cat(a, to, 0, true, vec![], None)
                    .await
                    .unwrap();
                // The coins the transfer creates, for the others to watch.
                let mut allocator = chia_wallet_sdk::prelude::Allocator::new();
                let (executed, _) = kit::simulate::execute(
                    &mut allocator,
                    &spends,
                    &kit::Ownership::default(),
                    kit::simulate::DEFAULT_MAX_COST,
                )
                .unwrap();
                let created: Vec<Bytes32> = executed
                    .iter()
                    .flat_map(|spend| {
                        spend.conditions.iter().filter_map(|condition| {
                            condition.as_create_coin().map(|create| {
                                Coin::new(spend.coin.coin_id(), create.puzzle_hash, create.amount)
                                    .coin_id()
                            })
                        })
                    })
                    .collect();
                makers_w[0].transact(spends).await;
                for maker in &makers_w[1..] {
                    maker.watch_coins(created.clone()).await;
                }
                // Until each wallet has synced the CAT it now holds, and the issuer its
                // change.
                for maker in &mut makers_w {
                    while maker.wallet.db.cat_balance(a).await.unwrap() != 1_000 {
                        maker.next_event().await;
                    }
                }
            }
            for (index, maker) in makers_w.iter().enumerate() {
                let offered = Offered {
                    cats: indexmap::indexmap! { a => 1_000 },
                    fee: fee_of(index),
                    ..Default::default()
                };
                let requested = Requested {
                    xch: PRICES[index],
                    ..Default::default()
                };
                offers.push(signed_offer(maker, offered, requested).await);
            }
            Book {
                taker,
                offers,
                cats: vec![a],
                pays: vec![leg(AssetId::Xch, PRICES[..makers].iter().sum())],
                receives: vec![leg(AssetId::Cat(a), 1_000 * n)],
            }
        }
        Market::Sell => {
            let b = issue_amount(&mut taker, 1_000 * n).await;
            for (index, maker) in makers_w.iter().enumerate() {
                let offered = Offered {
                    xch: PRICES[index],
                    fee: fee_of(index),
                    ..Default::default()
                };
                offers.push(signed_offer(maker, offered, asks_cat(b, 1_000)).await);
            }
            Book {
                taker,
                offers,
                cats: vec![b],
                pays: vec![leg(AssetId::Cat(b), 1_000 * n)],
                receives: vec![leg(AssetId::Xch, PRICES[..makers].iter().sum())],
            }
        }
        Market::Swap => {
            let b = issue_amount(&mut taker, 1_000 * n).await;
            let mut cats = vec![b];
            let mut receives = Vec::new();
            for (index, maker) in makers_w.iter_mut().enumerate() {
                let a = issue_amount(maker, 1_000).await;
                let offered = Offered {
                    cats: indexmap::indexmap! { a => 1_000 },
                    fee: fee_of(index),
                    ..Default::default()
                };
                offers.push(signed_offer(maker, offered, asks_cat(b, 1_000)).await);
                cats.push(a);
                receives.push(leg(AssetId::Cat(a), 1_000));
            }
            receives.sort_by_key(|l| l.asset);
            Book {
                taker,
                offers,
                cats,
                pays: vec![leg(AssetId::Cat(b), 1_000 * n)],
                receives,
            }
        }
    }
}

/// Join the makers' offers, take them all in one approval, and check the chain settles
/// every one: the taker's balances move by exactly what the offers said, plus the fee.
async fn aggregated_take_settles(
    market: Market,
    makers: usize,
    how: Join,
    maker_fee: u64,
    taker_fee: u64,
) {
    let label = format!("{market:?} ×{makers} {how:?} fees {maker_fee}/{taker_fee}");
    let Book {
        mut taker,
        offers,
        cats,
        pays,
        receives,
    } = book(market, makers, maker_fee).await;
    let offer = join(&offers, how);
    let xch_before = taker.wallet.db.xch_balance().await.unwrap();
    let mut cats_before = Vec::new();
    for &cat in &cats {
        cats_before.push(taker.wallet.db.cat_balance(cat).await.unwrap());
    }

    let Outcome {
        result,
        asked,
        sent,
        trade,
        linking,
        ..
    } = take(&taker, &cats, &offer, taker_fee, &take_permissions(), true).await;
    let answer = result.unwrap_or_else(|error| panic!("{label}: refused: {error:?}"));
    assert!(asked, "{label}: the user was never asked");
    // With the fee Klimper chose, Sage's take of several offers is still sealed.
    assert_eq!(linking.as_deref(), Some("sealed"), "{label}");
    assert!(answer.contains("transactionId"), "{answer}");

    // What the approval screen shows, worked out by the wallet.
    let trade: TradeOverview = trade.expect("no trade overview");
    assert_eq!(trade.offers, Some(makers), "{label}");
    assert_eq!(trade.pays, pays, "{label}");
    // Selling for XCH, Sage pays the user's fee out of the XCH the offers pay: the screen
    // shows what arrives, already less the fee, and no fee of the user's own XCH (H1).
    let from_proceeds = matches!(market, Market::Sell);
    let mut shown = receives.clone();
    if from_proceeds {
        for leg in &mut shown {
            if leg.asset == AssetId::Xch {
                leg.amount -= u128::from(taker_fee);
            }
        }
    }
    assert_eq!(trade.receives, shown, "{label}");
    assert_eq!(
        trade.fee,
        if from_proceeds { 0 } else { taker_fee },
        "{label}"
    );
    assert_eq!(
        trade.price.is_some(),
        pays.len() == 1 && receives.len() == 1,
        "{label}"
    );

    let bundle = sent.expect("nothing was sent");
    taker
        .push_bundle(bundle)
        .await
        .unwrap_or_else(|error| panic!("{label}: the chain refused the take: {error}"));
    taker.wait_for_coins().await;

    let mut xch = i128::try_from(xch_before).unwrap() - i128::from(taker_fee);
    for leg in &pays {
        if leg.asset == AssetId::Xch {
            xch -= i128::try_from(leg.amount).unwrap();
        }
    }
    for leg in &receives {
        if leg.asset == AssetId::Xch {
            xch += i128::try_from(leg.amount).unwrap();
        }
    }
    assert_eq!(
        i128::try_from(taker.wallet.db.xch_balance().await.unwrap()).unwrap(),
        xch,
        "{label}: XCH after the take"
    );
    for (&cat, before) in cats.iter().zip(cats_before) {
        let mut expected = i128::try_from(before).unwrap();
        for leg in pays.iter().filter(|l| l.asset == AssetId::Cat(cat)) {
            expected -= i128::try_from(leg.amount).unwrap();
        }
        for leg in receives.iter().filter(|l| l.asset == AssetId::Cat(cat)) {
            expected += i128::try_from(leg.amount).unwrap();
        }
        assert_eq!(
            i128::try_from(taker.wallet.db.cat_balance(cat).await.unwrap()).unwrap(),
            expected,
            "{label}: CAT {cat} after the take"
        );
    }
}

#[tokio::test]
async fn two_offers_for_one_cat_aggregated_by_sage_settle() {
    Box::pin(aggregated_take_settles(Market::Buy, 2, Join::Sage, 0, 0)).await;
}

#[tokio::test]
async fn three_offers_for_one_cat_with_both_fees_settle() {
    Box::pin(aggregated_take_settles(
        Market::Buy,
        3,
        Join::Sage,
        250,
        100,
    ))
    .await;
}

#[tokio::test]
async fn three_offers_for_one_cat_joined_in_the_browser_settle() {
    Box::pin(aggregated_take_settles(
        Market::Buy,
        3,
        Join::Concat,
        0,
        100,
    ))
    .await;
}

#[tokio::test]
async fn selling_a_cat_to_three_offers_settles() {
    Box::pin(aggregated_take_settles(Market::Sell, 3, Join::Sage, 0, 0)).await;
}

#[tokio::test]
async fn selling_a_cat_to_two_offers_joined_in_the_browser_with_both_fees_settles() {
    Box::pin(aggregated_take_settles(
        Market::Sell,
        2,
        Join::Concat,
        250,
        100,
    ))
    .await;
}

#[tokio::test]
async fn three_cats_for_one_cat_settle() {
    Box::pin(aggregated_take_settles(
        Market::Swap,
        3,
        Join::Sage,
        250,
        100,
    ))
    .await;
}

#[tokio::test]
async fn two_cats_for_one_cat_joined_in_the_browser_settle() {
    Box::pin(aggregated_take_settles(Market::Swap, 2, Join::Concat, 0, 0)).await;
}

#[tokio::test]
async fn three_offers_for_one_cat_merged_as_chia_writes_them_settle() {
    Box::pin(aggregated_take_settles(
        Market::Buy,
        3,
        Join::Merged,
        250,
        100,
    ))
    .await;
}

#[tokio::test]
async fn selling_a_cat_to_three_offers_merged_settles() {
    Box::pin(aggregated_take_settles(
        Market::Sell,
        3,
        Join::Merged,
        0,
        100,
    ))
    .await;
}

#[tokio::test]
async fn three_cats_for_one_cat_merged_settle() {
    Box::pin(aggregated_take_settles(
        Market::Swap,
        3,
        Join::Merged,
        250,
        0,
    ))
    .await;
}

/// One maker's signature replaced inside the aggregate: the sum no longer verifies over the
/// makers' spends, and the take is refused before the user is asked.
#[tokio::test]
async fn an_aggregate_with_one_forged_signature_is_refused_before_asking() {
    let Book {
        taker,
        mut offers,
        cats,
        ..
    } = Box::pin(book(Market::Buy, 3, 0)).await;
    offers[1].aggregated_signature =
        chia_bls::sign(&chia_bls::SecretKey::from_seed(&[9; 32]), b"forged");
    for how in [Join::Sage, Join::Concat, Join::Merged] {
        let outcome = take(
            &taker,
            &cats,
            &join(&offers, how),
            0,
            &take_permissions(),
            true,
        )
        .await;
        assert!(!outcome.asked, "{how:?}");
        assert!(outcome.sent.is_none(), "{how:?}");
        let error = outcome.result.unwrap_err();
        assert_eq!(error.code, codes::INVALID_PARAMS, "{how:?}");
        assert!(reason(&error).contains("offer_signature"), "{how:?}");
    }
}

/// One maker's signature left out of the sum: the same refusal.
#[tokio::test]
async fn an_aggregate_missing_one_signature_is_refused_before_asking() {
    let Book {
        taker,
        mut offers,
        cats,
        ..
    } = Box::pin(book(Market::Sell, 2, 0)).await;
    offers[0].aggregated_signature = Signature::default();
    let outcome = take(
        &taker,
        &cats,
        &join(&offers, Join::Sage),
        0,
        &take_permissions(),
        true,
    )
    .await;
    assert!(!outcome.asked);
    assert!(reason(&outcome.result.unwrap_err()).contains("offer_signature"));
}

/// The same offer twice spends the same coins twice: the chain would refuse the bundle,
/// so the wallet refuses it before the user is asked to approve a take that cannot happen.
#[tokio::test]
async fn an_aggregate_holding_one_offer_twice_is_refused_before_asking() {
    let Book {
        taker,
        offers,
        cats,
        ..
    } = Box::pin(book(Market::Buy, 2, 0)).await;
    let twice = [offers[0].clone(), offers[1].clone(), offers[0].clone()];
    for how in [Join::Sage, Join::Concat, Join::Merged] {
        let outcome = take(
            &taker,
            &cats,
            &join(&twice, how),
            0,
            &take_permissions(),
            true,
        )
        .await;
        assert!(!outcome.asked, "{how:?}");
        let error = outcome.result.unwrap_err();
        assert_eq!(error.code, codes::INVALID_PARAMS, "{how:?}");
        assert!(reason(&error).contains("offer_duplicate_coin"), "{how:?}");
    }
}

/// An aggregate is bounded like any offer: past [`MAX_OFFER_SPENDS`] spends it is refused
/// before anything runs, however many makers it names.
#[tokio::test]
async fn an_aggregate_over_the_spend_limit_is_refused_before_anything_runs() {
    let Book { offers, .. } = Box::pin(book(Market::Buy, 3, 0)).await;
    let per_round: usize = offers.iter().map(|o| o.coin_spends.len()).sum();
    let rounds = MAX_OFFER_SPENDS / per_round + 1;
    let many: Vec<SpendBundle> = offers
        .iter()
        .cycle()
        .take(rounds * offers.len())
        .cloned()
        .collect();
    let aggregate = join(&many, Join::Sage);
    assert!(aggregate.coin_spends.len() > MAX_OFFER_SPENDS);
    let error = checked(&aggregate).unwrap_err();
    assert_eq!(error.code, codes::INVALID_PARAMS);
    assert!(reason(&error).contains("offer_too_large"));
    // The three offers on their own are well within it, and pass.
    checked(&join(&offers, Join::Sage)).unwrap();
}

#[tokio::test]
async fn the_offers_of_an_aggregate_are_counted() {
    let Book { offers, .. } = Box::pin(book(Market::Sell, 3, 0)).await;
    assert_eq!(crate::trade::offers_in(&offers[0]), 1);
    assert_eq!(crate::trade::offers_in(&join(&offers[..2], Join::Sage)), 2);
    assert_eq!(crate::trade::offers_in(&join(&offers, Join::Concat)), 3);
    assert_eq!(crate::trade::offers_in(&join(&offers, Join::Merged)), 3);
}

/// One offer whose asked-for payment is notarized five times with five nonces (a maker
/// chooses nonces freely) is still one offer: its coins are tied together. TASK-139 audit,
/// L1: counted by nonces alone it showed "5 offers".
#[tokio::test]
async fn one_offer_with_five_nonces_counts_as_one() {
    use chia_puzzle_types::offer::NotarizedPayment;
    use chia_wallet_sdk::prelude::{Allocator, FromClvm, NodePtr, ToClvm};

    let Book { offers, .. } = Box::pin(book(Market::Sell, 2, 0)).await;
    let inflate = |offer: &SpendBundle| {
        let mut offer = offer.clone();
        let mut allocator = Allocator::new();
        for spend in &mut offer.coin_spends {
            if spend.coin.parent_coin_info != Bytes32::default() {
                continue;
            }
            let solution = spend.solution.to_clvm(&mut allocator).unwrap();
            let mut payments: Vec<NotarizedPayment<NodePtr>> =
                Vec::from_clvm(&allocator, solution).unwrap();
            let first = payments[0].clone();
            for n in 1..5_u8 {
                let mut copy = first.clone();
                copy.nonce = Bytes32::new([n; 32]);
                payments.push(copy);
            }
            let list = payments.to_clvm(&mut allocator).unwrap();
            spend.solution = Program::from_clvm(&allocator, list).unwrap();
        }
        offer
    };
    let one = inflate(&offers[0]);
    assert_eq!(crate::trade::offers_in(&one), 1);
    // Two such offers joined: two, not ten.
    let two = join(&[one, inflate(&offers[1])], Join::Concat);
    assert_eq!(crate::trade::offers_in(&two), 2);
}

// ---------------------------------------------------------------------------------------
// Fixtures for websites that aggregate offers themselves.
// ---------------------------------------------------------------------------------------

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/aggregate_offers.json"
);

/// Writes `tests/fixtures/aggregate_offers.json`: real offers on a simulator chain, Sage's
/// aggregate of them, the plain concatenation, and what Klimper's approval shows for the
/// take. Run by hand:
/// `cargo test -p tauri-plugin-klimper-xchonnect export_aggregate_fixtures -- --ignored`.
#[tokio::test]
#[ignore = "writes the committed fixture file; run by hand"]
async fn export_aggregate_fixtures() {
    let mut cases = Vec::new();
    for (name, market, makers, maker_fee) in [
        (
            "three makers sell the same CAT for XCH at three prices",
            Market::Buy,
            3,
            250,
        ),
        ("two makers buy the same CAT with XCH", Market::Sell, 2, 0),
        (
            "three makers each give a CAT of their own for one CAT",
            Market::Swap,
            3,
            0,
        ),
    ] {
        let Book {
            taker,
            offers,
            cats,
            ..
        } = Box::pin(book(market, makers, maker_fee)).await;
        let sage = join(&offers, Join::Sage);
        let concat = join(&offers, Join::Concat);
        let merged = join(&offers, Join::Merged);
        // Taken and declined, so the overview is the screen's and nothing is sent.
        let outcome = take(&taker, &cats, &sage, 0, &take_permissions(), false).await;
        assert!(outcome.asked, "{name}");
        cases.push(json!({
            "name": name,
            "network": "testnet11",
            "maker_offers": offers
                .iter()
                .map(|o| encode_offer(o).unwrap())
                .collect::<Vec<_>>(),
            "sage_aggregate": encode_offer(&sage).unwrap(),
            "concatenated": encode_offer(&concat).unwrap(),
            "merged": encode_offer(&merged).unwrap(),
            "expected": {
                "offers": makers,
                "coin_spends": sage.coin_spends.len(),
                "aggregated_signature": hex0x(sage.aggregated_signature.to_bytes()),
                "sage_aggregate_id": hex0x(sage.name()),
                "concatenated_id": hex0x(concat.name()),
                "merged_id": hex0x(merged.name()),
                "trade": outcome.trade.expect("no trade overview"),
            },
        }));
    }
    let document = json!({
        "about": "Real offers made and signed by Sage wallets on a simulated testnet11 chain \
                  (Klimper TASK-139), and Sage's aggregate_offers of them. `concatenated` is \
                  the plain concatenation of the makers' spends with their signatures summed. \
                  `merged` has one requested-payment placeholder per asset carrying every \
                  maker's notarized payments, then the makers' spends, signatures summed: \
                  chia's Offer.aggregate form, and Pengui's default. \
                  `expected.trade` is what Klimper's approval shows when the user takes \
                  `sage_aggregate` with no fee of their own: amounts in mojos, CATs at three \
                  decimals. Regenerate with `cargo test -p tauri-plugin-klimper-xchonnect \
                  export_aggregate_fixtures -- --ignored`.",
        "cases": cases,
    });
    std::fs::create_dir_all(std::path::Path::new(FIXTURES).parent().unwrap()).unwrap();
    std::fs::write(
        FIXTURES,
        format!("{}\n", serde_json::to_string_pretty(&document).unwrap()),
    )
    .unwrap();
}

/// The committed fixtures agree with Sage and Klimper today: Sage's aggregate of the
/// makers' offers is the committed one, byte for byte; it and the concatenation carry every
/// maker's signature and count as that many offers.
#[test]
fn the_committed_fixtures_still_hold() {
    let text = std::fs::read_to_string(FIXTURES).expect("the fixture file is committed");
    let document: serde_json::Value = serde_json::from_str(&text).unwrap();
    let cases = document["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let makers: Vec<SpendBundle> = case["maker_offers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| decode_offer(o.as_str().unwrap()).unwrap())
            .collect();
        let expected = &case["expected"];
        let sage = aggregate_offers(makers.clone());
        assert_eq!(
            encode_offer(&sage).unwrap(),
            case["sage_aggregate"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(hex0x(sage.name()), expected["sage_aggregate_id"], "{name}");
        let concat = decode_offer(case["concatenated"].as_str().unwrap()).unwrap();
        assert_eq!(concat, join(&makers, Join::Concat), "{name}");
        let merged = decode_offer(case["merged"].as_str().unwrap()).unwrap();
        assert_eq!(merged, join(&makers, Join::Merged), "{name}");
        assert_eq!(hex0x(merged.name()), expected["merged_id"], "{name}");
        for bundle in [&sage, &concat, &merged] {
            check_maker_signature(bundle, &kit::Network::Testnet11).unwrap();
            assert_eq!(
                serde_json::json!(crate::trade::offers_in(bundle)),
                expected["offers"],
                "{name}"
            );
        }
        assert_eq!(expected["trade"]["offers"], expected["offers"], "{name}");
    }
}
