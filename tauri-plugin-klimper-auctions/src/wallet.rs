// SPDX-License-Identifier: AGPL-3.0-or-later
//! NFT auctions on a Sage wallet, on chia-wallet-sdk's auction driver (`chia-sdk-next`): read
//! one from the chain and check it, start one, bid in XCH, end it, cancel the wallet's own.
//!
//! The driver does the puzzles. What is here is what a wallet must add (the challenger review
//! of 2026-11-07, `docs/auctions.md`):
//! - An auction is only biddable when the NFT it names is on the chain, unspent, a standard NFT,
//!   and locked by this auction: otherwise it can never end, and the winning bid stays locked.
//!   The royalty comes from that NFT, not from the launch.
//! - Timings within bounds (an end time months away, or a long grace period, would lock a
//!   winning bid as long), and the grace rule as the puzzles have it.
//! - Every spend of the wallet is tied to the auction spend it pays for (the bid's announcement,
//!   the end's or cancel's coin), so no part can be sent alone.

use std::num::NonZeroU64;

use chia_sdk_next::{
    AUCTION_MAX_BPS, Auction, AuctionBid, AuctionBidVerifier, AuctionBpsPayment, AuctionExt,
    AuctionLaunch, AuctionPayments, AuctionReserve, AuctionSettings, AuctionTimings,
    LauncherAuctionExt, auction_lock_p2_puzzle_hash, calculate_auction_payment,
};
use chia_wallet_sdk::clvm_traits::FromClvm;
use chia_wallet_sdk::{
    chia::{
        bls::Signature,
        protocol::{Bytes32, Coin, CoinSpend, SpendBundle},
        puzzle_types::{LineageProof, Memos, singleton::SingletonSolution, standard::StandardArgs},
    },
    clvm_traits::clvm_list,
    clvm_utils::tree_hash,
    clvmr::NodePtr,
    driver::{
        Action, Cat, FeeAction, Id, Launcher, Nft, Puzzle, RoyaltyInfo, SpendContext,
        SpendWithConditions, Spends, StandardLayer,
    },
    types::{Condition, Conditions, puzzles::RawActionLayerSolution, run_puzzle},
    utils::Address,
};
use sage_database::DeserializePrimitive;
use tauri_plugin_klimper_partial_offers::wallet::WalletCtx;

use crate::{
    Error,
    records::{AuctionStatus, AuctionView, Bid, Increment, StartRequest},
};

/// The furthest ahead an end time may be, in seconds (90 days).
pub const MAX_DURATION: u64 = 90 * 24 * 60 * 60;
/// The longest grace period, in seconds (one day).
pub const MAX_GRACE: u64 = 24 * 60 * 60;
/// The soonest an auction may end after it starts, in seconds (10 minutes).
pub const MIN_DURATION: u64 = 10 * 60;
/// Block timestamps lag the clock: a bid is not offered this close to its deadline, and an end
/// waits this long past it.
pub const MARGIN: u64 = 60;

fn hex0x(bytes: Bytes32) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn address(puzzle_hash: Bytes32, prefix: &str) -> Result<String, Error> {
    Address::new(puzzle_hash, prefix.to_string())
        .encode()
        .map_err(|error| Error::Invalid(error.to_string()))
}

pub fn parse_hash(text: &str) -> Result<Bytes32, Error> {
    let text = text.trim();
    if let Ok(address) = Address::decode(text) {
        return Ok(address.puzzle_hash);
    }
    let bytes = hex::decode(text.trim_start_matches("0x"))
        .map_err(|_| Error::Invalid(format!("not an id: {text}")))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| Error::Invalid(format!("not 32 bytes: {text}")))?;
    Ok(Bytes32::new(bytes))
}

pub fn parse_amount(text: &str) -> Result<u64, Error> {
    text.trim()
        .parse()
        .map_err(|_| Error::Invalid(format!("not an amount in mojos: {text}")))
}

/// An auction followed on the chain to its newest coin, with the NFT it holds.
#[derive(Debug, Clone)]
pub struct Followed {
    pub auction: Auction,
    /// The coin of the NFT the auction holds, while it holds it (unspent, locked by this
    /// auction). Only the id: an `Nft` points into the allocator that read it, so a spend reads
    /// it again into its own (`locked_nft`).
    pub locked_nft: Option<Bytes32>,
    /// Why the auction cannot take bids or end, if it holds no valid NFT.
    pub nft_problem: Option<String>,
    /// Unix seconds of the block that made the newest auction coin; `None` if the peer could
    /// not say (then no grace bid is offered, and with a grace period no end either).
    pub last_spend_time: Option<u64>,
    /// The spend that ended it was the seller's cancel (not an end).
    pub cancelled: bool,
}

async fn confirmed_spend(ctx: &WalletCtx, coin_id: Bytes32) -> Result<CoinSpend, Error> {
    let peer = ctx.peer().map_err(|_| Error::NoPeer)?;
    Ok(peer
        .fetch_coin_spend(coin_id, ctx.wallet.genesis_challenge)
        .await?)
}

