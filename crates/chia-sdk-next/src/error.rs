//! chia-wallet-sdk main's `DriverError` variants that the partial offer and auction drivers use
//! and 0.37.1 lacks; every other driver error is 0.37.1's, wrapped.

use chia_protocol::Bytes32;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DriverError {
    #[error(transparent)]
    Sdk(#[from] chia_wallet_sdk::driver::DriverError),

    #[error("missing required singleton asset info")]
    MissingAssetInfo,

    #[error("conflicting inputs in offers")]
    ConflictingOfferInputs,

    #[error("invalid partial offer terms")]
    InvalidPartialOffer,

    #[error(
        "partial offer fill is below the minimum, pays out nothing, or exceeds the remaining amount"
    )]
    InvalidPartialOfferFill,

    #[error("partial offer has unsupported persistent conditions")]
    UnsupportedPartialOfferConditions,

    #[error("partial offer coin is not created by the maker's spends")]
    PartialOfferNotCreated,

    #[error("partial offer coins with the same maker and requested asset can't share a parent")]
    PartialOfferSiblings,

    #[error("partial offer coin must not be created by a settlement payments coin")]
    UnsignedPartialOfferCreation,

    #[error("missing hint")]
    MissingHint,

    #[error("invalid memo")]
    InvalidMemo,

    #[error("invalid merkle proof")]
    InvalidMerkleProof,

    #[error("the auction's {name} of {bps} bps exceeds 10,000 bps")]
    AuctionBpsTooHigh { name: &'static str, bps: u64 },

    #[error("puzzle hash {0} receives more than one payment when the auction ends")]
    DuplicateAuctionPaymentPuzzleHash(Bytes32),

    #[error("puzzle hash {0} can't receive an auction payment, since it's reserved by the auction")]
    ReservedAuctionPaymentPuzzleHash(Bytes32),

    #[error("the launched singleton doesn't match the auction memo")]
    AuctionPuzzleHashMismatch,

    #[error("the auction singleton has an amount of {0}, but it must be 1")]
    InvalidAuctionSingletonAmount(u64),

    #[error("the auction reserve must be an empty coin locked by the auction")]
    InvalidAuctionReserve,

    #[error("the NFT must be locked by the auction before it's launched")]
    AuctionNftNotLocked,

    #[error("CAT spends can't be included with an auction that has an XCH reserve")]
    UnexpectedAuctionCatSpends,
}

impl From<clvm_traits::ToClvmError> for DriverError {
    fn from(error: clvm_traits::ToClvmError) -> Self {
        Self::Sdk(error.into())
    }
}

impl From<clvm_traits::FromClvmError> for DriverError {
    fn from(error: clvm_traits::FromClvmError) -> Self {
        Self::Sdk(error.into())
    }
}

impl From<clvmr::error::EvalErr> for DriverError {
    fn from(error: clvmr::error::EvalErr) -> Self {
        Self::Sdk(error.into())
    }
}
