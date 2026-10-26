// SPDX-License-Identifier: Apache-2.0
//! Partial offers on a simulated chain: made from a one-sided offer, filled in two parts by
//! takers' offers, read back from their text after each fill, and clawed back. For every
//! pairing of XCH, a CAT and a revocable CAT, with and without a required fee and an expiry.
#![allow(clippy::unwrap_used, clippy::too_many_lines)]

use anyhow::Result;
use chia_partial_offer::{
    PartialAsset, PartialOffer, PartialOfferTerms, PartialSpend, is_partial_offer,
    make_partial_offer,
};
use chia_wallet_sdk::{
    chia::{
        bls::{PublicKey, SecretKey},
        protocol::{Bytes32, Coin, SpendBundle},
        puzzle_types::{
            Memos,
            offer::{NotarizedPayment, Payment},
        },
    },
    driver::{
        AssetInfo, Cat, CatAssetInfo, CatSpend, Offer, OfferCoins, RequestedPayments, Spend,
        SpendContext, SpendWithConditions, StandardLayer,
    },
    puzzles::SETTLEMENT_PAYMENT_HASH,
    signer::AggSigConstants,
    test::{BlsPair, Simulator, sign_transaction},
    types::{Conditions, TESTNET11_CONSTANTS, announcement_id},
};
use clvm_traits::clvm_quote;
use clvmr::NodePtr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Xch,
    Cat,
    RevocableCat,
}

const OFFERED: u64 = 100_000;
const REQUESTED: u64 = 20_000;
const MIN_FILL: u64 = 1_000;

fn agg_sig() -> AggSigConstants {
    AggSigConstants::new(TESTNET11_CONSTANTS.agg_sig_me_additional_data)
}

struct Party {
    sk: SecretKey,
    pk: PublicKey,
    puzzle_hash: Bytes32,
}

fn party(index: u32) -> Party {
    use chia_wallet_sdk::chia::bls::DerivableKey;
    let pair = BlsPair::default();
    let sk = pair.sk.derive_unhardened(index);
    let pk = sk.public_key();
    let puzzle_hash =
        chia_wallet_sdk::chia::puzzle_types::standard::StandardArgs::curry_tree_hash(pk).into();
    Party {
        sk,
        pk,
        puzzle_hash,
    }
}

/// Issues a CAT to `owner` in coins of `amounts`, returns them.
fn issue(
    sim: &mut Simulator,
    ctx: &mut SpendContext,
    owner: &Party,
    kind: Kind,
    amounts: &[u64],
) -> Result<Vec<Cat>> {
    let total: u64 = amounts.iter().sum();
    let parent = sim.new_coin(owner.puzzle_hash, total);
    let mut conditions = Conditions::new();
    for amount in amounts {
        conditions = conditions.create_coin(owner.puzzle_hash, *amount, Memos::None);
    }
    let hidden = (kind == Kind::RevocableCat).then(Bytes32::default);
    let (issue, cats) = Cat::single_issuance(ctx, parent.coin_id(), hidden, total, conditions)?;
    StandardLayer::new(owner.pk).spend(ctx, parent, issue)?;
    sim.spend_coins(ctx.take(), std::slice::from_ref(&owner.sk))?;
    // The issuance's outputs: the owner's coins, in order.
    assert_eq!(cats.len(), amounts.len());
    Ok(cats)
}

fn asset(kind: Kind, cat: Option<&Cat>) -> PartialAsset {
    if kind == Kind::Xch {
        return PartialAsset::xch();
    }
    let cat = cat.unwrap();
    PartialAsset::cat(cat.info.asset_id, cat.info.hidden_puzzle_hash)
}