/// The NFT at `nft_coin_id`, checked: on the chain, unspent, a standard NFT, and locked by the
/// auction `launcher_id`. `Err` names what is wrong.
async fn locked_nft(
    ctx: &WalletCtx,
    sctx: &mut SpendContext,
    launcher_id: Bytes32,
    nft_coin_id: Bytes32,
) -> Result<Result<Nft, String>, Error> {
    let peer = ctx.peer().map_err(|_| Error::NoPeer)?;
    let genesis = ctx.wallet.genesis_challenge;
    let Some(state) = peer.fetch_optional_coin(nft_coin_id, genesis).await? else {
        return Ok(Err(
            "the NFT this auction names is not on the chain".to_string()
        ));
    };
    if state.spent_height.is_some() {
        return Ok(Err("the NFT this auction names has moved".to_string()));
    }
    let parent = peer
        .fetch_coin_spend(state.coin.parent_coin_info, genesis)
        .await?;
    let puzzle = sctx.alloc(&parent.puzzle_reveal)?;
    let puzzle = Puzzle::parse(sctx, puzzle);
    let solution = sctx.alloc(&parent.solution)?;
    let Some(nft) = Nft::parse_child(sctx, parent.coin, puzzle, solution)? else {
        return Ok(Err(
            "the coin this auction names is not a standard NFT".to_string()
        ));
    };
    if nft.coin.coin_id() != nft_coin_id {
        return Ok(Err(
            "the coin this auction names is not a standard NFT".to_string()
        ));
    }
    if nft.info.p2_puzzle_hash != auction_lock_p2_puzzle_hash(launcher_id) {
        return Ok(Err("the NFT is not held by this auction".to_string()));
    }
    Ok(Ok(nft))
}

/// The CAT a reserve holds bids in; `None` for XCH.
fn reserve_asset(reserve: &AuctionReserve) -> Option<Bytes32> {
    match reserve {
        AuctionReserve::Xch(_) => None,
        AuctionReserve::Cat(cat) => Some(cat.info.asset_id),
    }
}

fn royalty(nft: &Nft) -> RoyaltyInfo {
    RoyaltyInfo::new(
        nft.info.launcher_id,
        nft.info.royalty_puzzle_hash,
        nft.info.royalty_basis_points,
    )
}

/// A CAT reserve's lineage proof, from the spend that made it (as the SDK's tests take it).
async fn reserve_lineage_proof(
    ctx: &WalletCtx,
    launch: &AuctionLaunch,
) -> Result<Option<LineageProof>, Error> {
    if launch.memo.reserve.asset_id().is_none() {
        return Ok(None);
    }
    let parent = confirmed_spend(ctx, launch.reserve_parent_coin_info()).await?;
    let mut sctx = SpendContext::new();
    let puzzle = sctx.alloc(&parent.puzzle_reveal)?;
    let puzzle = Puzzle::parse(&sctx, puzzle);
    let solution = sctx.alloc(&parent.solution)?;
    let lock = auction_lock_p2_puzzle_hash(launch.launcher_coin.coin_id());
    let reserve = Cat::parse_children(&mut sctx, parent.coin, puzzle, solution)?
        .unwrap_or_default()
        .into_iter()
        .find(|cat| cat.info.p2_puzzle_hash == lock && cat.coin.amount == 0)
        .ok_or_else(|| Error::NotAuction("its bid reserve is not there".to_string()))?;
    Ok(reserve.lineage_proof)
}

/// Reads the auction `launcher_id` from the chain and follows it to its newest coin. Only
/// confirmed spends are read (the driver runs their actions without checking them otherwise).
pub async fn follow(ctx: &WalletCtx, launcher_id: Bytes32) -> Result<Followed, Error> {
    let peer = ctx.peer().map_err(|_| Error::NoPeer)?;
    let genesis = ctx.wallet.genesis_challenge;
    let Some(launcher) = peer.fetch_optional_coin_spend(launcher_id, genesis).await? else {
        return Err(Error::NotAuction(
            "nothing was launched with this id".to_string(),
        ));
    };
    let mut sctx = SpendContext::new();
    let solution = sctx.alloc(&launcher.solution)?;
    let launch = AuctionLaunch::parse(&sctx, launcher.coin, solution)
        .ok_or_else(|| Error::NotAuction("this id is not an auction".to_string()))?;

    // The NFT as the auction was launched with it: its royalty is part of the auction's puzzle.
    let launch_nft = locked_nft(ctx, &mut sctx, launcher_id, launch.nft_coin_id()).await?;
    let nft_royalty = if let Ok(nft) = &launch_nft {
        royalty(nft)
    } else {
        // Moved since: read it as the launch saw it, for the royalty only.
        let state = peer
            .fetch_optional_coin(launch.nft_coin_id(), genesis)
            .await?
            .ok_or_else(|| Error::NotAuction("its NFT is not on the chain".to_string()))?;
        let parent = confirmed_spend(ctx, state.coin.parent_coin_info).await?;
        let puzzle = sctx.alloc(&parent.puzzle_reveal)?;
        let puzzle = Puzzle::parse(&sctx, puzzle);
        let solution = sctx.alloc(&parent.solution)?;
        let nft = Nft::parse_child(&mut sctx, parent.coin, puzzle, solution)?
            .ok_or_else(|| Error::NotAuction("its NFT is not a standard NFT".to_string()))?;
        royalty(&nft)
    };
    let mut auction =
        launch.into_auction(nft_royalty, reserve_lineage_proof(ctx, &launch).await?)?;

    let mut cancelled = false;
    let last_spend_time;
    loop {
        let state = peer
            .fetch_optional_coin(auction.coin.coin_id(), genesis)
            .await?
            .ok_or_else(|| {
                Error::NotAuction("the auction's coin is not on the chain".to_string())
            })?;
        if state.spent_height.is_none() {
            // Only a grace period needs the last spend's time; a peer that cannot say gives none.
            last_spend_time = match state.created_height {
                Some(height) if auction.info.settings.timings.grace_period > 0 => peer
                    .block_timestamp(height)
                    .await
                    .ok()
                    .map(|(_, time)| time),
                _ => None,
            };
            break;
        }
        let spend = confirmed_spend(ctx, auction.coin.coin_id()).await?;
        let solution = sctx.alloc(&spend.solution)?;
        let was_ended = auction.info.state.ended;
        auction = auction.parse_child(&mut sctx, solution)?;
        if auction.info.state.ended && !was_ended {
            cancelled = ran_action(&sctx, solution, auction.info.cancel_action_hash().into());
        }
    }

    // The NFT it holds now: the one it was launched with until the end or a cancel moves it.
    let (locked, nft_problem) = if auction.info.state.ended {
        (None, None)
    } else {
        match launch_nft {
            Ok(nft) => (Some(nft.coin.coin_id()), None),
            Err(problem) => (None, Some(problem)),
        }
    };

    Ok(Followed {
        auction,
        locked_nft: locked,
        nft_problem,
        last_spend_time,
        cancelled,
    })
}

