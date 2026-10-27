// SPDX-License-Identifier: Apache-2.0
//! The hint: the placeholder spend's puzzle, which carries the terms.

use chia_wallet_sdk::chia::{protocol::Bytes32, puzzle_types::LineageProof};
use clvm_traits::{FromClvm, ToClvm};

use crate::PartialPriceData;

/// The placeholder coin's puzzle hash: the spend that carries the hint.
pub const PARTIAL_HINT_PUZZLE_HASH: Bytes32 = Bytes32::new([1; 32]);

/// One side's asset: XCH (no asset id), a CAT, or a revocable CAT (with its hidden puzzle hash).
/// A list, so later versions can add fields.
#[derive(FromClvm, ToClvm, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[clvm(list)]
pub struct PartialAsset {
    pub asset_id: Option<Bytes32>,
    pub hidden_puzzle_hash: Option<Bytes32>,
}

impl PartialAsset {
    pub fn xch() -> Self {
        Self {
            asset_id: None,
            hidden_puzzle_hash: None,
        }
    }

    pub fn cat(asset_id: Bytes32, hidden_puzzle_hash: Option<Bytes32>) -> Self {
        Self {
            asset_id: Some(asset_id),
            hidden_puzzle_hash,
        }
    }

    pub fn is_xch(&self) -> bool {
        self.asset_id.is_none()
    }

    /// A revocable CAT: its issuer can take coins back.
    pub fn is_revocable(&self) -> bool {
        self.asset_id.is_some() && self.hidden_puzzle_hash.is_some()
    }
}

/// The partial hint. The partial coin's parent and amount are the placeholder coin's.
#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct PartialHint<IP> {
    pub lineage_proof: Option<LineageProof>,
    pub offered_asset_info: PartialAsset,
    pub requested_asset_info: PartialAsset,
    pub price_data: PartialPriceData,
    pub maker_puzzle_hash: Bytes32,
    /// `(q . conditions)`: what every fill must also satisfy (expiry, a minimum fee).
    pub inner_puzzle: IP,
    pub min_other_asset_amount_minus_one: u64,
}
