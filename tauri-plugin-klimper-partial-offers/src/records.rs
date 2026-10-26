// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the commands take and answer. Field names follow nodexch's `partial` fields, so the UI
//! reads a partial offer from the wallet and from nodexch the same way. Amounts are mojos, as
//! strings: an XCH amount can exceed what a JavaScript number holds exactly.

use chia_partial_offer::{PartialAsset, PartialOffer};
use chia_wallet_sdk::{chia::protocol::Bytes32, utils::Address};
use serde::{Deserialize, Serialize};

/// One side's asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Side {
    /// The CAT's asset id, hex without `0x`; `None` for XCH.
    pub asset_id: Option<String>,
    /// A revocable CAT's hidden puzzle hash (its issuer can take coins back).
    pub hidden_puzzle_hash: Option<String>,
    pub revocable: bool,
}

impl From<PartialAsset> for Side {
    fn from(asset: PartialAsset) -> Self {
        Self {
            asset_id: asset.asset_id.map(hex::encode),
            hidden_puzzle_hash: asset.hidden_puzzle_hash.map(hex::encode),
            revocable: asset.is_revocable(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Nothing taken yet.
    Open,
    /// Taken in part; the rest is still on offer.
    PartlyFilled,
    /// Everything taken.
    Filled,
    /// The maker took the rest back (or spent the coins before the first fill).
    Cancelled,
    /// Past its expiry: no fill is valid any more, and the maker can take the rest back.
    Expired,
    /// No connection to the network: the status could not be checked.
    Unknown,
    /// This wallet sent a cancel that the chain has not confirmed yet. Only the wallet's own
    /// list says this (it knows the cancel was sent); following the chain never does.
    Cancelling,
}

impl Status {
    pub fn is_live(self) -> bool {
        matches!(self, Self::Open | Self::PartlyFilled | Self::Unknown)
    }
}

/// One fill, as followed on the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fill {
    /// The partial coin the fill spent, `0x` hex.
    pub coin_id: String,
    /// What the taker received of the offered asset.
    pub bought: String,
    /// What the taker paid of the requested asset (what `bought` costs at the price).
    pub paid: String,
}

/// A partial offer as the screens show it: its terms, what is left, and its status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialOfferView {
    /// The newest `partial1…` text: the one to take or share.
    pub offer: String,
    pub offered: Side,
    pub requested: Side,
    /// The maker's initial amounts, which fix the price.
    pub initial_offered: String,
    pub initial_requested: String,
    /// What is left, and the most one fill can pay for it.
    pub remaining_offered: String,
    pub remaining_requested: String,
    pub min_fill_requested: String,
    pub min_fill_offered: String,
    /// The blockchain fee each fill must pay, apart from the network fee.
    pub required_fee: Option<String>,
    /// Unix seconds; no fill is valid at or after it.
    pub expires_at: Option<u64>,
    pub maker_address: String,
    /// The coin that holds what is left, `0x` hex.
    pub coin_id: String,
    /// Before the first fill.
    pub first: bool,
    pub status: Status,
    /// The fills seen while following the offer to its newest coin (only those after the text
    /// that was read).
    pub fills: Vec<Fill>,
    /// The maker is this wallet.
    pub is_mine: bool,
}

impl PartialOfferView {
    pub fn new(
        offer: &PartialOffer,
        status: Status,
        fills: Vec<Fill>,
        is_mine: bool,
        prefix: &str,
    ) -> Result<Self, crate::Error> {
        let info = &offer.info;
        // A filled offer has nothing left; a cancelled one shows what was left when the maker
        // took it back, so it does not read as sold.
        let remaining = if status == Status::Filled {
            0
        } else {
            offer.coin.amount
        };
        Ok(Self {
            offer: offer.to_text()?,
            offered: info.offered_asset_info.into(),
            requested: info.requested_asset_info.into(),
            initial_offered: info.initial_offered().to_string(),
            initial_requested: info.initial_requested().to_string(),
            remaining_offered: remaining.to_string(),
            remaining_requested: info.max_fill_requested(remaining).to_string(),
            min_fill_requested: info.min_fill_requested().to_string(),
            min_fill_offered: info.price_data.quote(info.min_fill_requested()).to_string(),
            required_fee: info.required_fee.map(|fee| fee.to_string()),
            expires_at: info.expiration,
            maker_address: Address::new(info.maker_puzzle_hash, prefix.to_string())
                .encode()
                .map_err(|error| crate::Error::Invalid(error.to_string()))?,
            coin_id: hex0x(offer.coin.coin_id()),
            first: offer.is_first(),
            status,
            fills,
            is_mine,
        })
    }
}

/// A partial offer this wallet made, as `get_partial_offers` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyPartialOffer {
    /// The first partial coin's id, `0x` hex: stays the same across fills.
    pub id: String,
    /// Unix seconds.
    pub created_at: u64,
    /// Every fill since it was made.
    pub fills: Vec<Fill>,
    /// The cancel this wallet sent, while the chain has not confirmed it.
    pub cancel: Option<PendingCancel>,
    pub view: PartialOfferView,
}

/// A cancel sent and not confirmed yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCancel {
    pub transaction_id: String,
    /// The partial coin the cancel spends, `0x` hex. Once the offer has moved past it (a fill
    /// confirmed first), the cancel failed.
    pub coin_id: String,
    /// Unix seconds.
    pub sent_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeResult {
    /// The transaction's id (the spend bundle's name), `0x` hex.
    pub transaction_id: String,
    pub bought: String,
    pub paid: String,
    /// The next partial offer, with what this fill left (none if it took everything).
    pub next_offer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MakeRequest {
    /// The offered CAT's asset id (hex); `None` for XCH.
    pub offered_asset_id: Option<String>,
    pub offered_amount: String,
    /// The requested CAT's asset id (hex); `None` for XCH.
    pub requested_asset_id: Option<String>,
    pub requested_amount: String,
    /// The least of the requested asset one fill pays.
    pub min_fill_requested: String,
    /// The blockchain fee each fill must pay; none or "0" for no requirement.
    #[serde(default)]
    pub required_fee: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    pub expires_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MakeResult {
    pub id: String,
    pub offer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelResult {
    pub transaction_id: String,
}

pub fn hex0x(bytes: Bytes32) -> String {
    format!("0x{}", hex::encode(bytes))
}

pub fn parse_amount(text: &str) -> Result<u64, crate::Error> {
    text.trim()
        .parse()
        .map_err(|_| crate::Error::Invalid(format!("not an amount in mojos: {text}")))
}

pub fn parse_hash(text: &str) -> Result<Bytes32, crate::Error> {
    let bytes = hex::decode(text.trim().trim_start_matches("0x"))
        .map_err(|_| crate::Error::Invalid(format!("not a hash: {text}")))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| crate::Error::Invalid(format!("not 32 bytes: {text}")))?;
    Ok(Bytes32::new(bytes))
}