/// Whether the auction spend whose solution this is ran the action puzzle `action_hash`.
fn ran_action(sctx: &SpendContext, solution: NodePtr, action_hash: Bytes32) -> bool {
    let Ok(solution) =
        SingletonSolution::<RawActionLayerSolution<NodePtr, NodePtr, NodePtr>>::from_clvm(
            sctx, solution,
        )
    else {
        return false;
    };
    solution
        .inner_solution
        .puzzles
        .iter()
        .any(|puzzle| Bytes32::from(tree_hash(sctx, *puzzle)) == action_hash)
}

/// The least the next bid can be: the minimum before any bid, then the winning bid plus the
/// increment (and always more than the winning bid).
pub fn next_minimum_bid(auction: &Auction) -> u64 {
    let state = auction.info.state;
    let verifier = auction.info.settings.bid_verifier;
    if state.winning_bid.amount == 0 {
        return verifier.minimum_bid().get();
    }
    let winning = state.winning_bid.amount;
    let step = match verifier {
        AuctionBidVerifier::Flat { bid_increment, .. } => bid_increment.get(),
        AuctionBidVerifier::Percent {
            bid_increment_bps, ..
        } => calculate_auction_payment(winning, bid_increment_bps.get()).unwrap_or(u64::MAX),
    };
    winning
        .saturating_add(step.max(1))
        .max(verifier.minimum_bid().get())
}

/// Where the auction stands at `now`.
pub fn status(followed: &Followed, now: u64) -> AuctionStatus {
    let info = &followed.auction.info;
    let timings = info.settings.timings;
    if info.state.ended {
        if info.state.winning_bid.amount > 0 {
            return AuctionStatus::Sold;
        }
        // A cancel and an end without bids leave the same state; the action that ran tells.
        return if followed.cancelled {
            AuctionStatus::Cancelled
        } else {
            AuctionStatus::Unsold
        };
    }
    if now < timings.end_time || in_grace_window(followed, now) {
        AuctionStatus::Live
    } else {
        AuctionStatus::Ending
    }
}

/// After the end time, a bid is still valid within the grace period of the last spend.
fn in_grace_window(followed: &Followed, now: u64) -> bool {
    let grace = followed.auction.info.settings.timings.grace_period;
    followed
        .last_spend_time
        .is_some_and(|last| grace > 0 && now.saturating_add(MARGIN) < last.saturating_add(grace))
}

/// Whether a bid placed now must be a grace-period bid, and whether it may be placed at all.
fn bid_window(followed: &Followed, now: u64) -> Result<bool, String> {
    let timings = followed.auction.info.settings.timings;
    if now.saturating_add(MARGIN) < timings.end_time {
        return Ok(false);
    }
    if in_grace_window(followed, now) && now >= timings.end_time {
        return Ok(true);
    }
    Err("bidding has closed".to_string())
}

/// Whether the auction can be ended now: past its end time, and past the grace period since the
/// last spend.
///
/// With a grace period, the last spend's time is needed: grace bids after the end time can keep
/// it open. A peer that cannot say when it was gives no end at all (rather than one the chain
/// would refuse); without a grace period the time is not needed.
fn can_end(followed: &Followed, now: u64) -> bool {
    let info = &followed.auction.info;
    let timings = info.settings.timings;
    let last_spend = if timings.grace_period == 0 {
        timings.end_time
    } else {
        let Some(last) = followed.last_spend_time else {
            return false;
        };
        last
    };
    !info.state.ended
        && followed.locked_nft.is_some()
        && now >= timings.end_time.saturating_add(MARGIN)
        && now
            >= last_spend
                .max(timings.end_time)
                .saturating_add(timings.grace_period)
                .saturating_add(MARGIN)
}

