//! chia-wallet-sdk's partial offer driver (CHIP-0052), copied from its unreleased main branch
//! onto chia-wallet-sdk 0.37.1. See README.md; delete once Sage's SDK release has it.

mod error;
mod partial_offer;
pub(crate) mod sdk;
mod text;

pub use error::*;
pub use partial_offer::*;
pub use text::*;

/// chia-sdk-types' test macro, which the copied puzzle test uses.
#[cfg(test)]
macro_rules! assert_puzzle_hash {
    ($puzzle:ident => $puzzle_hash:ident) => {
        let mut a = clvmr::Allocator::new();
        let ptr = clvmr::serde::node_from_bytes(&mut a, &$puzzle)?;
        let hash = clvm_utils::tree_hash(&mut a, ptr);
        assert_eq!($puzzle_hash, hash);
    };
}
#[cfg(test)]
pub(crate) use assert_puzzle_hash;