/// The maker's signed one-sided offer: the offered asset (and one mojo of XCH) into
/// settlement coins, requesting nothing. What a wallet's `make_offer` produces.
fn one_sided_offer(
    sim: &mut Simulator,
    ctx: &mut SpendContext,
    maker: &Party,
    terms: &PartialOfferTerms,
    maker_cat: Option<Cat>,
) -> Result<SpendBundle> {
    let (xch, cat) = terms.one_sided_offer();
    let xch_coin = sim.new_coin(maker.puzzle_hash, xch);
    StandardLayer::new(maker.pk).spend(
        ctx,
        xch_coin,
        Conditions::new().create_coin(SETTLEMENT_PAYMENT_HASH.into(), xch, Memos::None),
    )?;
    if let Some((_, amount)) = cat {
        let maker_cat = maker_cat.unwrap();
        let inner = StandardLayer::new(maker.pk).spend_with_conditions(
            ctx,
            Conditions::new().create_coin(SETTLEMENT_PAYMENT_HASH.into(), amount, Memos::None),
        )?;
        let _ = Cat::spend_all(ctx, &[CatSpend::new(maker_cat, inner)])?;
    }
    let spends = ctx.take();
    let signature = sign_transaction(&spends, std::slice::from_ref(&maker.sk))?;
    Ok(SpendBundle::new(spends, signature))
}

/// The taker's offer for one fill: `paid` of the requested asset (plus the required fee in
/// XCH) into settlement coins, requesting what it buys of the offered asset.
#[allow(clippy::too_many_arguments)]
fn taker_offer(
    sim: &mut Simulator,
    ctx: &mut SpendContext,
    taker: &Party,
    partial: &PartialOffer,
    paid: u64,
    taker_cat: Option<Cat>,
    nonce: Bytes32,
) -> Result<Offer> {
    let info = &partial.info;
    let fee = info.required_fee.unwrap_or(0);
    let bought = info.price_data.quote(paid);

    let requested_payment = NotarizedPayment::new(
        nonce,
        vec![Payment::new(
            taker.puzzle_hash,
            bought,
            ctx.hint(taker.puzzle_hash)?,
        )],
    );
    let payment_ptr = ctx.alloc(&requested_payment)?;
    let assertion = Conditions::new().assert_puzzle_announcement(announcement_id(
        info.settlement_puzzle_hash(),
        ctx.tree_hash(payment_ptr).to_vec(),
    ));

    let mut offered = OfferCoins::new();
    let mut asset_info = AssetInfo::new();
    if let Some(asset_id) = info.offered_asset_info.asset_id {
        asset_info.insert_cat(
            asset_id,
            CatAssetInfo::new(info.offered_asset_info.hidden_puzzle_hash),
        )?;
    }

    let xch_into_settlement = if info.requested_asset_info.is_xch() {
        paid + fee
    } else {
        fee
    };
    if xch_into_settlement > 0 {
        let coin = sim.new_coin(taker.puzzle_hash, xch_into_settlement);
        StandardLayer::new(taker.pk).spend(
            ctx,
            coin,
            assertion.clone().create_coin(
                SETTLEMENT_PAYMENT_HASH.into(),
                xch_into_settlement,
                Memos::None,
            ),
        )?;
        offered.xch.push(Coin::new(
            coin.coin_id(),
            SETTLEMENT_PAYMENT_HASH.into(),
            xch_into_settlement,
        ));
    }
    if let Some(cat) = taker_cat {
        assert_eq!(cat.coin.amount, paid);
        let inner = StandardLayer::new(taker.pk).spend_with_conditions(
            ctx,
            assertion.create_coin(SETTLEMENT_PAYMENT_HASH.into(), paid, Memos::None),
        )?;
        let _ = Cat::spend_all(ctx, &[CatSpend::new(cat, inner)])?;
        asset_info.insert_cat(
            cat.info.asset_id,
            CatAssetInfo::new(cat.info.hidden_puzzle_hash),
        )?;
        offered.cats.insert(
            cat.info.asset_id,
            vec![cat.child(SETTLEMENT_PAYMENT_HASH.into(), paid)],
        );
    }

    let mut requested = RequestedPayments::new();
    match info.offered_asset_info.asset_id {
        Some(asset_id) => {
            requested.cats.insert(asset_id, vec![requested_payment]);
        }
        None => requested.xch.push(requested_payment),
    }

    let spends = ctx.take();
    let signature = sign_transaction(&spends, std::slice::from_ref(&taker.sk))?;
    Ok(Offer::new(
        SpendBundle::new(spends, signature),
        offered,
        requested,
        asset_info,
    ))
}