/// Bounds on another seller's auction too: an end time or grace period this far off would lock
/// the winning bid as long.
fn timing_problem(timings: AuctionTimings, now: u64) -> Option<String> {
    if timings.end_time > now.saturating_add(MAX_DURATION) {
        return Some("it ends more than 90 days from now".to_string());
    }
    if timings.grace_period > MAX_GRACE {
        return Some("its grace period is longer than a day".to_string());
    }
    None
}

/// The auction as the screens show it.
pub async fn view(ctx: &WalletCtx, followed: &Followed, now: u64) -> Result<AuctionView, Error> {
    let info = &followed.auction.info;
    let settings = info.settings;
    let payments = settings.payments;
    let prefix = &ctx.prefix;
    let owns = async |puzzle_hash: Bytes32| -> Result<bool, Error> {
        Ok(ctx.wallet.db.public_key(puzzle_hash).await?.is_some())
    };
    let status = status(followed, now);
    let is_mine = owns(payments.payout_puzzle_hash).await?;
    let winning = info.state.winning_bid;
    let has_bid = winning.amount > 0;
    let next = next_minimum_bid(&followed.auction);

    let cannot_bid = if reserve_asset(&info.reserve).is_some() {
        Some("this auction takes bids in a token; Klimper bids in XCH only".to_string())
    } else if let Some(problem) = &followed.nft_problem {
        Some(problem.clone())
    } else if let Some(problem) = timing_problem(settings.timings, now) {
        Some(problem)
    } else if status != AuctionStatus::Live {
        Some("bidding has closed".to_string())
    } else {
        bid_window(followed, now).err()
    };

    Ok(AuctionView {
        id: hex0x(info.launcher_id),
        nft_id: hex0x(info.nft_royalty.launcher_id),
        nft_address: address(info.nft_royalty.launcher_id, "nft")?,
        nft_coin_id: hex0x(info.nft_coin_id),
        bid_asset_id: reserve_asset(&info.reserve).map(hex::encode),
        minimum_bid: settings.bid_verifier.minimum_bid().get().to_string(),
        increment: match settings.bid_verifier {
            AuctionBidVerifier::Flat { bid_increment, .. } => Increment::Flat {
                amount: bid_increment.get().to_string(),
            },
            AuctionBidVerifier::Percent {
                bid_increment_bps, ..
            } => Increment::Percent {
                bps: bid_increment_bps.get(),
            },
        },
        end_time: settings.timings.end_time,
        grace_period: settings.timings.grace_period,
        buyers_premium_bps: payments.buyers_premium.bps,
        buyers_premium_address: address(payments.buyers_premium.puzzle_hash, prefix)?,
        commission_bps: payments.commission.bps,
        commission_address: address(payments.commission.puzzle_hash, prefix)?,
        royalty_bps: info.nft_royalty.basis_points,
        royalty_address: address(info.nft_royalty.puzzle_hash, prefix)?,
        seller_address: address(payments.payout_puzzle_hash, prefix)?,
        winning_bid: if has_bid {
            Some(Bid {
                amount: winning.amount.to_string(),
                address: address(winning.puzzle_hash, prefix)?,
                is_mine: owns(winning.puzzle_hash).await?,
            })
        } else {
            None
        },
        next_minimum_bid: next.to_string(),
        next_minimum_cost: info
            .reserve_amount_for_bid(next)
            .map_or_else(|| "0".to_string(), |cost| cost.to_string()),
        last_spend_time: followed.last_spend_time,
        in_grace: bid_window(followed, now) == Ok(true),
        status,
        can_bid: cannot_bid.is_none(),
        cannot_bid_reason: cannot_bid,
        can_end: can_end(followed, now),
        is_mine,
        can_cancel: is_mine && !has_bid && !info.state.ended && followed.locked_nft.is_some(),
    })
}

/// One of the wallet's spendable XCH coins of at least `amount`, the smallest that will do, with
/// its standard key: spent by hand where the auction needs a known parent or puzzle hash.
async fn own_xch_coin(
    ctx: &WalletCtx,
    amount: u64,
) -> Result<(Coin, chia_wallet_sdk::chia::bls::PublicKey), Error> {
    let mut coins = ctx.wallet.db.selectable_xch_coins().await?;
    coins.sort_by_key(|coin| coin.amount);
    for coin in coins {
        if coin.amount < amount {
            continue;
        }
        let Some(public_key) = ctx.wallet.db.public_key(coin.puzzle_hash).await? else {
            continue;
        };
        if Bytes32::from(StandardArgs::curry_tree_hash(public_key)) == coin.puzzle_hash {
            return Ok((coin, public_key));
        }
    }
    Err(Error::Invalid(format!(
        "the wallet needs one coin of at least {amount} mojos for this"
    )))
}

/// The wallet's own standard puzzle hash (where bids are refunded, the NFT goes, and a seller is
/// paid), refused unless standard.
async fn own_puzzle_hash(ctx: &WalletCtx) -> Result<Bytes32, Error> {
    let puzzle_hash = ctx.wallet.change_p2_puzzle_hash().await?;
    let Some(public_key) = ctx.wallet.db.public_key(puzzle_hash).await? else {
        return Err(Error::Invalid(
            "the wallet has no key for its own address".to_string(),
        ));
    };
    if Bytes32::from(StandardArgs::curry_tree_hash(public_key)) != puzzle_hash {
        return Err(Error::Invalid(
            "the wallet's address is not a standard puzzle".to_string(),
        ));
    }
    Ok(puzzle_hash)
}

