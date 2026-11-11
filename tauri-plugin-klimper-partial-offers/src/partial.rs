// SPDX-License-Identifier: AGPL-3.0-or-later
//! Partial offers as this plugin and Xchonnect use them, on chia-wallet-sdk's CHIP-0052 driver
//! (`chia-sdk-next`, a copy of the SDK's unreleased driver; docs/partial-offers.md).
//!
//! The SDK's driver does the puzzles, the text, the fills and the claw back. What is here is
//! what a wallet adds on top, from Klimper's own earlier driver:
//! - checks on a text from someone else that the SDK leaves to the wallet ([`check_info`]);
//! - the terms a maker asks for, checked so that nobody makes an offer no one can fill, and
//!   with the price as the maker's own amounts, never reduced ([`Terms`]);
//! - following a spend on the chain: a fill (and what is left) or the maker's claw back.

use std::num::NonZeroU64;

use chia_sdk_next::{
    DriverError as PartialDriverError, PartialOffer as SdkPartialOffer, PartialOfferAsset,
    PartialOfferBundle, PartialOfferInfo, PartialOfferPrice, decode_partial_offer,
    encode_partial_offer,
};
use chia_wallet_sdk::{
    chia::protocol::{Bytes32, Coin, CoinSpend, SpendBundle},
    driver::{DriverError, Puzzle, SpendContext},
};

