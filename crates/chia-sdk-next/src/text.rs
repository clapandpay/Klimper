//! `encode_partial_offer` / `decode_partial_offer`, from chia-wallet-sdk main's
//! `crates/chia-sdk-driver/src/offers/compress.rs`: compressed like an offer, bech32m with the
//! prefix `partial`.

use chia_protocol::SpendBundle;
use chia_wallet_sdk::{
    driver::{compress_offer, decompress_offer},
    utils::Bech32,
};

use crate::DriverError;

pub fn encode_partial_offer(spend_bundle: &SpendBundle) -> Result<String, DriverError> {
    Ok(
        Bech32::new(compress_offer(spend_bundle)?.into(), "partial".to_string())
            .encode()
            .map_err(chia_wallet_sdk::driver::DriverError::from)?,
    )
}

pub fn decode_partial_offer(text: &str) -> Result<SpendBundle, DriverError> {
    Ok(decompress_offer(
        &Bech32::decode(text)
            .map_err(chia_wallet_sdk::driver::DriverError::from)?
            .expect_prefix("partial")
            .map_err(chia_wallet_sdk::driver::DriverError::from)?,
    )?)
}