/// The settings a start request asks for, checked: a minimum and an increment above zero, an end
/// at least ten minutes and at most 90 days away, a grace period of at most a day. No buyer's
/// premium and no commission: Klimper takes nothing.
pub fn settings(
    req: &StartRequest,
    payout_puzzle_hash: Bytes32,
    now: u64,
) -> Result<AuctionSettings, Error> {
    let nonzero = |amount: u64, what: &str| {
        NonZeroU64::new(amount)
            .ok_or_else(|| Error::Invalid(format!("{what} must be more than zero")))
    };
    let minimum_bid = nonzero(parse_amount(&req.minimum_bid)?, "the minimum bid")?;
    let bid_verifier = match &req.increment {
        Increment::Flat { amount } => AuctionBidVerifier::Flat {
            minimum_bid,
            bid_increment: nonzero(parse_amount(amount)?, "the increment")?,
        },
        Increment::Percent { bps } => {
            if *bps > AUCTION_MAX_BPS {
                return Err(Error::Invalid(
                    "the increment is more than 100%".to_string(),
                ));
            }
            AuctionBidVerifier::Percent {
                minimum_bid,
                bid_increment_bps: nonzero(*bps, "the increment")?,
            }
        }
    };
    if req.end_time < now.saturating_add(MIN_DURATION) {
        return Err(Error::Invalid(
            "the auction must run at least ten minutes".to_string(),
        ));
    }
    let timings = AuctionTimings::new(req.end_time, req.grace_period);
    if let Some(problem) = timing_problem(timings, now) {
        return Err(Error::Invalid(format!("the auction: {problem}")));
    }
    Ok(AuctionSettings::new(
        bid_verifier,
        timings,
        AuctionPayments::new(
            AuctionBpsPayment::new(0, payout_puzzle_hash),
            AuctionBpsPayment::new(0, payout_puzzle_hash),
            payout_puzzle_hash,
        ),
    ))
}

/// Starts an auction of one of the wallet's NFTs, with bids in XCH: the NFT sent to the auction's
/// lock (with the launcher id in the memo, so the wallet can find it again), and one of the
/// wallet's coins launching the auction and making its empty reserve. Answers the auction's id
/// and the signed bundle.
pub async fn start(
    ctx: &WalletCtx,
    req: &StartRequest,
    now: u64,
) -> Result<(Bytes32, SpendBundle), Error> {
    let nft_id = parse_hash(&req.nft_id)?;
    let fee = req
        .fee
        .as_deref()
        .map(parse_amount)
        .transpose()?
        .unwrap_or(0);
    let payout = own_puzzle_hash(ctx).await?;
    let settings = settings(req, payout, now)?;

    let Some(nft) = ctx.wallet.db.spendable_nft(nft_id).await? else {
        return Err(Error::Invalid(
            "the wallet does not hold this NFT".to_string(),
        ));
    };
    let mut sctx = SpendContext::new();
    let nft = nft.deserialize(&mut sctx)?;

    // The coin that launches the auction comes first: the launcher's id, and with it the lock's
    // puzzle hash, follow from it.
    let (funding, funding_key) = own_xch_coin(ctx, fee.saturating_add(1)).await?;
    let launcher = Launcher::new(funding.coin_id(), 1);
    let launcher_id = launcher.coin().coin_id();
    let lock = auction_lock_p2_puzzle_hash(launcher_id);

    // The NFT to the lock, through Sage's spends, never with the launching coin.
    let memos = sctx.memos(&clvm_list!(lock, launcher_id))?;
    let actions = [Action::send(Id::Existing(nft_id), lock, 1, memos)];
    let mut spends = ctx
        .wallet
        .prepare_spends_for_selection(&mut sctx, &[nft.coin.coin_id()])
        .await?;
    ctx.wallet
        .select_spends_excluding(&mut sctx, &mut spends, &actions, &[funding.coin_id()])
        .await?;
    let deltas = spends.apply(&mut sctx, &actions)?;
    let outputs = ctx
        .wallet
        .complete_spends(&mut sctx, &deltas, spends)
        .await?;
    let locked = *outputs
        .nfts
        .get(&Id::Existing(nft_id))
        .ok_or_else(|| Error::Invalid("the NFT did not move to the auction".to_string()))?;

    let reserve = AuctionReserve::Xch(Coin::new(funding.coin_id(), lock, 0));
    let (launch, auction) = launcher.launch_auction(&mut sctx, settings, reserve, &locked)?;
    let change = funding.amount - 1 - fee;
    let mut conditions = launch
        .create_coin(lock, 0, Memos::None)
        .assert_concurrent_spend(nft.coin.coin_id());
    if change > 0 {
        let hint = sctx.hint(payout)?;
        conditions = conditions.create_coin(payout, change, hint);
    }
    if fee > 0 {
        conditions = conditions.reserve_fee(fee);
    }
    StandardLayer::new(funding_key).spend(&mut sctx, funding, conditions)?;

    let bundle = ctx
        .sign(SpendBundle::new(sctx.take(), Signature::default()))
        .await?;
    debug_assert_eq!(auction.info.launcher_id, launcher_id);
    Ok((launcher_id, bundle))
}