/// Why a partial offer could not be read, made, filled or cancelled.
#[derive(Debug, thiserror::Error)]
pub enum PartialOfferError {
    #[error("not a partial offer: {0}")]
    NotPartial(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Driver(#[from] PartialDriverError),
}

impl From<DriverError> for PartialOfferError {
    fn from(error: DriverError) -> Self {
        Self::Driver(error.into())
    }
}

type Result<T, E = PartialOfferError> = std::result::Result<T, E>;

/// Whether `text` is a partial offer's text (`partial1…`), not an ordinary offer.
pub fn is_partial_offer(text: &str) -> bool {
    text.trim().to_lowercase().starts_with("partial1")
}

/// The CAT's asset id, or `None` for XCH.
pub fn asset_id(asset: PartialOfferAsset) -> Option<Bytes32> {
    match asset {
        PartialOfferAsset::Xch => None,
        PartialOfferAsset::Cat { asset_id, .. } => Some(asset_id),
    }
}

/// A revocable CAT's hidden puzzle hash.
pub fn hidden_puzzle_hash(asset: PartialOfferAsset) -> Option<Bytes32> {
    match asset {
        PartialOfferAsset::Xch => None,
        PartialOfferAsset::Cat {
            hidden_puzzle_hash, ..
        } => hidden_puzzle_hash,
    }
}

/// What the SDK accepts from a text and a wallet should not: the same asset on both sides, a
/// fill that can never pay out, and a CAT coin without the lineage proof it is spent with (or
/// an XCH coin with one).
pub fn check_info(offer: &SdkPartialOffer) -> Result<()> {
    let info = &offer.info;
    if info.offered_asset == info.requested_asset {
        return Err(PartialOfferError::Invalid(
            "a partial offer of an asset for itself".to_string(),
        ));
    }
    if info.offered_amount(info.min_requested_amount.get()) == Some(0) {
        return Err(PartialOfferError::Invalid(
            "the least fill of this partial offer buys nothing".to_string(),
        ));
    }
    if matches!(info.offered_asset, PartialOfferAsset::Xch) != offer.lineage_proof.is_none() {
        return Err(PartialOfferError::Invalid(
            "a lineage proof for XCH, or none for a CAT".to_string(),
        ));
    }
    Ok(())
}

/// A partial offer: its newest coin, and while that is the first one, the maker's spends that
/// create it (a taker sends them with the first fill).
#[derive(Debug, Clone)]
pub struct PartialOffer {
    bundle: PartialOfferBundle,
}

/// What a spend of a partial coin did.
#[derive(Debug, Clone)]
pub enum PartialSpend {
    /// A taker paid `paid` of the requested asset for `bought` of the offered one; `next` holds
    /// what is left (none when everything was taken).
    Filled {
        paid: u64,
        bought: u64,
        next: Option<Box<PartialOffer>>,
    },
    /// The maker took the coin back, or a revocable CAT's issuer revoked it.
    ClawedBack,
}

impl PartialOffer {
    fn new(bundle: PartialOfferBundle) -> Result<Self> {
        check_info(bundle.partial_offer())?;
        Ok(Self { bundle })
    }

    /// The offer of a coin on the chain, without the maker's spends.
    pub fn from_coin(coin: SdkPartialOffer) -> Result<Self> {
        Self::new(PartialOfferBundle::from_coin(coin))
    }

    /// The maker's first offer, from the maker's own signed spends that create its coin. The
    /// SDK refuses spends that would make it unsafe to share (a coin made by a settlement coin,
    /// siblings sharing a payment nonce, wrong memos).
    pub fn from_maker_spend_bundle(
        spend_bundle: SpendBundle,
        info: PartialOfferInfo,
    ) -> Result<Self> {
        let mut ctx = SpendContext::new();
        Self::new(PartialOfferBundle::from_maker_spend_bundle(
            &mut ctx,
            spend_bundle,
            info,
        )?)
    }

    pub fn from_spend_bundle(spend_bundle: &SpendBundle) -> Result<Self> {
        let mut ctx = SpendContext::new();
        Self::new(PartialOfferBundle::from_spend_bundle(
            &mut ctx,
            spend_bundle,
        )?)
    }

    pub fn from_text(text: &str) -> Result<Self> {
        let text = text.trim();
        if !is_partial_offer(text) {
            return Err(PartialOfferError::NotPartial(
                "a partial offer starts with partial1".to_string(),
            ));
        }
        let spend_bundle = decode_partial_offer(text)
            .map_err(|error| PartialOfferError::NotPartial(error.to_string()))?;
        Self::from_spend_bundle(&spend_bundle)
    }

    pub fn to_spend_bundle(&self) -> Result<SpendBundle> {
        let mut ctx = SpendContext::new();
        Ok(self.bundle.clone().to_spend_bundle(&mut ctx)?)
    }

    pub fn to_text(&self) -> Result<String> {
        Ok(encode_partial_offer(&self.to_spend_bundle()?)?)
    }

    pub fn bundle(&self) -> &PartialOfferBundle {
        &self.bundle
    }

    /// The newest coin and its terms.
    pub fn offer(&self) -> &SdkPartialOffer {
        self.bundle.partial_offer()
    }

    pub fn coin(&self) -> Coin {
        self.offer().coin
    }

    pub fn info(&self) -> &PartialOfferInfo {
        &self.offer().info
    }

    /// The maker's spends that create the first coin.
    pub fn maker_spends(&self) -> &SpendBundle {
        self.bundle.spend_bundle()
    }

    /// Before the first fill: the coin is not on the chain yet, the maker's spends make it.
    pub fn is_first(&self) -> bool {
        !self.maker_spends().coin_spends.is_empty()
    }

    /// The coins the maker's spends spend (spending any one of them cancels a first offer).
    pub fn input_coin_ids(&self) -> Vec<Bytes32> {
        self.maker_spends()
            .coin_spends
            .iter()
            .map(|spend| spend.coin.coin_id())
            .collect()
    }

    /// The same coin without the maker's spends: once the coin is on the chain, a taker must
    /// not send them again.
    #[must_use]
    pub fn on_chain(&self) -> Self {
        Self {
            bundle: PartialOfferBundle::from_coin(*self.offer()),
        }
    }

    /// The maker's amounts, which fix the price. Klimper's offers (and Klimper 1.1/1.2's) keep
    /// them as the price; the SDK's own maker reduces the price by the greatest common divisor,
    /// so for such an offer the first coin's amount is the best known, and for a later coin of
    /// it the reduced price, scaled up to cover the coin.
    pub fn initial_amounts(&self) -> (u64, u64) {
        let price = self.info().price;
        let (offered, requested) = (price.price_precision.get(), price.precision.get());
        let amount = self.coin().amount;
        if offered >= amount {
            return (offered, requested);
        }
        let scale = amount.div_ceil(offered);
        (
            offered.saturating_mul(scale),
            requested.saturating_mul(scale),
        )
    }

    /// The most one fill can pay now: everything left, less any rounding dust only the maker
    /// can claw back. Zero if no fill is possible.
    pub fn max_fill_requested(&self) -> u64 {
        self.offer().max_requested_amount().unwrap_or(0)
    }

    /// The least of the requested asset one fill pays.
    pub fn min_fill_requested(&self) -> u64 {
        self.info().min_requested_amount.get()
    }

    /// What paying `requested` releases, rounded down as the puzzle does.
    pub fn quote(&self, requested: u64) -> u64 {
        self.info().offered_amount(requested).unwrap_or(0)
    }

    /// The least payment that releases as much as paying `requested` does: paying more than
    /// that buys nothing, since the puzzle rounds down.
    pub fn trim_payment(&self, requested: u64) -> u64 {
        let info = self.info();
        info.offered_amount(requested)
            .and_then(|offered| info.requested_amount(offered))
            .map_or(requested, |least| least.min(requested))
    }

    /// The least payment that buys `bought` of the offered asset.
    pub fn least_payment(&self, bought: u64) -> u64 {
        self.info().requested_amount(bought).unwrap_or(u64::MAX)
    }

    /// What `coin_spend`, a spend of this offer's coin seen on the chain, did.
    pub fn follow(&self, coin_spend: &CoinSpend) -> Result<PartialSpend> {
        let offer = self.offer();
        if coin_spend.coin != offer.coin {
            return Err(PartialOfferError::Invalid(
                "a spend of another coin".to_string(),
            ));
        }
        let mut ctx = SpendContext::new();
        let puzzle = ctx.alloc(&coin_spend.puzzle_reveal)?;
        let solution = ctx.alloc(&coin_spend.solution)?;
        let puzzle = Puzzle::parse(&ctx, puzzle);
        let requested = Some(offer.info.requested_asset);
        // `parse` tells a fill (and what it paid) from a claw back; `parse_child` alone would
        // read a fill that took everything as a claw back.
        let Some((_, paid)) =
            SdkPartialOffer::parse(&ctx, offer.coin, puzzle, solution, requested)?
        else {
            return Ok(PartialSpend::ClawedBack);
        };
        let bought = offer
            .info
            .offered_amount(paid)
            .ok_or_else(|| PartialOfferError::Invalid("a fill too large to read".to_string()))?;
        let next = SdkPartialOffer::parse_child(&ctx, offer.coin, puzzle, solution, requested)?
            .map(|child| Self::from_coin(child).map(Box::new))
            .transpose()?;
        Ok(PartialSpend::Filled { paid, bought, next })
    }
}

/// What a maker asks for: `offered_amount` of one asset for `requested_amount` of another, in
/// fills of at least `min_fill_requested`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Terms {
    pub offered: PartialOfferAsset,
    pub offered_amount: u64,
    pub requested: PartialOfferAsset,
    pub requested_amount: u64,
    pub min_fill_requested: u64,
    /// Unix seconds; no fill is valid at or after it.
    pub expiration: Option<u64>,
    /// The blockchain fee each fill must pay; `None` (never `Some(0)`, which would be another
    /// puzzle) for none.
    pub required_fee: Option<u64>,
    pub maker_puzzle_hash: Bytes32,
}

impl Terms {
    /// Refuses terms nobody could fill, or an asset for itself. The words are Klimper 1.2's,
    /// which websites may already show.
    pub fn check(&self) -> Result<()> {
        let invalid = |reason: &str| Err(PartialOfferError::Invalid(reason.to_string()));
        if self.offered == self.requested {
            return invalid("a partial offer of an asset for itself");
        }
        if self.offered_amount == 0 || self.requested_amount == 0 {
            return invalid("a partial offer needs both amounts");
        }
        if self.min_fill_requested == 0 || self.min_fill_requested > self.requested_amount {
            return invalid("the minimum fill is between 1 and the requested amount");
        }
        if self.info()?.offered_amount(self.min_fill_requested) == Some(0) {
            return invalid("the minimum fill buys nothing at this price");
        }
        Ok(())
    }

    /// The SDK's terms, with the price as the maker's own amounts (not reduced: readers show the
    /// price as the amounts the offer was made with).
    pub fn info(&self) -> Result<PartialOfferInfo> {
        let nonzero = |amount: u64, what: &str| {
            NonZeroU64::new(amount)
                .ok_or_else(|| PartialOfferError::Invalid(format!("{what} must be more than zero")))
        };
        let mut info = PartialOfferInfo::new(
            self.offered,
            self.requested,
            self.maker_puzzle_hash,
            PartialOfferPrice::new(
                nonzero(self.offered_amount, "the offered amount")?,
                nonzero(self.requested_amount, "the requested amount")?,
            ),
        )
        .with_min_requested_amount(nonzero(self.min_fill_requested, "the least fill")?);
        if let Some(expiration) = self.expiration {
            info = info.with_expiration(expiration);
        }
        if let Some(fee) = self.required_fee.filter(|fee| *fee > 0) {
            info = info.with_required_fee(fee);
        }
        Ok(info)
    }
}
