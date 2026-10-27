// SPDX-License-Identifier: Apache-2.0
//! Making a partial offer from an ordinary one-sided offer, as the reference does: the wallet
//! makes and signs an offer of the offered asset (plus one mojo of XCH) for nothing, and this
//! turns its settlement coins into the first partial coin and a security coin.

use chia_wallet_sdk::{
    chia::{
        bls::{SecretKey, Signature, sign},
        protocol::{Bytes32, SpendBundle},
        puzzle_types::{
            Memos,
            offer::{NotarizedPayment, Payment, SettlementPaymentsSolution},
        },
    },
    driver::{Cat, CatSpend, Offer, Spend, SpendContext, StandardLayer, create_security_coin},
    puzzles::SETTLEMENT_PAYMENT_HASH,
    signer::{AggSigConstants, RequiredSignature},
    types::{Conditions, announcement_id, puzzles::SettlementPayment},
};

use crate::{
    PartialAsset, PartialOffer, PartialOfferError, PartialOfferInfo, PartialPriceData, Result,
};

/// What the maker asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartialOfferTerms {
    pub offered: PartialAsset,
    pub offered_amount: u64,
    pub requested: PartialAsset,
    pub requested_amount: u64,
    /// The least of the requested asset one fill pays (at least 1).
    pub min_fill_requested: u64,
    /// Unix seconds; no fill is valid at or after it.
    pub expiration: Option<u64>,
    /// The blockchain fee each fill must pay.
    pub required_fee: Option<u64>,
    /// Where payments go and who may cancel: a standard puzzle hash of the maker's.
    pub maker_puzzle_hash: Bytes32,
}

impl PartialOfferTerms {
    pub fn check(&self) -> Result<()> {
        let refuse = |reason: &str| Err(PartialOfferError::Invalid(reason.to_string()));
        if self.offered == self.requested {
            return refuse("a partial offer of an asset for itself");
        }
        if self.offered_amount == 0 || self.requested_amount == 0 {
            return refuse("a partial offer needs both amounts");
        }
        if self.min_fill_requested == 0 || self.min_fill_requested > self.requested_amount {
            return refuse("the minimum fill is between 1 and the requested amount");
        }
        let price = self.price_data();
        if price.quote(self.min_fill_requested) == 0 {
            return refuse("the minimum fill buys nothing at this price");
        }
        Ok(())
    }

    pub fn price_data(&self) -> PartialPriceData {
        PartialPriceData {
            price_precision: self.offered_amount,
            precision: self.requested_amount,
        }
    }

    /// The one-sided offer to make first: this much XCH and, for a CAT, this much of it. The
    /// extra mojo of XCH funds the security coin.
    pub fn one_sided_offer(&self) -> (u64, Option<(Bytes32, u64)>) {
        match self.offered.asset_id {
            None => (self.offered_amount + 1, None),
            Some(asset_id) => (1, Some((asset_id, self.offered_amount))),
        }
    }
}