/// What the bundle leaves to the farmer: inputs less outputs, from every spend's conditions.
fn bundle_fee(bundle: &SpendBundle) -> Result<i128, Error> {
    let mut sctx = SpendContext::new();
    let mut fee = 0i128;
    for spend in &bundle.coin_spends {
        fee += i128::from(spend.coin.amount);
        let puzzle = sctx.alloc(&spend.puzzle_reveal)?;
        let solution = sctx.alloc(&spend.solution)?;
        let output = run_puzzle(&mut sctx, puzzle, solution)?;
        for condition in Vec::<Condition>::from_clvm(&sctx, output)? {
            if let Some(create) = condition.into_create_coin() {
                fee -= i128::from(create.amount);
            }
        }
    }
    Ok(fee)
}

/// Bids `amount` mojos of XCH on the auction, with `fee` as the network fee. What is locked up is
/// the bid with the buyer's premium and the royalty; an outbid bid comes back in full, to the
/// wallet's own address, as the NFT does if this bid wins. The auction is read from the chain
/// right before: a bid someone placed first, or an auction that cannot end, refuses here.
pub async fn bid(
    ctx: &WalletCtx,
    launcher_id: Bytes32,
    amount: u64,
    fee: u64,
    now: u64,
) -> Result<SpendBundle, Error> {
    let followed = follow(ctx, launcher_id).await?;
    let info = followed.auction.info;
    if reserve_asset(&info.reserve).is_some() {
        return Err(Error::Closed(
            "this auction takes bids in a token; Klimper bids in XCH only".to_string(),
        ));
    }
    if let Some(problem) = &followed.nft_problem {
        return Err(Error::Closed(problem.clone()));
    }
    if let Some(problem) = timing_problem(info.settings.timings, now) {
        return Err(Error::Closed(problem));
    }
    if info.state.ended {
        return Err(Error::Closed("the auction has ended".to_string()));
    }
    let grace = bid_window(&followed, now).map_err(Error::Closed)?;
    if amount < next_minimum_bid(&followed.auction) {
        return Err(Error::Invalid(format!(
            "the least bid now is {}",
            next_minimum_bid(&followed.auction)
        )));
    }
    let reserve_amount = info
        .reserve_amount_for_bid(amount)
        .ok_or_else(|| Error::Invalid("this bid is too large".to_string()))?;

    let mut sctx = SpendContext::new();
    let bid = AuctionBid::new(amount, own_puzzle_hash(ctx).await?);
    let action = followed.auction.bid_action_spend(&mut sctx, bid, grace)?;
    // The driver runs the bid as the puzzle will: a bid it refuses would fail on the chain.
    let child = followed.auction.child_state(&mut sctx, &[action])?;
    if child.state.reserve_amount != reserve_amount {
        return Err(Error::Invalid(
            "the auction would lock up another amount than shown".to_string(),
        ));
    }

    // The wallet's coins pay the reserve (left unreserved: the auction's reserve takes it) and
    // the fee. The first of their spends asserts this bid's announcement (Sage's spend builder
    // puts required conditions there), and Sage ties the rest to it as concurrent spends, so
    // they pay for nothing else. A remark keeps the auction's id in the wallet's own spend.
    let actions = [
        Action::Fee(FeeAction {
            amount: reserve_amount,
            reserved: false,
        }),
        Action::fee(fee),
    ];
    let mut spends: Spends = ctx
        .wallet
        .prepare_spends(&mut sctx, Vec::new(), &actions)
        .await?;
    let remark = sctx.alloc(&launcher_id)?;
    spends.conditions.required = Conditions::new()
        .assert_coin_announcement(followed.auction.bid_announcement_id(bid))
        .remark(remark);
    let deltas = spends.apply(&mut sctx, &actions)?;
    ctx.wallet
        .complete_spends(&mut sctx, &deltas, spends)
        .await?;
    let _ = followed
        .auction
        .spend(&mut sctx, vec![action], Vec::new())?;

    let bundle = SpendBundle::new(sctx.take(), Signature::default());
    // Everything above the network fee goes into the reserve; nothing is left to the farmer.
    if bundle_fee(&bundle)? != i128::from(fee) {
        return Err(Error::Invalid(
            "the bid would leave more than the fee to the network".to_string(),
        ));
    }
    Ok(ctx.sign(bundle).await?)
}

/// The wallet's fee, if any, tied to the auction's spend: a fee spend that could be sent alone
/// would pay for nothing.
async fn tied_fee(
    ctx: &WalletCtx,
    sctx: &mut SpendContext,
    auction_coin_id: Bytes32,
    fee: u64,
) -> Result<(), Error> {
    if fee == 0 {
        return Ok(());
    }
    let actions = [Action::fee(fee)];
    let mut spends = ctx
        .wallet
        .prepare_spends(sctx, Vec::new(), &actions)
        .await?;
    spends.conditions.required = Conditions::new().assert_concurrent_spend(auction_coin_id);
    let deltas = spends.apply(sctx, &actions)?;
    ctx.wallet.complete_spends(sctx, &deltas, spends).await?;
    Ok(())
}