/// What `owner` holds of `asset` on the chain.
fn balance(sim: &Simulator, owner: &Party, asset: PartialAsset) -> u64 {
    let puzzle_hash =
        chia_partial_offer::PartialOfferInfo::full_asset_puzzle_hash(asset, owner.puzzle_hash);
    sim.unspent_coins(puzzle_hash, false)
        .iter()
        .map(|coin| coin.amount)
        .sum()
}

fn run(
    offered_kind: Kind,
    requested_kind: Kind,
    required_fee: Option<u64>,
    expiration: Option<u64>,
) -> Result<()> {
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();
    let maker = party(1);
    let taker = party(2);

    let maker_cat = match offered_kind {
        Kind::Xch => None,
        kind => Some(issue(&mut sim, ctx, &maker, kind, &[OFFERED])?[0]),
    };
    // The taker's CAT comes in the two fills' coins: a quarter, then the rest.
    let taker_cats = match requested_kind {
        Kind::Xch => None,
        kind => Some(issue(
            &mut sim,
            ctx,
            &taker,
            kind,
            &[REQUESTED / 4, REQUESTED * 3 / 4],
        )?),
    };
    let offered = asset(offered_kind, maker_cat.as_ref());
    let requested = asset(requested_kind, taker_cats.as_ref().map(|cats| &cats[0]));

    let terms = PartialOfferTerms {
        offered,
        offered_amount: OFFERED,
        requested,
        requested_amount: REQUESTED,
        min_fill_requested: MIN_FILL,
        expiration,
        required_fee,
        maker_puzzle_hash: maker.puzzle_hash,
    };
    let maker_before = balance(&sim, &maker, requested);
    let one_sided = one_sided_offer(&mut sim, ctx, &maker, &terms, maker_cat)?;
    let made = make_partial_offer(&one_sided, &terms, &agg_sig())?;

    // The text a maker shares, and what a taker reads from it.
    let text = made.to_text()?;
    assert!(is_partial_offer(&text));
    let mut partial = PartialOffer::from_text(&text)?;
    assert!(partial.is_first());
    assert_eq!(partial.coin, made.coin);
    assert_eq!(partial.info, made.info);
    assert_eq!(partial.coin.amount, OFFERED);
    assert_eq!(
        partial.info.max_fill_requested(partial.coin.amount),
        REQUESTED
    );

    let mut received = 0;
    for (fill, paid) in [REQUESTED / 4, REQUESTED * 3 / 4].into_iter().enumerate() {
        let taker_cat = taker_cats.as_ref().map(|cats| cats[fill]);
        let offer = taker_offer(
            &mut sim,
            ctx,
            &taker,
            &partial,
            paid,
            taker_cat,
            Bytes32::new([u8::try_from(fill).unwrap() + 10; 32]),
        )?;
        let bought = partial.info.price_data.quote(paid);
        let next = partial.child(partial.coin.amount - bought);
        let bundle = partial.clone().accept_offer(ctx, offer)?;
        sim.new_transaction(bundle)?;
        received += bought;

        // Followed from the chain, the spend reads as this fill and the same next offer.
        let spend = sim.coin_spend(partial.coin.coin_id()).unwrap();
        match partial.follow(&spend)? {
            PartialSpend::Filled {
                bought: seen,
                next: followed,
            } => {
                assert_eq!(seen, bought);
                assert_eq!(
                    followed.map(|next| next.coin),
                    (next.coin.amount > 0).then_some(next.coin)
                );
            }
            PartialSpend::ClawedBack => panic!("a fill read as a claw back"),
        }

        assert_eq!(balance(&sim, &taker, offered), received);
        if next.coin.amount > 0 {
            // The next offer is on the chain, and its text reads back to the same coin.
            assert!(sim.coin_state(next.coin.coin_id()).is_some());
            let again = PartialOffer::from_text(&next.to_text()?)?;
            assert!(!again.is_first());
            assert_eq!(again.coin, next.coin);
        } else {
            assert!(sim.coin_state(next.coin.coin_id()).is_none());
        }
        partial = next;
    }
    assert_eq!(received, OFFERED);
    assert_eq!(balance(&sim, &maker, requested) - maker_before, REQUESTED);
    Ok(())
}