/// Turns the maker's signed one-sided offer into a partial offer. `agg_sig` is the network's
/// (the security coin's signature is made here, with a key used once).
pub fn make_partial_offer(
    one_sided_offer: &SpendBundle,
    terms: &PartialOfferTerms,
    agg_sig: &AggSigConstants,
) -> Result<PartialOffer> {
    terms.check()?;
    let mut ctx = SpendContext::new();
    let offer = Offer::from_spend_bundle(&mut ctx, one_sided_offer)?;
    let (xch, cat) = terms.one_sided_offer();

    let xch_coins = &offer.offered_coins().xch;
    let [xch_coin] = xch_coins.as_slice() else {
        return Err(PartialOfferError::Invalid(format!(
            "the one-sided offer has {} XCH coins, not one",
            xch_coins.len()
        )));
    };
    let xch_coin = *xch_coin;
    if xch_coin.amount != xch {
        return Err(PartialOfferError::Invalid(format!(
            "the one-sided offer offers {} mojos of XCH, not {xch}",
            xch_coin.amount
        )));
    }
    let cat_coin = match cat {
        None => None,
        Some((asset_id, amount)) => {
            let cats = offer
                .offered_coins()
                .cats
                .get(&asset_id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let [cat_coin] = cats else {
                return Err(PartialOfferError::Invalid(format!(
                    "the one-sided offer has {} coins of the CAT, not one",
                    cats.len()
                )));
            };
            if cat_coin.coin.amount != amount {
                return Err(PartialOfferError::Invalid(format!(
                    "the one-sided offer offers {} of the CAT, not {amount}",
                    cat_coin.coin.amount
                )));
            }
            if cat_coin.info.hidden_puzzle_hash != terms.offered.hidden_puzzle_hash {
                return Err(PartialOfferError::Invalid(
                    "the offered CAT's revocation does not match".to_string(),
                ));
            }
            Some(*cat_coin)
        }
    };
    if !offer.requested_payments().xch.is_empty()
        || !offer.requested_payments().cats.is_empty()
        || !offer.requested_payments().nfts.is_empty()
    {
        return Err(PartialOfferError::Invalid(
            "the one-sided offer requests something".to_string(),
        ));
    }

    let info = PartialOfferInfo {
        lineage_proof: cat_coin.map(|cat| cat.child_lineage_proof()),
        offered_asset_info: terms.offered,
        requested_asset_info: terms.requested,
        maker_puzzle_hash: terms.maker_puzzle_hash,
        expiration: terms.expiration,
        required_fee: terms.required_fee.filter(|fee| *fee > 0),
        price_data: terms.price_data(),
        min_other_asset_amount_minus_one: terms.min_fill_requested - 1,
    };

    // The XCH settlement coin pays the security coin (and, for XCH, the partial coin).
    let (security_sk, security_coin) = create_security_coin(&mut ctx, xch_coin)?;
    let _ = ctx.take(); // the spend it made; the one below replaces it
    let settlement = ctx.alloc_mod::<SettlementPayment>()?;
    let partial_payment = Payment::new(
        info.inner_puzzle_hash().into(),
        terms.offered_amount,
        Memos::None,
    );
    let security_payment =
        Payment::new(security_coin.puzzle_hash, security_coin.amount, Memos::None);
    let xch_payments = if cat_coin.is_none() {
        vec![partial_payment.clone(), security_payment]
    } else {
        vec![security_payment]
    };
    // The settlement coins can be spent by anyone, and the maker's signed spends create them.
    // What makes them pay into the partial coin and nowhere else is the security coin: its
    // spend asserts each settlement coin's announcement of exactly these payments, and its
    // signature is aggregated with the maker's, so it can neither be dropped nor kept with
    // the settlement coins spent another way. (The reference spends it to a bare remark,
    // which leaves the maker's coins to whoever reads the offer first.)
    let xch_notarized = NotarizedPayment::new(Bytes32::default(), xch_payments);
    let xch_ptr = ctx.alloc(&xch_notarized)?;
    let xch_announcement = announcement_id(
        SETTLEMENT_PAYMENT_HASH.into(),
        ctx.tree_hash(xch_ptr).to_vec(),
    );
    let mut assertions = Conditions::new().assert_puzzle_announcement(xch_announcement);
    let solution = ctx.alloc(&SettlementPaymentsSolution {
        notarized_payments: vec![xch_notarized],
    })?;
    ctx.spend(xch_coin, Spend::new(settlement, solution))?;

    if let Some(cat_coin) = cat_coin {
        let cat_notarized = NotarizedPayment::new(Bytes32::default(), vec![partial_payment]);
        let cat_ptr = ctx.alloc(&cat_notarized)?;
        assertions = assertions.assert_puzzle_announcement(announcement_id(
            PartialOfferInfo::full_asset_puzzle_hash(terms.offered, SETTLEMENT_PAYMENT_HASH.into()),
            ctx.tree_hash(cat_ptr).to_vec(),
        ));
        let solution = ctx.alloc(&SettlementPaymentsSolution {
            notarized_payments: vec![cat_notarized],
        })?;
        let _ = Cat::spend_all(
            &mut ctx,
            &[CatSpend::new(cat_coin, Spend::new(settlement, solution))],
        )?;
    }

    let security_signature =
        spend_security_coin(&mut ctx, security_coin, assertions, &security_sk, agg_sig)?;

    let parent_coin_id = cat_coin.map_or(xch_coin.coin_id(), |cat| cat.coin.coin_id());
    let mut partial = PartialOffer::new(parent_coin_id, terms.offered_amount, info);
    let mut coin_spends = ctx.take();
    coin_spends.extend(offer.spend_bundle().coin_spends.iter().cloned());
    partial.spend_bundle = SpendBundle::new(
        coin_spends,
        security_signature + &offer.spend_bundle().aggregated_signature,
    );
    Ok(partial)
}

/// Spends the security coin with `conditions` and signs it with its one-time key.
fn spend_security_coin(
    ctx: &mut SpendContext,
    security_coin: chia_wallet_sdk::chia::protocol::Coin,
    conditions: Conditions,
    sk: &SecretKey,
    agg_sig: &AggSigConstants,
) -> Result<Signature> {
    let before = ctx.take();
    StandardLayer::new(sk.public_key()).spend(ctx, security_coin, conditions)?;
    let spends = ctx.take();
    let required = RequiredSignature::from_coin_spends(ctx, &spends, agg_sig)
        .map_err(|error| PartialOfferError::Invalid(error.to_string()))?;
    let mut signature = Signature::default();
    for required in required {
        let RequiredSignature::Bls(required) = required else {
            return Err(PartialOfferError::Invalid(
                "the security coin asks for a secp signature".to_string(),
            ));
        };
        signature += &sign(sk, required.message());
    }
    for spend in before.into_iter().chain(spends) {
        ctx.insert(spend);
    }
    Ok(signature)
}
