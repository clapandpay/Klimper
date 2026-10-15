// SPDX-License-Identifier: AGPL-3.0-or-later
//! The trade overview of an approval (clapandpay TASK-139): what the user pays, what the
//! user receives, the user's own fee, the effective price of a two-asset trade, and for a
//! take how many offers are taken together.
//!
//! Everything here is computed by the wallet: the amounts from wallet-kit's simulation of
//! the exact bundle that is signed (`Summary`, `BindingReport`), the number of offers from
//! the decoded offer itself. Nothing a website wrote is read (xchonnect spec 11.1). It
//! reaches the approval screen as the prompt's `trade` key, next to `cross_checked`, and the
//! screen reads it by explicit key (`xchonnectPrompt.ts`).
//!
//! What counts as received: what arrives from the user's own spends (`net`), plus what the
//! user's spends are **bound** to receive from the other side (`bound_received`, spec 11.2).
//! A settlement payment the user's spends assert cannot be left out of the bundle without
//! the user's own spends failing, so it is as certain as the user's own outputs. Anything
//! merely conditional and not bound is not counted.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chia_protocol::{Bytes32, CoinSpend, SpendBundle};
use serde::{Serialize, Serializer};
use xchonnect_wallet_kit::{AssetId, BindingReport, Prompt, Summary};

/// XCH is counted in mojos.
pub const XCH_PRECISION: u32 = 12;
/// CATs carry three decimals by convention (the approval screen assumes the same).
pub const CAT_PRECISION: u32 = 3;
/// Significant digits of a price.
const PRICE_DIGITS: usize = 6;

fn precision(asset: AssetId) -> u32 {
    match asset {
        AssetId::Xch => XCH_PRECISION,
        AssetId::Cat(_) => CAT_PRECISION,
    }
}

/// Amounts as decimal strings: a JSON number loses exactness past 2^53.
fn as_string<S: Serializer, T: ToString>(value: &T, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&value.to_string())
}

/// One asset of the trade and how much of it, in the asset's smallest unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Leg {
    pub asset: AssetId,
    #[serde(serialize_with = "as_string")]
    pub amount: u128,
}

/// The effective price of a trade with one asset on each side, both ways round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Price {
    /// The asset priced: the CAT when the other side is XCH, otherwise what the user
    /// receives.
    pub base: AssetId,
    /// The asset it is priced in.
    pub quote: AssetId,
    /// How much `quote` one whole `base` costs in this trade, as a decimal (cut, not
    /// rounded, to [`PRICE_DIGITS`] significant digits).
    pub quote_per_base: String,
    /// The same price the other way round.
    pub base_per_quote: String,
}

/// What the approval screen shows above everything else for a trade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TradeOverview {
    /// For a take: how many offers (distinct makers' payment groups) are taken together in
    /// this one bundle, all or nothing. `None` for anything that is not a take.
    pub offers: Option<usize>,
    /// What leaves the user's wallet, the fee not included. XCH first.
    pub pays: Vec<Leg>,
    /// What the user receives. XCH first.
    pub receives: Vec<Leg>,
    /// The fee the user pays: what the user's own spends reserve, as far as the user's
    /// own XCH funds it. A maker's fee is the maker's, and a fee the user's spends
    /// declare but another party's coin pays for is not the user's either.
    #[serde(serialize_with = "as_string")]
    pub fee: u64,
    /// For one asset paid for one asset received.
    pub price: Option<Price>,
}

/// The overview of a spend prompt, or `None` when the request is not a trade (nothing is
/// both paid and received) or is not a spend.
pub fn of_prompt(prompt: &Prompt<'_>, offers: Option<usize>) -> Option<TradeOverview> {
    match prompt {
        Prompt::SignCoinSpends {
            summary, binding, ..
        }
        | Prompt::SubmitCoinSpends {
            summary, binding, ..
        } => overview(summary, *binding, offers),
        Prompt::SignMessage { .. } => None,
    }
}

