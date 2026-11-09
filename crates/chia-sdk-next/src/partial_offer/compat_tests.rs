//! Klimper only, not from the SDK: partial offers made by Klimper 1.1 and 1.2's own driver
//! (the first coin made from a one-sided offer's settlement coins and a security coin, signed
//! with a one-time key) read, filled and clawed back with the SDK's driver.
//!
//! The texts in `tests/fixtures/klimper-1.2/` were made by that driver on the same simulated
//! chain these tests build (the parties' keys and coins are deterministic), for every pairing
//! with and without a fee per fill and an expiry. Klimper 1.2 users have such offers live.

use anyhow::Result;
use chia_sdk_test::Simulator;
use rstest::rstest;

use super::tests::{Kind, Party};
use crate::decode_partial_offer;
use crate::sdk::{
    Conditions, PartialOffer, PartialOfferBundle, Puzzle, SpendContext, SpendWithConditions,
    StandardLayer,
};

const OFFERED: u64 = 100_000;
const REQUESTED: u64 = 20_000;
const MIN_FILL: u64 = 1_000;

#[rstest]
#[case::xch_for_cat(Kind::Xch, Kind::Cat)]
#[case::cat_for_xch(Kind::Cat, Kind::Xch)]
#[case::cat_for_cat(Kind::Cat, Kind::Cat)]
#[case::xch_for_revocable_cat(Kind::Xch, Kind::RevocableCat)]
#[case::revocable_cat_for_xch(Kind::RevocableCat, Kind::Xch)]
#[case::cat_for_revocable_cat(Kind::Cat, Kind::RevocableCat)]
fn klimper_1_2_offers_work_with_the_sdk_driver(
    #[case] offered: Kind,
    #[case] requested: Kind,
    #[values(None, Some(4_200_000))] required_fee: Option<u64>,
    #[values(false, true)] expires: bool,
) -> Result<()> {
    let mut sim = Simulator::new();
    let ctx = &mut SpendContext::new();
    let fees = 2 * required_fee.unwrap_or(0);
    // The maker's coins as Klimper 1.2's one-sided offer spent them: the offered amount and
    // one mojo of XCH for the security coin.
    let maker = if offered == Kind::Xch {
        Party::new(&mut sim, ctx, offered, OFFERED + 1, 0)?
    } else {
        Party::new(&mut sim, ctx, offered, OFFERED, 1)?
    };
    let taker = Party::new(&mut sim, ctx, requested, REQUESTED, fees)?;
    let expiration = expires.then(|| sim.next_timestamp() + 10_000);

    let name = format!(
        "{offered:?}-{requested:?}-{}-{}.txt",
        required_fee.map_or("nofee", |_| "fee"),
        if expires { "expires" } else { "noexpiry" }
    );
    let text = std::fs::read_to_string(format!(
        "{}/tests/fixtures/klimper-1.2/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))?;

    // The SDK reads the terms Klimper 1.2 made.
    let bundle = PartialOfferBundle::from_spend_bundle(ctx, &decode_partial_offer(text.trim())?)?;
    let first = *bundle.partial_offer();
    assert_eq!(first.coin.amount, OFFERED);
    assert_eq!(first.coin.puzzle_hash, first.info.puzzle_hash());
    assert_eq!(first.info.offered_asset, maker.asset);
    assert_eq!(first.info.requested_asset, taker.asset);
    assert_eq!(first.info.maker_puzzle_hash, maker.puzzle_hash);
    assert_eq!(first.info.expiration, expiration);
    assert_eq!(first.info.required_fee, required_fee);
    assert_eq!(first.info.min_requested_amount.get(), MIN_FILL);
    assert_eq!(first.info.offered_amount(REQUESTED), Some(OFFERED));

    // The first fill carries the maker's spends; the SDK reads it back from the chain.
    let (spend_bundle, child) = taker.take(ctx, &bundle, REQUESTED / 4, 0)?;
    sim.new_transaction(spend_bundle)?;
    let child = child.expect("a quarter leaves the rest");
    assert_eq!(child.coin.amount, OFFERED * 3 / 4);
    let spend = sim.coin_spend(first.coin.coin_id()).expect("filled");
    let puzzle = ctx.alloc(&spend.puzzle_reveal)?;
    let solution = ctx.alloc(&spend.solution)?;
    let puzzle = Puzzle::parse(ctx, puzzle);
    let (_, paid) =
        PartialOffer::parse(ctx, first.coin, puzzle, solution, Some(taker.asset))?.expect("a fill");
    assert_eq!(paid, REQUESTED / 4);
    let next = PartialOffer::parse_child(ctx, first.coin, puzzle, solution, Some(taker.asset))?;
    assert_eq!(next.map(|next| next.coin), Some(child.coin));
    assert_eq!(taker.balance_of(&sim, maker.puzzle_hash), REQUESTED / 4);

    // The maker claws back the rest with the SDK.
    let hint = ctx.hint(maker.puzzle_hash)?;
    let inner = StandardLayer::new(maker.pk).spend_with_conditions(
        ctx,
        Conditions::new().create_coin(maker.puzzle_hash, child.coin.amount, hint),
    )?;
    child.claw_back(ctx, inner)?;
    sim.spend_coins(ctx.take(), std::slice::from_ref(&maker.sk))?;
    assert_eq!(maker.balance_of(&sim, maker.puzzle_hash), OFFERED * 3 / 4);
    Ok(())
}
