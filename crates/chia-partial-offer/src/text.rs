// SPDX-License-Identifier: Apache-2.0
//! The `partial1…` text: bech32m with the prefix `partial`, compressed like an offer.

use chia_wallet_sdk::{
    chia::protocol::SpendBundle,
    driver::{compress_offer, decompress_offer},
    utils::Bech32,
};

use crate::{PartialOfferError, Result};

/// The bech32m prefix of a partial offer.
pub const PARTIAL_OFFER_PREFIX: &str = "partial";

/// Whether `text` looks like a partial offer (not whether it is a valid one).
pub fn is_partial_offer(text: &str) -> bool {
    text.trim().to_ascii_lowercase().starts_with("partial1")
}

pub fn encode_partial_offer(spend_bundle: &SpendBundle) -> Result<String> {
    let data = compress_offer(spend_bundle)?;
    Bech32::new(data.into(), PARTIAL_OFFER_PREFIX.to_string())
        .encode()
        .map_err(|error| PartialOfferError::Invalid(error.to_string()))
}

pub fn decode_partial_offer(text: &str) -> Result<SpendBundle> {
    let data = Bech32::decode(text.trim())
        .and_then(|bech32| bech32.expect_prefix(PARTIAL_OFFER_PREFIX))
        .map_err(|error| PartialOfferError::NotPartial(error.to_string()))?;
    Ok(decompress_offer(&data)?)
}
