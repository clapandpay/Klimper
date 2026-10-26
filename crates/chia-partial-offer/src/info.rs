// SPDX-License-Identifier: Apache-2.0
//! The terms of a partial offer and the puzzle hashes they fix.

use chia_wallet_sdk::{
    chia::{
        protocol::Bytes32,
        puzzle_types::{LineageProof, cat::CatArgs},
    },
    driver::{CatMaker, DriverError, SpendContext},
    puzzles::SETTLEMENT_PAYMENT_HASH,
    types::{
        Condition, Conditions, MerkleTree, Mod,
        puzzles::{P2OneOfManyArgs, RevocationArgs},
    },
};
use clvm_traits::{FromClvm, ToClvm, clvm_quote};
use clvm_utils::{ToTreeHash, TreeHash, tree_hash};
use clvmr::{Allocator, NodePtr};

use crate::{
    PartialAsset, PartialHint, PartialOfferError, PartialPriceData, PartialPuzzleArgs, Result,
    puzzle::mul_div,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialOfferInfo {
    /// The partial coin's lineage, for a CAT (`None` for XCH).
    pub lineage_proof: Option<LineageProof>,
    pub offered_asset_info: PartialAsset,
    pub requested_asset_info: PartialAsset,
    /// Where the payments go, and who may cancel.
    pub maker_puzzle_hash: Bytes32,
    /// Unix seconds; no fill is valid at or after it.
    pub expiration: Option<u64>,
    /// The blockchain fee each fill must pay (`RESERVE_FEE`).
    pub required_fee: Option<u64>,
    pub price_data: PartialPriceData,
    pub min_other_asset_amount_minus_one: u64,
}

impl PartialOfferInfo {
    /// The maker's initial offered amount.
    pub fn initial_offered(&self) -> u64 {
        self.price_data.price_precision
    }

    /// The maker's initial requested amount.
    pub fn initial_requested(&self) -> u64 {
        self.price_data.precision
    }

    /// The least of the requested asset one fill pays.
    pub fn min_fill_requested(&self) -> u64 {
        self.min_other_asset_amount_minus_one.saturating_add(1)
    }

    /// The most of the requested asset one fill of a coin of `remaining_offered` can pay: what
    /// it pays out must not exceed the coin (the puzzle rounds down, so up to one price step
    /// of dust can stay behind, which the maker can claw back).
    pub fn max_fill_requested(&self, remaining_offered: u64) -> u64 {
        mul_div(
            remaining_offered,
            self.price_data.precision,
            self.price_data.price_precision,
        )
    }

    /// The least payment that receives as much as paying `requested` does: paying more than
    /// that buys nothing, because the puzzle rounds down.
    pub fn trim_payment(&self, requested: u64) -> u64 {
        let receives = self.price_data.quote(requested);
        let mut least = self.price_data.reverse_quote(receives);
        if self.price_data.quote(least) < receives {
            least += 1;
        }
        least.min(requested)
    }

    #[must_use]
    pub fn with_lineage_proof(self, lineage_proof: Option<LineageProof>) -> Self {
        Self {
            lineage_proof,
            ..self
        }
    }

    pub fn cat_maker(asset: PartialAsset) -> CatMaker {
        match (asset.asset_id, asset.hidden_puzzle_hash) {
            (None, _) => CatMaker::Xch,
            (Some(asset_id), None) => CatMaker::Default {
                tail_hash_hash: asset_id.tree_hash(),
            },
            (Some(asset_id), Some(hidden)) => CatMaker::Revocable {
                tail_hash_hash: asset_id.tree_hash(),
                hidden_puzzle_hash_hash: hidden.tree_hash(),
            },
        }
    }

    /// `inner_puzzle_hash` wrapped as the asset's coins are: bare for XCH, in the CAT layer for
    /// a CAT, and in the revocation layer under it for a revocable CAT.
    pub fn full_asset_puzzle_hash(asset: PartialAsset, inner_puzzle_hash: Bytes32) -> Bytes32 {
        match (asset.asset_id, asset.hidden_puzzle_hash) {
            (None, _) => inner_puzzle_hash,
            (Some(asset_id), None) => {
                CatArgs::curry_tree_hash(asset_id, inner_puzzle_hash.into()).into()
            }
            (Some(asset_id), Some(hidden)) => CatArgs::curry_tree_hash(
                asset_id,
                RevocationArgs::new(hidden, inner_puzzle_hash).curry_tree_hash(),
            )
            .into(),
        }
    }

    /// The asset wrapper around `inner_puzzle`.
    pub fn full_puzzle(
        ctx: &mut SpendContext,
        asset: PartialAsset,
        inner_puzzle: NodePtr,
    ) -> Result<NodePtr, DriverError> {
        match (asset.asset_id, asset.hidden_puzzle_hash) {
            (None, _) => Ok(inner_puzzle),
            (Some(asset_id), None) => ctx.curry(CatArgs::new(asset_id, inner_puzzle)),
            (Some(asset_id), Some(hidden)) => {
                let inner_puzzle_hash = ctx.tree_hash(inner_puzzle);
                let revocable = ctx.curry(RevocationArgs::new(hidden, inner_puzzle_hash.into()))?;
                ctx.curry(CatArgs::new(asset_id, revocable))
            }
        }
    }

    /// The conditions every fill outputs.
    pub fn inner_conditions(&self) -> Conditions {
        let mut conditions = Conditions::new();
        if let Some(expiration) = self.expiration {
            conditions = conditions.assert_before_seconds_absolute(expiration);
        }
        if let Some(required_fee) = self.required_fee {
            conditions = conditions.reserve_fee(required_fee);
        }
        conditions
    }

    pub fn quoted_inner_conditions(&self) -> (u8, Conditions) {
        clvm_quote!(self.inner_conditions())
    }

    fn inner_conditions_hash(&self) -> TreeHash {
        let mut allocator = Allocator::new();
        let ptr = self
            .quoted_inner_conditions()
            .to_clvm(&mut allocator)
            .expect("conditions allocate");
        tree_hash(&allocator, ptr)
    }

    pub fn to_args(
        &self,
        ctx: &mut SpendContext,
    ) -> Result<PartialPuzzleArgs<NodePtr, (u8, Conditions)>, DriverError> {
        Ok(PartialPuzzleArgs {
            cat_maker: Self::cat_maker(self.offered_asset_info).get_puzzle(ctx)?,
            other_asset_offer_mod: Self::full_asset_puzzle_hash(
                self.requested_asset_info,
                SETTLEMENT_PAYMENT_HASH.into(),
            ),
            receiver_puzzle_hash: self.maker_puzzle_hash,
            inner_puzzle: self.quoted_inner_conditions(),
            min_other_asset_amount_minus_one: self.min_other_asset_amount_minus_one,
            price_data: self.price_data,
        })
    }

    pub fn partial_puzzle_hash(&self) -> TreeHash {
        PartialPuzzleArgs {
            cat_maker: Self::cat_maker(self.offered_asset_info).curry_tree_hash(),
            other_asset_offer_mod: Self::full_asset_puzzle_hash(
                self.requested_asset_info,
                SETTLEMENT_PAYMENT_HASH.into(),
            ),
            receiver_puzzle_hash: self.maker_puzzle_hash,
            inner_puzzle: self.inner_conditions_hash(),
            min_other_asset_amount_minus_one: self.min_other_asset_amount_minus_one,
            price_data: self.price_data,
        }
        .curry_tree_hash()
    }

    pub fn merkle_tree(&self) -> MerkleTree {
        MerkleTree::new(&[self.partial_puzzle_hash().into(), self.maker_puzzle_hash])
    }

    /// The partial coin's inner puzzle hash: 1-of-2 between the partial puzzle and the maker.
    pub fn inner_puzzle_hash(&self) -> TreeHash {
        P2OneOfManyArgs::new(self.merkle_tree().root()).curry_tree_hash()
    }

    /// The inner puzzle hash under the CAT layer: with the revocation layer for a revocable CAT.
    pub fn cat_inner_puzzle_hash(&self) -> Bytes32 {
        match self.offered_asset_info.hidden_puzzle_hash {
            Some(hidden) if self.offered_asset_info.asset_id.is_some() => {
                RevocationArgs::new(hidden, self.inner_puzzle_hash().into())
                    .curry_tree_hash()
                    .into()
            }
            _ => self.inner_puzzle_hash().into(),
        }
    }

    /// The partial coin's puzzle hash.
    pub fn puzzle_hash(&self) -> Bytes32 {
        Self::full_asset_puzzle_hash(self.offered_asset_info, self.inner_puzzle_hash().into())
    }

    /// The puzzle hash a fill pays the offered asset to: a child of the partial coin with it
    /// means a taker filled it.
    pub fn settlement_puzzle_hash(&self) -> Bytes32 {
        Self::full_asset_puzzle_hash(self.offered_asset_info, SETTLEMENT_PAYMENT_HASH.into())
    }

    pub fn to_hint(&self) -> PartialHint<(u8, Conditions)> {
        PartialHint {
            lineage_proof: self.lineage_proof,
            offered_asset_info: self.offered_asset_info,
            requested_asset_info: self.requested_asset_info,
            price_data: self.price_data,
            maker_puzzle_hash: self.maker_puzzle_hash,
            inner_puzzle: self.quoted_inner_conditions(),
            min_other_asset_amount_minus_one: self.min_other_asset_amount_minus_one,
        }
    }

    /// Reads the terms from a hint. Refuses a hint whose conditions are anything but an expiry
    /// and a required fee, in that order: the puzzle hash is derived from them, so anything else
    /// would point at a coin that is not there.
    pub fn from_hint(allocator: &Allocator, hint: NodePtr) -> Result<Self> {
        let hint = PartialHint::<NodePtr>::from_clvm(allocator, hint)
            .map_err(|error| PartialOfferError::Invalid(format!("partial hint: {error}")))?;
        let price = hint.price_data;
        if price.price_precision == 0 || price.precision == 0 {
            return Err(PartialOfferError::Invalid(
                "a partial offer without a price".to_string(),
            ));
        }
        if hint.offered_asset_info == hint.requested_asset_info {
            return Err(PartialOfferError::Invalid(
                "a partial offer of an asset for itself".to_string(),
            ));
        }
        let (quote, conditions) =
            <(u8, Vec<Condition<NodePtr>>)>::from_clvm(allocator, hint.inner_puzzle).map_err(
                |error| PartialOfferError::Invalid(format!("partial conditions: {error}")),
            )?;
        if quote != 1 {
            return Err(PartialOfferError::Invalid(
                "partial conditions are not quoted".to_string(),
            ));
        }
        let info = Self {
            lineage_proof: hint.lineage_proof,
            offered_asset_info: hint.offered_asset_info,
            requested_asset_info: hint.requested_asset_info,
            maker_puzzle_hash: hint.maker_puzzle_hash,
            expiration: conditions
                .iter()
                .find_map(Condition::as_assert_before_seconds_absolute)
                .map(|condition| condition.seconds),
            required_fee: conditions
                .iter()
                .find_map(Condition::as_reserve_fee)
                .map(|condition| condition.amount),
            price_data: price,
            min_other_asset_amount_minus_one: hint.min_other_asset_amount_minus_one,
        };
        if info.inner_conditions_hash() != tree_hash(allocator, hint.inner_puzzle) {
            return Err(PartialOfferError::Invalid(
                "partial conditions other than an expiry and a fee".to_string(),
            ));
        }
        if info.offered_asset_info.asset_id.is_some() != info.lineage_proof.is_some() {
            return Err(PartialOfferError::Invalid(
                "a lineage proof for XCH, or none for a CAT".to_string(),
            ));
        }
        Ok(info)
    }
}