/// Ends an auction past its end time and grace period: the NFT to the winner (or back to the
/// seller, if nobody bid), the winning bid to the seller. Anyone may end it; `fee` is this
/// wallet's.
pub async fn end(
    ctx: &WalletCtx,
    launcher_id: Bytes32,
    fee: u64,
    now: u64,
) -> Result<SpendBundle, Error> {
    let followed = follow(ctx, launcher_id).await?;
    if followed.auction.info.state.ended {
        return Err(Error::Closed("the auction has ended".to_string()));
    }
    let Some(nft_coin_id) = followed.locked_nft else {
        return Err(Error::Closed(
            followed
                .nft_problem
                .unwrap_or_else(|| "the auction holds no NFT".to_string()),
        ));
    };
    if !can_end(&followed, now) {
        return Err(Error::Invalid(
            "the auction cannot be ended yet".to_string(),
        ));
    }
    let mut sctx = SpendContext::new();
    let nft = locked_nft(ctx, &mut sctx, launcher_id, nft_coin_id)
        .await?
        .map_err(Error::Closed)?;
    let auction = followed.auction;
    let action = auction.end_action_spend(&mut sctx, &nft)?;
    let _ = auction.unlock_nft(&mut sctx, &nft)?;
    let _ = auction.spend(&mut sctx, vec![action], Vec::new())?;
    tied_fee(ctx, &mut sctx, auction.coin.coin_id(), fee).await?;
    Ok(ctx
        .sign(SpendBundle::new(sctx.take(), Signature::default()))
        .await?)
}

/// Cancels the wallet's own auction before any bid: the NFT comes back to the seller's address.
/// The auction only accepts a cancel from a coin at that address, so the wallet makes one (of
/// nothing) and spends it in the same bundle.
pub async fn cancel(ctx: &WalletCtx, launcher_id: Bytes32, fee: u64) -> Result<SpendBundle, Error> {
    let followed = follow(ctx, launcher_id).await?;
    let info = followed.auction.info;
    let payout = info.settings.payments.payout_puzzle_hash;
    let Some(payout_key) = ctx.wallet.db.public_key(payout).await? else {
        return Err(Error::NotMine);
    };
    if Bytes32::from(StandardArgs::curry_tree_hash(payout_key)) != payout {
        return Err(Error::NotMine);
    }
    if info.state.ended {
        return Err(Error::Closed("the auction has ended".to_string()));
    }
    if info.state.winning_bid.amount > 0 {
        return Err(Error::Closed(
            "someone has bid: the auction runs to its end".to_string(),
        ));
    }
    let Some(nft_coin_id) = followed.locked_nft else {
        return Err(Error::Closed(
            followed
                .nft_problem
                .unwrap_or_else(|| "the auction holds no NFT".to_string()),
        ));
    };

    let mut sctx = SpendContext::new();
    let nft = locked_nft(ctx, &mut sctx, launcher_id, nft_coin_id)
        .await?
        .map_err(Error::Closed)?;
    let auction = followed.auction;
    let auction_coin_id = auction.coin.coin_id();
    let (coin, key) = own_xch_coin(ctx, fee).await?;
    let mut conditions = Conditions::new()
        .create_coin(payout, 0, Memos::None)
        .assert_concurrent_spend(auction_coin_id);
    if coin.amount > fee {
        let hint = sctx.hint(payout)?;
        conditions = conditions.create_coin(payout, coin.amount - fee, hint);
    }
    if fee > 0 {
        conditions = conditions.reserve_fee(fee);
    }
    StandardLayer::new(key).spend(&mut sctx, coin, conditions)?;
    let messenger = Coin::new(coin.coin_id(), payout, 0);
    let authorization = auction.cancel_authorization(&mut sctx)?;
    let messenger_spend =
        StandardLayer::new(payout_key).spend_with_conditions(&mut sctx, authorization)?;
    sctx.spend(messenger, messenger_spend)?;
    let action = auction.cancel_action_spend(&mut sctx, &nft)?;
    let _ = auction.unlock_nft(&mut sctx, &nft)?;
    let _ = auction.spend(&mut sctx, vec![action], Vec::new())?;
    Ok(ctx
        .sign(SpendBundle::new(sctx.take(), Signature::default()))
        .await?)
}

#[cfg(test)]
mod tests {
    use chia_sdk_next::{AuctionInfo, AuctionState};
    use chia_wallet_sdk::chia::puzzle_types::{EveProof, Proof};

    use super::*;

    const NOW: u64 = 1_800_000_000;
    const SELLER: Bytes32 = Bytes32::new([5; 32]);

    fn request(end_time: u64, grace_period: u64) -> StartRequest {
        StartRequest {
            nft_id: hex::encode([1; 32]),
            minimum_bid: "1000".to_string(),
            increment: Increment::Flat {
                amount: "100".to_string(),
            },
            end_time,
            grace_period,
            fee: None,
        }
    }

