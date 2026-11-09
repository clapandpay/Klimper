// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the commands take and answer. Amounts are mojos as strings (an XCH amount can exceed
//! what a JavaScript number holds exactly); times are Unix seconds.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuctionStatus {
    /// Taking bids.
    Live,
    /// No more bids are valid; anyone can end it to pay out and hand over the NFT.
    Ending,
    /// Ended: the NFT went to the winning bidder and the bid to the seller.
    Sold,
    /// Ended without a bid: the NFT went back to the seller.
    Unsold,
    /// The seller took the NFT back before any bid.
    Cancelled,
    /// No connection to the network: the state could not be followed.
    Unknown,
}

/// How much each bid must beat the one before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Increment {
    /// At least this many mojos more.
    Flat { amount: String },
    /// At least this many basis points (1/100 of a percent) more.
    Percent { bps: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bid {
    pub amount: String,
    pub address: String,
    pub is_mine: bool,
}

/// An auction as the screens show it, read and checked from the chain. The flags are what the
/// screens decide on, one each.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuctionView {
    /// The auction's launcher id, `0x` hex: what a link names.
    pub id: String,
    /// The NFT for sale: its launcher id, `0x` hex, always shown (names can be copied).
    pub nft_id: String,
    /// The NFT's id as an address (`nft1…`).
    pub nft_address: String,
    /// The NFT's coin while the auction holds it, `0x` hex.
    pub nft_coin_id: String,
    /// The CAT bids are in, hex; `None` for XCH. Klimper bids in XCH only.
    pub bid_asset_id: Option<String>,
    pub minimum_bid: String,
    pub increment: Increment,
    pub end_time: u64,
    /// After the end time, a bid may still come in within this many seconds of the last one.
    pub grace_period: u64,
    /// Basis points the winner pays on top of the bid, and to whom.
    pub buyers_premium_bps: u64,
    pub buyers_premium_address: String,
    /// Basis points taken out of the winning bid, and to whom.
    pub commission_bps: u64,
    pub commission_address: String,
    /// The NFT's royalty, paid by the winner on top of the bid, from the NFT on the chain.
    pub royalty_bps: u16,
    pub royalty_address: String,
    /// Where the winning bid goes (and the NFT, if nobody bids).
    pub seller_address: String,
    pub winning_bid: Option<Bid>,
    /// The least the next bid can be.
    pub next_minimum_bid: String,
    /// What bidding `next_minimum_bid` locks up: the bid, the buyer's premium and the royalty.
    pub next_minimum_cost: String,
    /// Unix seconds of the auction's last spend (its launch or the last bid), if known.
    pub last_spend_time: Option<u64>,
    /// Whether a bid now must be a grace-period bid (after the end time).
    pub in_grace: bool,
    pub status: AuctionStatus,
    /// Whether this wallet can bid now, and if not, why.
    pub can_bid: bool,
    pub cannot_bid_reason: Option<String>,
    /// Whether anyone (this wallet too) can end it now.
    pub can_end: bool,
    /// The seller is this wallet.
    pub is_mine: bool,
    /// The seller can cancel: its own, and no bid yet.
    pub can_cancel: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartRequest {
    /// The NFT's launcher id (hex or `nft1…`).
    pub nft_id: String,
    pub minimum_bid: String,
    pub increment: Increment,
    pub end_time: u64,
    #[serde(default)]
    pub grace_period: u64,
    /// The network fee, in mojos.
    #[serde(default)]
    pub fee: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sent {
    /// The auction's id, `0x` hex.
    pub id: String,
    pub transaction_id: String,
}

/// An auction this wallet started or bid on, as `get_auctions` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MyAuction {
    pub id: String,
    /// `started` or `bid`.
    pub role: String,
    pub created_at: u64,
    /// The last view, or `None` if it could not be read.
    pub view: Option<AuctionView>,
    pub error: Option<String>,
}