/// The overview from the simulation of the bundle and its binding report.
pub fn overview(
    summary: &Summary,
    binding: Option<&BindingReport>,
    offers: Option<usize>,
) -> Option<TradeOverview> {
    let mut change: BTreeMap<AssetId, i128> = BTreeMap::new();
    for delta in &summary.assets {
        *change.entry(delta.asset).or_default() += delta.net;
    }
    // The user's fee is the part of the fee the user's spends reserve that the user's own
    // XCH paid for: never more than the XCH that left the user's coins. A spend of the
    // user's may declare a `RESERVE_FEE` that someone else's coin funds (a settlement coin
    // in the same bundle needs no signature); counting all of it as the user's would turn
    // the fee into XCH "received" below.
    let own_xch = change.get(&AssetId::Xch).copied().unwrap_or(0);
    let funded = i128::from(summary.owned_reserve_fee).min(own_xch.min(0).saturating_neg());
    let fee = u64::try_from(funded).unwrap_or(0);
    // Only a binding that holds for every user spend makes the other side's payments
    // certain; a partial one is refused before the prompt anyway.
    if let Some(binding) = binding.filter(|b| b.all_bound) {
        for received in &binding.bound_received {
            let amount = i128::try_from(received.amount).ok()?;
            *change.entry(received.asset).or_default() += amount;
        }
    }
    // The fee is its own line: XCH paid for the trade is what leaves without it. Only
    // the part the user funded comes back here, so the fee can never show as received.
    if funded > 0 {
        *change.entry(AssetId::Xch).or_default() += funded;
    }

    let mut pays = Vec::new();
    let mut receives = Vec::new();
    // `AssetId` orders XCH first, then CATs by id: the same order every time.
    for (asset, delta) in change {
        let amount = delta.unsigned_abs();
        match delta.signum() {
            -1 => pays.push(Leg { asset, amount }),
            1 => receives.push(Leg { asset, amount }),
            _ => {}
        }
    }
    if pays.is_empty() || receives.is_empty() {
        return None;
    }
    let price = match (pays.as_slice(), receives.as_slice()) {
        ([paid], [received]) => price(*paid, *received),
        _ => None,
    };
    Some(TradeOverview {
        offers,
        pays,
        receives,
        fee,
        price,
    })
}

/// The price of a one-for-one trade: a CAT priced in XCH, otherwise what is received
/// priced in what is paid.
fn price(paid: Leg, received: Leg) -> Option<Price> {
    let (base, quote) = if paid.asset == AssetId::Xch {
        (received, paid)
    } else if received.asset == AssetId::Xch {
        (paid, received)
    } else {
        (received, paid)
    };
    Some(Price {
        base: base.asset,
        quote: quote.asset,
        quote_per_base: ratio(quote, base)?,
        base_per_quote: ratio(base, quote)?,
    })
}

/// `top / bottom` in whole units of each asset, as a decimal string.
fn ratio(top: Leg, bottom: Leg) -> Option<String> {
    let ten = |power: u32| 10_u128.checked_pow(power);
    let numerator = top.amount.checked_mul(ten(precision(bottom.asset))?)?;
    let denominator = bottom.amount.checked_mul(ten(precision(top.asset))?)?;
    decimal(numerator, denominator, PRICE_DIGITS)
}

/// `numerator / denominator` cut to `digits` significant digits (never rounded up), with
/// trailing zeros dropped. The whole part is always kept in full.
fn decimal(numerator: u128, denominator: u128, digits: usize) -> Option<String> {
    if denominator == 0 {
        return None;
    }
    let whole = numerator / denominator;
    let mut remainder = numerator % denominator;
    let mut text = whole.to_string();
    let mut significant = if whole == 0 { 0 } else { text.len() };
    let mut fraction = String::new();
    // At most 30 places: past that a price is not a price anybody reads.
    while remainder != 0 && significant < digits && fraction.len() < 30 {
        remainder = remainder.checked_mul(10)?;
        let digit = remainder / denominator;
        remainder %= denominator;
        fraction.push(char::from_digit(u32::try_from(digit).ok()?, 10)?);
        if significant > 0 || digit != 0 {
            significant += 1;
        }
    }
    let fraction = fraction.trim_end_matches('0');
    if !fraction.is_empty() {
        text.push('.');
        text.push_str(fraction);
    }
    Some(text)
}

