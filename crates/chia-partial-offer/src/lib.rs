// SPDX-License-Identifier: Apache-2.0
//! Partial offers (CHIP-0052): an offer of one asset for another at a fixed price that takers
//! can fill in parts.
//!
//! The text is bech32m with the prefix `partial` (`partial1…`), compressed like an offer. The
//! spend bundle in it holds the maker's own spends (only before the first fill: they create the
//! first partial coin) and exactly one placeholder spend, of a coin with the puzzle hash
//! `0101…01`, whose puzzle is the hint ([`PartialHint`]) and whose parent and amount are the
//! partial coin's.
//!
//! The partial coin is XCH, a CAT or a revocable CAT. Inside the asset wrapper sits `P2_1_OF_N` over
//! `[partial puzzle, maker]`, so the maker can cancel at any time by spending the latest coin
//! ([`PartialOffer::claw_back`]). A fill pays `x ≥ min` of the requested asset to the maker and
//! receives `x * initial_offered / initial_requested` of the offered one; a child coin with the
//! same puzzle hash keeps the rest ([`PartialOffer::child`]).
//!
//! This crate depends on chia-wallet-sdk alone, so it can move into the SDK (or Sage) as it is.
//! Ported from the reference implementation, github.com/Yakuhito/partial (MIT, see NOTICE.md).

mod hint;
mod info;
mod make;
mod offer;
mod puzzle;
mod text;

pub use hint::*;
pub use info::*;
pub use make::*;
pub use offer::*;
pub use puzzle::*;
pub use text::*;

use chia_wallet_sdk::driver::DriverError;

/// Why a partial offer could not be read, made, filled or cancelled.
#[derive(Debug, thiserror::Error)]
pub enum PartialOfferError {
    #[error("not a partial offer: {0}")]
    NotPartial(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Driver(#[from] DriverError),
}

pub type Result<T, E = PartialOfferError> = std::result::Result<T, E>;