#[test]
fn every_pairing_fills_in_two_parts() {
    let kinds = [Kind::Xch, Kind::Cat, Kind::RevocableCat];
    for offered in kinds {
        for requested in kinds {
            if offered == Kind::Xch && requested == Kind::Xch {
                continue;
            }
            for required_fee in [None, Some(4_200_000)] {
                for expiration in [None, Some(1_900_000_000)] {
                    run(offered, requested, required_fee, expiration).unwrap_or_else(|error| {
                        panic!(
                            "{offered:?} for {requested:?}, fee {required_fee:?}, \
                             expiry {expiration:?}: {error}"
                        )
                    });
                }
            }
        }
    }
}

/// Makes a CAT-for-XCH (or `offered_kind`-for-XCH) partial offer and fills a quarter of it.
fn made_and_filled_once(
    sim: &mut Simulator,
    ctx: &mut SpendContext,
    maker: &Party,
    taker: &Party,
    offered_kind: Kind,
    expiration: Option<u64>,
) -> Result<PartialOffer> {
    let maker_cat = match offered_kind {
        Kind::Xch => None,
        kind => Some(issue(sim, ctx, maker, kind, &[OFFERED])?[0]),
    };
    let terms = PartialOfferTerms {
        offered: asset(offered_kind, maker_cat.as_ref()),
        offered_amount: OFFERED,
        requested: if offered_kind == Kind::Xch {
            // XCH for a CAT needs a CAT; keep these tests to one pairing per offered kind.
            unreachable!()
        } else {
            PartialAsset::xch()
        },
        requested_amount: REQUESTED,
        min_fill_requested: MIN_FILL,
        expiration,
        required_fee: None,
        maker_puzzle_hash: maker.puzzle_hash,
    };
    let one_sided = one_sided_offer(sim, ctx, maker, &terms, maker_cat)?;
    let partial = make_partial_offer(&one_sided, &terms, &agg_sig())?;
    let offer = taker_offer(
        sim,
        ctx,
        taker,
        &partial,
        REQUESTED / 4,
        None,
        Bytes32::new([3; 32]),
    )?;
    let next = partial.child(OFFERED - partial.info.price_data.quote(REQUESTED / 4));
    sim.new_transaction(partial.accept_offer(ctx, offer)?)?;
    Ok(next)
}

#[test]
fn the_maker_claws_back_what_is_left() -> Result<()> {
    for kind in [Kind::Cat, Kind::RevocableCat] {
        let mut sim = Simulator::new();
        let ctx = &mut SpendContext::new();
        let maker = party(1);
        let taker = party(2);
        let partial = made_and_filled_once(&mut sim, ctx, &maker, &taker, kind, None)?;
        let left = partial.coin.amount;

        let conditions = partial.claw_back_conditions(ctx)?;
        let delegated = Spend::new(ctx.alloc(&clvm_quote!(conditions))?, NodePtr::NIL);
        let inner = StandardLayer::new(maker.pk).delegated_inner_spend(ctx, delegated)?;
        partial.claw_back(ctx, inner)?;
        sim.spend_coins(ctx.take(), std::slice::from_ref(&maker.sk))?;

        assert_eq!(balance(&sim, &maker, partial.info.offered_asset_info), left);
        let spend = sim.coin_spend(partial.coin.coin_id()).unwrap();
        assert!(matches!(partial.follow(&spend)?, PartialSpend::ClawedBack));
        assert!(
            sim.coin_state(partial.coin.coin_id())
                .unwrap()
                .spent_height
                .is_some()
        );
    }
    Ok(())
}