/// The root of `node` in a union-find forest, halving the path on the way.
fn root(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

/// What ties two of a maker's spends together in [`offers_in`].
const COIN_ANNOUNCEMENT: u8 = 0;
const PUZZLE_ANNOUNCEMENT: u8 = 1;
const COIN: u8 = 2;
const PUZZLE: u8 = 3;

/// How many offers a decoded offer bundle takes together, counted so that one maker cannot
/// make one offer look like many (TASK-139 audit, L1).
///
/// Two counts, and the smaller is shown:
///
/// * the distinct nonces of the requested payments: every offer notarizes what it asks
///   with a nonce of its own, but a maker chooses nonces freely, so one offer can carry
///   five;
/// * the separate groups of the maker's coins: spends joined by an announcement one makes
///   and another asserts (the ring every wallet ties an offer's coins with, a CAT's own
///   ring), by asserting the same announcement (two coins of one offer asserting the same
///   payment), by a concurrent-spend assertion, or by sitting at one puzzle hash (one
///   owner). Coins of one offer are tied together,
///   or one could be taken out of the bundle; coins of different makers are not.
///
/// A maker who splits an offer into untied groups, one nonce each, has in effect made that
/// many offers: each group could be taken on its own. An offer that cannot be read or run
/// counts as one; it is refused on its own grounds.
pub fn offers_in(offer: &SpendBundle) -> usize {
    use chia_wallet_sdk::prelude::Condition;
    use chia_wallet_sdk::types::announcement_id;
    use xchonnect_wallet_kit::simulate::{DEFAULT_MAX_COST, execute};

    let mut allocator = chia_wallet_sdk::prelude::Allocator::new();
    let Ok(parsed) = chia_wallet_sdk::driver::Offer::from_spend_bundle(&mut allocator, offer)
    else {
        return 1;
    };
    let requested = parsed.requested_payments();
    let nonces: BTreeSet<Bytes32> = requested
        .xch
        .iter()
        .chain(requested.cats.values().flatten())
        .chain(requested.nfts.values().flatten())
        .chain(requested.options.values().flatten())
        .map(|payment| payment.nonce)
        .collect();

    // The maker's spends, run on their own; the asked-for placeholders (parent all zeroes)
    // are nobody's, and a requested CAT's placeholder does not run as a CAT spend.
    let makers: Vec<CoinSpend> = offer
        .coin_spends
        .iter()
        .filter(|spend| spend.coin.parent_coin_info != Bytes32::default())
        .cloned()
        .collect();
    let Ok((makers, _)) = execute(
        &mut allocator,
        &makers,
        &xchonnect_wallet_kit::Ownership::default(),
        DEFAULT_MAX_COST,
    ) else {
        return 1;
    };

    // Union-find over the maker's spends (0..n) and the things that tie them (n..).
    let mut parent: Vec<usize> = (0..makers.len()).collect();
    let mut ties: HashMap<(u8, Bytes32), usize> = HashMap::new();
    let mut tie = |parent: &mut Vec<usize>, spend: usize, key: (u8, Bytes32)| {
        let node = *ties.entry(key).or_insert_with(|| {
            parent.push(parent.len());
            parent.len() - 1
        });
        let (a, b) = (root(parent, spend), root(parent, node));
        parent[a] = b;
    };
    for (index, spend) in makers.iter().enumerate() {
        let coin_id = spend.coin.coin_id();
        tie(&mut parent, index, (COIN, coin_id));
        tie(&mut parent, index, (PUZZLE, spend.coin.puzzle_hash));
        for condition in &spend.conditions {
            let key = match condition {
                Condition::CreateCoinAnnouncement(c) => {
                    (COIN_ANNOUNCEMENT, announcement_id(coin_id, &c.message))
                }
                Condition::AssertCoinAnnouncement(c) => (COIN_ANNOUNCEMENT, c.announcement_id),
                Condition::CreatePuzzleAnnouncement(c) => (
                    PUZZLE_ANNOUNCEMENT,
                    announcement_id(spend.coin.puzzle_hash, &c.message),
                ),
                Condition::AssertPuzzleAnnouncement(c) => (PUZZLE_ANNOUNCEMENT, c.announcement_id),
                Condition::AssertConcurrentSpend(c) => (COIN, c.coin_id),
                Condition::AssertConcurrentPuzzle(c) => (PUZZLE, c.puzzle_hash),
                _ => continue,
            };
            tie(&mut parent, index, key);
        }
    }
    let groups: BTreeSet<usize> = (0..makers.len()).map(|i| root(&mut parent, i)).collect();

    nonces.len().min(groups.len()).max(1)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use xchonnect_wallet_kit::binding::AssetAmount;
    use xchonnect_wallet_kit::{AssetDelta, TimeLocks};

    fn cat(byte: u8) -> AssetId {
        AssetId::Cat(Bytes32::new([byte; 32]))
    }

    fn delta(asset: AssetId, net: i128, conditional: u128) -> AssetDelta {
        AssetDelta {
            asset,
            sent: net.min(0).unsigned_abs(),
            received: u128::try_from(net.max(0)).unwrap(),
            conditional_received: conditional,
            net,
        }
    }

    fn summary(assets: Vec<AssetDelta>, fee: u64) -> Summary {
        Summary {
            assets,
            implied_fee: Some(fee),
            reserve_fee: fee,
            owned_reserve_fee: fee,
            time_locks: TimeLocks::default(),
            unknown_puzzles: vec![],
            cost: 0,
            spends: vec![],
            outputs: vec![],
        }
    }

    fn bound(received: Vec<(AssetId, u128)>, all_bound: bool) -> BindingReport {
        BindingReport {
            all_bound,
            unbound_user_spends: vec![],
            bound_payments: vec![],
            bound_received: received
                .into_iter()
                .map(|(asset, amount)| AssetAmount { asset, amount })
                .collect(),
        }
    }

    #[test]
    fn a_take_pays_xch_for_a_cat_at_a_price() {
        // 1.5 XCH and a 0.0001 XCH fee leave; 3000.000 of a CAT arrive by settlement.
        let fee = 100_000_000;
        let s = summary(
            vec![
                delta(AssetId::Xch, -1_500_000_000_000 - i128::from(fee), 0),
                delta(cat(1), 0, 3_000_000),
            ],
            fee,
        );
        let trade = overview(&s, Some(&bound(vec![(cat(1), 3_000_000)], true)), Some(3)).unwrap();
        assert_eq!(trade.offers, Some(3));
        assert_eq!(
            trade.pays,
            [Leg {
                asset: AssetId::Xch,
                amount: 1_500_000_000_000
            }]
        );
        assert_eq!(
            trade.receives,
            [Leg {
                asset: cat(1),
                amount: 3_000_000
            }]
        );
        assert_eq!(trade.fee, fee);
        let price = trade.price.unwrap();
        assert_eq!(price.base, cat(1));
        assert_eq!(price.quote, AssetId::Xch);
        assert_eq!(price.quote_per_base, "0.0005");
        assert_eq!(price.base_per_quote, "2000");
    }

    /// The exploit of the TASK-139 audit: the user's CAT spend pays 1000 of the CAT away
    /// and declares a `RESERVE_FEE` of F, while a settlement coin of the website's (no
    /// signature needed) pays it. The user's XCH does not move, so nothing of F is the
    /// user's: no XCH received, and no fee of the user's.
    #[test]
    fn a_reserve_fee_funded_by_others_is_neither_received_nor_the_users() {
        let f = 50_000_000;
        let s = summary(vec![delta(cat(1), -1_000_000, 0)], f);
        assert_eq!(overview(&s, None, None), None);
        // Even with something bound to come back, F is not received and not the user's.
        let b = bound(vec![(cat(2), 10)], true);
        let trade = overview(&s, Some(&b), None).unwrap();
        assert!(trade.receives.iter().all(|l| l.asset != AssetId::Xch));
        assert_eq!(
            trade.receives,
            [Leg {
                asset: cat(2),
                amount: 10
            }]
        );
        assert_eq!(trade.fee, 0);
    }

    /// Part of a declared fee paid by the user's XCH, the rest by someone else's coin: the
    /// user's fee is the part the user paid, and XCH paid for the trade stays zero.
    #[test]
    fn a_reserve_fee_partly_funded_by_others_counts_only_the_users_part() {
        let s = summary(
            vec![delta(AssetId::Xch, -30, 0), delta(cat(1), 0, 1_000)],
            100,
        );
        let trade = overview(&s, Some(&bound(vec![(cat(1), 1_000)], true)), None);
        // The user pays 30 of fee and nothing else: no trade (nothing is paid).
        assert_eq!(trade, None);
        let s = summary(vec![delta(AssetId::Xch, -30, 0), delta(cat(2), -5, 0)], 100);
        let trade = overview(&s, Some(&bound(vec![(cat(1), 1_000)], true)), None).unwrap();
        assert_eq!(trade.fee, 30);
        assert_eq!(
            trade.pays,
            [Leg {
                asset: cat(2),
                amount: 5
            }]
        );
        assert_eq!(
            trade.receives,
            [Leg {
                asset: cat(1),
                amount: 1_000
            }]
        );
    }

    /// Selling a CAT for XCH with a fee: the user's XCH pays the fee and receives the
    /// price; the fee is the user's in full and the price is not reduced by it.
    #[test]
    fn a_sale_with_a_fee_paid_from_the_users_xch() {
        let s = summary(
            vec![delta(AssetId::Xch, -100, 0), delta(cat(1), -1_000, 0)],
            100,
        );
        let trade = overview(&s, Some(&bound(vec![(AssetId::Xch, 400)], true)), None).unwrap();
        assert_eq!(trade.fee, 100);
        assert_eq!(
            trade.receives,
            [Leg {
                asset: AssetId::Xch,
                amount: 400
            }]
        );
        assert_eq!(
            trade.pays,
            [Leg {
                asset: cat(1),
                amount: 1_000
            }]
        );
    }

    #[test]
    fn what_is_not_bound_is_not_received() {
        let s = summary(
            vec![delta(AssetId::Xch, -500, 0), delta(cat(1), 0, 1_000)],
            0,
        );
        assert_eq!(overview(&s, None, None), None);
        assert_eq!(
            overview(&s, Some(&bound(vec![(cat(1), 1_000)], false)), None),
            None
        );
    }

    #[test]
    fn a_send_is_no_trade() {
        let s = summary(vec![delta(AssetId::Xch, -1_000, 0)], 10);
        assert_eq!(overview(&s, None, None), None);
    }

    #[test]
    fn many_assets_are_listed_and_have_no_single_price() {
        let s = summary(
            vec![
                delta(AssetId::Xch, 0, 0),
                delta(cat(3), -1_000, 0),
                delta(cat(1), 0, 0),
                delta(cat(2), 0, 0),
            ],
            0,
        );
        let b = bound(vec![(cat(1), 10), (cat(2), 20), (AssetId::Xch, 5)], true);
        let trade = overview(&s, Some(&b), Some(2)).unwrap();
        assert_eq!(trade.pays.len(), 1);
        assert_eq!(
            trade.receives.iter().map(|l| l.asset).collect::<Vec<_>>(),
            [AssetId::Xch, cat(1), cat(2)]
        );
        assert_eq!(trade.price, None);
    }

    #[test]
    fn a_cat_for_a_cat_is_priced_in_what_is_paid() {
        let s = summary(vec![delta(cat(2), -3_000, 0), delta(cat(1), 1_000, 0)], 0);
        let price = overview(&s, None, None).unwrap().price.unwrap();
        assert_eq!(price.base, cat(1));
        assert_eq!(price.quote_per_base, "3");
        assert_eq!(price.base_per_quote, "0.333333");
    }

    #[test]
    fn decimals_are_cut_never_rounded_up() {
        assert_eq!(decimal(2, 3, 6).unwrap(), "0.666666");
        assert_eq!(decimal(10, 1, 6).unwrap(), "10");
        assert_eq!(decimal(1_234_567_891, 1, 6).unwrap(), "1234567891");
        assert_eq!(decimal(1, 8, 6).unwrap(), "0.125");
        assert_eq!(decimal(1, 3_000_000, 3).unwrap(), "0.000000333");
        assert_eq!(decimal(1, 0, 6), None);
    }

    #[test]
    fn the_overview_serialises_amounts_as_strings() {
        let s = summary(
            vec![delta(AssetId::Xch, -500, 0), delta(cat(1), 1_000, 0)],
            0,
        );
        let json = serde_json::to_value(overview(&s, None, None).unwrap()).unwrap();
        assert_eq!(json["pays"][0]["amount"], "500");
        assert_eq!(json["pays"][0]["asset"]["type"], "xch");
        assert_eq!(json["receives"][0]["asset"]["type"], "cat");
        assert_eq!(json["receives"][0]["asset"]["asset_id"], "01".repeat(32));
        assert_eq!(json["fee"], "0");
        assert_eq!(json["offers"], serde_json::Value::Null);
        // 500 mojos for 1.000 of the CAT.
        assert_eq!(json["price"]["quote_per_base"], "0.0000000005");
    }
}