    fn followed(end_time: u64, grace: u64, last_spend_time: Option<u64>, bid: u64) -> Followed {
        let settings = settings(&request(end_time, grace), SELLER, end_time - 3_600).unwrap();
        let mut state = AuctionState::initial(SELLER);
        state.winning_bid = AuctionBid::new(bid, Bytes32::new([9; 32]));
        let info = AuctionInfo::new(
            Bytes32::new([2; 32]),
            settings,
            Bytes32::new([3; 32]),
            RoyaltyInfo::new(Bytes32::new([4; 32]), Bytes32::new([6; 32]), 0),
            state,
            AuctionReserve::Xch(Coin::new(Bytes32::new([7; 32]), Bytes32::new([8; 32]), 0)),
        );
        let coin = Coin::new(Bytes32::new([10; 32]), Bytes32::new([11; 32]), 1);
        let proof = Proof::Eve(EveProof {
            parent_parent_coin_info: Bytes32::new([12; 32]),
            parent_amount: 1,
        });
        Followed {
            auction: Auction::new(coin, proof, info),
            locked_nft: Some(Bytes32::new([3; 32])),
            nft_problem: None,
            last_spend_time,
            cancelled: false,
        }
    }

    #[test]
    fn a_start_is_bounded() {
        assert!(settings(&request(NOW + 3_600, 0), SELLER, NOW).is_ok());
        assert!(settings(&request(NOW + MIN_DURATION - 1, 0), SELLER, NOW).is_err());
        assert!(settings(&request(NOW + MAX_DURATION, MAX_GRACE), SELLER, NOW).is_ok());
        assert!(settings(&request(NOW + MAX_DURATION + 1, 0), SELLER, NOW).is_err());
        assert!(settings(&request(NOW + 3_600, MAX_GRACE + 1), SELLER, NOW).is_err());
        let mut zero = request(NOW + 3_600, 0);
        zero.minimum_bid = "0".to_string();
        assert!(settings(&zero, SELLER, NOW).is_err());
        let mut over = request(NOW + 3_600, 0);
        over.increment = Increment::Percent { bps: 10_001 };
        assert!(settings(&over, SELLER, NOW).is_err());
    }

    #[test]
    fn another_sellers_far_off_auction_is_not_biddable() {
        let timings = AuctionTimings::new(NOW + MAX_DURATION + 1, 0);
        assert!(timing_problem(timings, NOW).is_some());
        assert!(timing_problem(AuctionTimings::new(NOW + 60, MAX_GRACE + 1), NOW).is_some());
        assert!(timing_problem(AuctionTimings::new(NOW + 60, MAX_GRACE), NOW).is_none());
    }

    #[test]
    fn bids_before_the_end_then_only_in_the_grace_period() {
        let end = NOW;
        // Without a grace period: until a minute before the end.
        let plain = followed(end, 0, None, 0);
        assert_eq!(bid_window(&plain, end - MARGIN - 1), Ok(false));
        assert!(bid_window(&plain, end - MARGIN).is_err());
        assert!(bid_window(&plain, end + 1).is_err());
        // With one: after the end, within the grace of the last spend, as a grace bid.
        let graced = followed(end, 600, Some(end - 100), 1_000);
        assert_eq!(bid_window(&graced, end + 10), Ok(true));
        assert!(bid_window(&graced, end - 100 + 600 - MARGIN).is_err());
        // The last spend's time unknown: no grace bid.
        let unknown = followed(end, 600, None, 1_000);
        assert!(bid_window(&unknown, end + 10).is_err());
    }

    #[test]
    fn an_end_waits_for_the_end_time_and_the_grace_period() {
        let end = NOW;
        let plain = followed(end, 0, None, 1_000);
        assert!(!can_end(&plain, end));
        assert!(can_end(&plain, end + MARGIN));
        // A grace bid after the end time keeps it open for the grace period after it.
        let graced = followed(end, 600, Some(end + 300), 1_000);
        assert!(!can_end(&graced, end + MARGIN));
        assert!(!can_end(&graced, end + 300 + 600));
        assert!(can_end(&graced, end + 300 + 600 + MARGIN));
        // With a grace period, an unknown last spend gives no end at all.
        let unknown = followed(end, 600, None, 1_000);
        assert!(!can_end(&unknown, end + 10 * MAX_GRACE));
        // An ended auction, or one whose NFT is gone, is not ended again.
        let mut gone = followed(end, 0, None, 1_000);
        gone.locked_nft = None;
        assert!(!can_end(&gone, end + MARGIN));
    }

    #[test]
    fn statuses() {
        let end = NOW;
        assert_eq!(
            status(&followed(end, 0, None, 0), end - 1),
            AuctionStatus::Live
        );
        assert_eq!(
            status(&followed(end, 0, None, 0), end + 1),
            AuctionStatus::Ending
        );
        let mut sold = followed(end, 0, None, 1_000);
        sold.auction.info.state.ended = true;
        assert_eq!(status(&sold, end + 1), AuctionStatus::Sold);
        let mut unsold = followed(end, 0, None, 0);
        unsold.auction.info.state.ended = true;
        assert_eq!(status(&unsold, end + 1), AuctionStatus::Unsold);
        unsold.cancelled = true;
        assert_eq!(status(&unsold, end + 1), AuctionStatus::Cancelled);
    }

    #[test]
    fn the_next_bid() {
        assert_eq!(next_minimum_bid(&followed(NOW, 0, None, 0).auction), 1_000);
        assert_eq!(
            next_minimum_bid(&followed(NOW, 0, None, 1_000).auction),
            1_100
        );
    }
}