#[test]
fn nobody_but_the_maker_can_claw_back() -> Result<()> {
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();
    let maker = party(1);
    let taker = party(2);
    let partial = made_and_filled_once(&mut sim, ctx, &maker, &taker, Kind::Cat, None)?;

    let conditions =
        Conditions::new().create_coin(taker.puzzle_hash, partial.coin.amount, Memos::None);
    let delegated = Spend::new(ctx.alloc(&clvm_quote!(conditions))?, NodePtr::NIL);
    // The taker's own key does not open the maker's leaf.
    let inner = StandardLayer::new(taker.pk).delegated_inner_spend(ctx, delegated)?;
    assert!(
        partial.claw_back(ctx, inner).is_err() || {
            sim.spend_coins(ctx.take(), std::slice::from_ref(&taker.sk))
                .is_err()
        }
    );
    Ok(())
}

#[test]
fn a_fill_below_the_minimum_or_above_what_is_left_is_refused() -> Result<()> {
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();
    let maker = party(1);
    let taker = party(2);
    let partial = made_and_filled_once(&mut sim, ctx, &maker, &taker, Kind::Cat, None)?;

    let too_small = taker_offer(
        &mut sim,
        ctx,
        &taker,
        &partial,
        MIN_FILL - 1,
        None,
        Bytes32::new([4; 32]),
    )?;
    assert!(partial.clone().accept_offer(ctx, too_small).is_err());
    let _ = ctx.take();

    let max = partial.info.max_fill_requested(partial.coin.amount);
    let too_big = taker_offer(
        &mut sim,
        ctx,
        &taker,
        &partial,
        max + 5,
        None,
        Bytes32::new([5; 32]),
    )?;
    assert!(partial.clone().accept_offer(ctx, too_big).is_err());
    let _ = ctx.take();

    // Exactly what is left empties it.
    let all = taker_offer(
        &mut sim,
        ctx,
        &taker,
        &partial,
        max,
        None,
        Bytes32::new([6; 32]),
    )?;
    sim.new_transaction(partial.clone().accept_offer(ctx, all)?)?;
    assert!(
        sim.coin_state(partial.coin.coin_id())
            .unwrap()
            .spent_height
            .is_some()
    );
    Ok(())
}

#[test]
fn an_expired_offer_cannot_be_filled() -> Result<()> {
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();
    let maker = party(1);
    let taker = party(2);
    let expires = sim.next_timestamp() + 1_000;
    let partial = made_and_filled_once(&mut sim, ctx, &maker, &taker, Kind::Cat, Some(expires))?;
    assert_eq!(partial.info.expiration, Some(expires));

    sim.pass_time(2_000);
    let offer = taker_offer(
        &mut sim,
        ctx,
        &taker,
        &partial,
        MIN_FILL,
        None,
        Bytes32::new([7; 32]),
    )?;
    let bundle = partial.accept_offer(ctx, offer)?;
    assert!(sim.new_transaction(bundle).is_err());
    Ok(())
}

#[test]
fn a_maker_cannot_ask_for_nothing_or_for_the_same_asset() {
    let terms = PartialOfferTerms {
        offered: PartialAsset::xch(),
        offered_amount: OFFERED,
        requested: PartialAsset::xch(),
        requested_amount: REQUESTED,
        min_fill_requested: MIN_FILL,
        expiration: None,
        required_fee: None,
        maker_puzzle_hash: Bytes32::default(),
    };
    assert!(terms.check().is_err());
    let cat = PartialAsset::cat(Bytes32::new([7; 32]), None);
    assert!(
        PartialOfferTerms {
            requested: cat,
            min_fill_requested: 0,
            ..terms
        }
        .check()
        .is_err()
    );
    assert!(
        PartialOfferTerms {
            requested: cat,
            min_fill_requested: REQUESTED + 1,
            ..terms
        }
        .check()
        .is_err()
    );
    assert!(
        PartialOfferTerms {
            requested: cat,
            ..terms
        }
        .check()
        .is_ok()
    );
    // Not a partial offer.
    assert!(PartialOffer::from_text("offer1qqqq").is_err());
    assert!(!is_partial_offer("offer1qqqq"));
}
