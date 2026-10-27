// SPDX-License-Identifier: Apache-2.0
//! The partial puzzle (`puzzles/partial.rue` of the reference), its curried arguments and its
//! solution.

use std::borrow::Cow;

use chia_wallet_sdk::{
    chia::{protocol::Bytes32, puzzle_types::CoinProof},
    types::Mod,
};
use clvm_traits::{FromClvm, ToClvm};
use clvm_utils::TreeHash;
use hex_literal::hex;

pub const PARTIAL_PUZZLE: [u8; 429] = hex!(
    "
    ff02ffff01ff02ffff01ff04ffff04ffff013fffff04ffff0bff17ffff02ff05
    ffff04ff05ffff04ff8204ffffff04ffff04ff2fffff04ffff02ffff03ffff15
    ff8205ffff81bf80ffff018205ffffff01ff088080ff0180ffff04ffff04ff2f
    ff8080ff80808080ff808080808080ff808080ffff04ffff04ffff0146ffff04
    ffff30ff8204ffffff02ff0bffff04ff820affff8217ff8080ff8216ff80ff80
    8080ffff04ffff03ffff15ff04ff8080ffff04ffff0133ffff04ff820affffff
    04ff04ffff04ffff04ff820affff8080ff8080808080ffff04ffff0101ff8080
    80ffff03ffff02ffff03ff820bffffff01ff02ffff03ffff15ff822bffffff01
    81ff80ffff01ff0101ffff018080ff0180ffff018080ff0180ffff04ffff04ff
    ff0133ff820bff80ff0680ff0680808080ffff04ffff04ffff11ff820b7fffff
    13ffff12ff8202ffff82013f80ff8201bf8080ffff02ff2fff820fff8080ff01
    8080ffff04ffff01ff02ffff03ffff07ff0380ffff01ff0bffff0102ffff02ff
    02ffff04ff02ff058080ffff02ff02ffff04ff02ff07808080ffff01ff0bffff
    0101ff038080ff0180ff018080
    "
);

pub const PARTIAL_PUZZLE_HASH: TreeHash = TreeHash::new(hex!(
    "e8b38a9da89c523f3a1e4b2679d893e1f565d800ecd4e37b25ef644ae84fe868"
));

/// The price: `price_precision` offered mojos for `precision` requested mojos, the maker's
/// initial amounts.
#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(list)]
pub struct PartialPriceData {
    pub price_precision: u64,
    #[clvm(rest)]
    pub precision: u64,
}

impl PartialPriceData {
    /// How much of the offered asset a fill paying `requested` mojos receives (rounded down, as
    /// the puzzle computes it).
    pub fn quote(&self, requested: u64) -> u64 {
        mul_div(requested, self.price_precision, self.precision)
    }

    /// How much of the requested asset taking `offered` mojos costs (rounded down).
    pub fn reverse_quote(&self, offered: u64) -> u64 {
        mul_div(offered, self.precision, self.price_precision)
    }
}

/// `a * b / c`, rounding down, without overflowing (the puzzle works on big integers).
pub(crate) fn mul_div(a: u64, b: u64, c: u64) -> u64 {
    if c == 0 {
        return 0;
    }
    u64::try_from(u128::from(a) * u128::from(b) / u128::from(c)).unwrap_or(u64::MAX)
}

#[derive(ToClvm, FromClvm, Debug, Clone, Copy, PartialEq, Eq)]
#[clvm(curry)]
pub struct PartialPuzzleArgs<CM, IP> {
    pub cat_maker: CM,
    pub other_asset_offer_mod: Bytes32,
    pub receiver_puzzle_hash: Bytes32,
    pub inner_puzzle: IP,
    pub min_other_asset_amount_minus_one: u64,
    pub price_data: PartialPriceData,
}

impl<CM, IP> Mod for PartialPuzzleArgs<CM, IP> {
    fn mod_reveal() -> Cow<'static, [u8]> {
        Cow::Borrowed(&PARTIAL_PUZZLE)
    }

    fn mod_hash() -> TreeHash {
        PARTIAL_PUZZLE_HASH
    }
}

#[derive(FromClvm, ToClvm, Debug, Clone, PartialEq, Eq)]
#[clvm(list)]
pub struct PartialSolution<CCR, CMS, IPS> {
    pub my_data: CoinProof,
    pub other_asset_amount: u64,
    pub create_coin_rest: Option<CCR>,
    pub cat_maker_solution: CMS,
    #[clvm(rest)]
    pub inner_puzzle_solution: IPS,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clvm_utils::tree_hash;
    use clvmr::{Allocator, serde::node_from_bytes};

    #[test]
    fn the_partial_puzzle_hashes_to_its_hash() {
        let mut a = Allocator::new();
        let ptr = node_from_bytes(&mut a, &PARTIAL_PUZZLE).unwrap();
        assert_eq!(tree_hash(&a, ptr), PARTIAL_PUZZLE_HASH);
    }

    #[test]
    fn quotes_round_down_and_do_not_overflow() {
        let price = PartialPriceData {
            price_precision: 1_000,
            precision: 5_000_000_000_000,
        };
        assert_eq!(price.quote(100_000_000_000), 20);
        assert_eq!(price.quote(4_999_999_999), 0);
        assert_eq!(price.reverse_quote(20), 100_000_000_000);
        let big = PartialPriceData {
            price_precision: u64::MAX,
            precision: u64::MAX,
        };
        assert_eq!(big.quote(u64::MAX), u64::MAX);
    }
}
