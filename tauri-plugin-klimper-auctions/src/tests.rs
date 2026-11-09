// SPDX-License-Identifier: AGPL-3.0-or-later
//! Auctions end to end on Sage wallets and a simulated chain: started by one wallet, bid on by
//! two (the first outbid and refunded in full), ended, the NFT in the winner's wallet and the bid
//! with the seller; cancelled before a bid, the NFT back with the seller; an id that is no
//! auction refused.
#![allow(clippy::unwrap_used)]

use chia_sdk_next::{AuctionBid, AuctionExt, auction_lock_p2_puzzle_hash};
use chia_wallet_sdk::{
    chia::{
        protocol::{Bytes32, Coin, SpendBundle},
        puzzle_types::{Memos, nft::NftMetadata},
    },
    clvm_traits::FromClvm,
    clvmr::{NodePtr, serde::node_to_bytes},
    driver::SpendContext,
    types::{Condition, run_puzzle},
};
use sage_database::DeserializePrimitive;
use sage_wallet::WalletNftMint;

use crate::{
    Error,
    records::{AuctionStatus, Increment, StartRequest},
    sage_sim::SimWallet,
    wallet,
};

const ROYALTY_BPS: u16 = 300;

/// Mints one NFT into `wallet` (from a new DID, as Sage mints), with a 3% royalty to
/// `royalty_to`. Returns its launcher id.
async fn mint(wallet: &mut SimWallet, royalty_to: Bytes32) -> Bytes32 {
    let (spends, did) = wallet.wallet.create_did(0).await.unwrap();
    wallet.transact(spends).await;
    wallet.wait_for_coins().await;
    let (spends, nfts) = wallet
        .wallet
        .bulk_mint_nfts(
            0,
            did.info.launcher_id,
            vec![WalletNftMint {
                metadata: NftMetadata::default(),
                p2_puzzle_hash: None,
                royalty_puzzle_hash: Some(royalty_to),
                royalty_basis_points: ROYALTY_BPS,
            }],
        )
        .await
        .unwrap();
    wallet.transact(spends).await;
    wallet.wait_for_coins().await;
    nfts[0].info.launcher_id
}

async fn chain_time(wallet: &SimWallet) -> u64 {
    wallet.sim.lock().await.next_timestamp()
}

/// The NFT's coin id in `wallet`'s database, if the wallet holds it.
async fn held_nft(wallet: &SimWallet, nft_id: Bytes32) -> Option<Bytes32> {
    let nft = wallet.wallet.db.spendable_nft(nft_id).await.unwrap()?;
    let mut ctx = SpendContext::new();
    Some(nft.deserialize(&mut ctx).unwrap().coin.coin_id())
}

/// Starts an auction of `nft_id` ending an hour after `now`, and waits for the chain to have it.
async fn start(seller: &mut SimWallet, nft_id: Bytes32, now: u64) -> Bytes32 {
    let nft_coin = held_nft(seller, nft_id).await.unwrap();
    let request = StartRequest {
        nft_id: hex::encode(nft_id),
        minimum_bid: "1000".to_string(),
        increment: Increment::Flat {
            amount: "100".to_string(),
        },
        end_time: now + 3_600,
        grace_period: 0,
        fee: Some("10".to_string()),
    };
    let ctx = seller.ctx().await;
    let (id, bundle) = wallet::start(&ctx, &request, now).await.unwrap();
    ctx.submit(bundle).await.unwrap();
    seller.wait_until_spent(nft_coin).await;
    seller.wait_for_coins().await;
    id
}

/// Bids `amount` from `bidder` and waits until the auction's coin is spent.
async fn bid(bidder: &mut SimWallet, id: Bytes32, amount: u64, now: u64) {
    let ctx = bidder.ctx().await;
    let before = wallet::follow(&ctx, id)
        .await
        .unwrap()
        .auction
        .coin
        .coin_id();
    let bundle = wallet::bid(&ctx, id, amount, 5, now).await.unwrap();
    ctx.submit(bundle).await.unwrap();
    bidder.wait_until_spent(before).await;
    bidder.wait_for_coins().await;
}

#[tokio::test]
async fn an_auction_is_started_bid_on_twice_and_ended() {
    let mut seller = SimWallet::new(10_000).await;
    let mut first = seller.next(20_000).await;
    let mut second = first.next(20_000).await;
    let royalty_to = Bytes32::new([0x77; 32]);
    let nft_id = mint(&mut seller, royalty_to).await;
    let now = chain_time(&seller).await;

    let id = start(&mut seller, nft_id, now).await;
    assert!(held_nft(&seller, nft_id).await.is_none(), "the NFT left");

    // Anyone with the id reads it from the chain, checked.
    let followed = wallet::follow(&first.ctx().await, id).await.unwrap();
    let view = wallet::view(&first.ctx().await, &followed, now)
        .await
        .unwrap();
    assert_eq!(view.status, AuctionStatus::Live);
    assert!(view.can_bid, "{:?}", view.cannot_bid_reason);
    assert!(!view.is_mine);
    assert_eq!(view.nft_id, format!("0x{}", hex::encode(nft_id)));
    assert_eq!(view.royalty_bps, ROYALTY_BPS);
    assert_eq!(view.next_minimum_bid, "1000");
    assert_eq!(view.next_minimum_cost, "1030", "the bid and the 3% royalty");
    let seen_by_seller = wallet::view(&seller.ctx().await, &followed, now)
        .await
        .unwrap();
    assert!(seen_by_seller.is_mine && seen_by_seller.can_cancel);

    // Too little is refused before anything is signed.
    let low = wallet::bid(&first.ctx().await, id, 999, 0, now).await;
    assert!(matches!(low, Err(Error::Invalid(_))), "{low:?}");

    let first_before = first.wallet.db.xch_balance().await.unwrap();
    bid(&mut first, id, 1_000, now).await;
    assert_eq!(
        first.wallet.db.xch_balance().await.unwrap(),
        first_before - 1_030 - 5
    );
    let view = wallet::view(
        &second.ctx().await,
        &wallet::follow(&second.ctx().await, id).await.unwrap(),
        now,
    )
    .await
    .unwrap();
    assert_eq!(view.winning_bid.as_ref().unwrap().amount, "1000");
    assert_eq!(view.next_minimum_bid, "1100");
    assert!(!view.can_end);

    let second_before = second.wallet.db.xch_balance().await.unwrap();
    bid(&mut second, id, 1_100, now).await;
    assert_eq!(
        second.wallet.db.xch_balance().await.unwrap(),
        second_before - 1_133 - 5
    );
    // The first bid came back in full.
    first.wait_for_coins().await;
    assert_eq!(
        first.wallet.db.xch_balance().await.unwrap(),
        first_before - 5
    );
    // A seller's cancel is refused once someone has bid.
    let refused = wallet::cancel(&seller.ctx().await, id, 0).await;
    assert!(matches!(refused, Err(Error::Closed(_))), "{refused:?}");

    // Past the end: no more bids; anyone ends it (here the seller, with a fee).
    let end_time = now + 3_600;
    seller
        .sim
        .lock()
        .await
        .set_next_timestamp(end_time + 120)
        .unwrap();
    let later = end_time + 200;
    let late = wallet::bid(&first.ctx().await, id, 2_000, 0, later).await;
    assert!(matches!(late, Err(Error::Closed(_))), "{late:?}");
    let seller_before = seller.wallet.db.xch_balance().await.unwrap();
    let ctx = seller.ctx().await;
    let auction_coin = wallet::follow(&ctx, id)
        .await
        .unwrap()
        .auction
        .coin
        .coin_id();
    let bundle = wallet::end(&ctx, id, 7, later).await.unwrap();
    ctx.submit(bundle).await.unwrap();
    seller.wait_until_spent(auction_coin).await;
    seller.wait_for_coins().await;

    let followed = wallet::follow(&seller.ctx().await, id).await.unwrap();
    let view = wallet::view(&seller.ctx().await, &followed, later)
        .await
        .unwrap();
    assert_eq!(view.status, AuctionStatus::Sold);
    assert!(!view.can_end && !view.can_bid && !view.can_cancel);
    // The bid to the seller, less the fee the seller paid to end it.
    assert_eq!(
        seller.wallet.db.xch_balance().await.unwrap(),
        seller_before + 1_100 - 7
    );
    // The NFT in the winner's wallet, once its sync has seen it.
    let mut held = false;
    for _ in 0..100 {
        if held_nft(&second, nft_id).await.is_some() {
            held = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(held, "the winner holds the NFT");
}

#[tokio::test]
async fn the_seller_cancels_before_any_bid_and_gets_the_nft_back() {
    let mut seller = SimWallet::new(10_000).await;
    let bidder = seller.next(20_000).await;
    let nft_id = mint(&mut seller, Bytes32::new([0x77; 32])).await;
    let now = chain_time(&seller).await;
    let id = start(&mut seller, nft_id, now).await;

    // Nobody else can cancel it.
    let not_mine = wallet::cancel(&bidder.ctx().await, id, 0).await;
    assert!(matches!(not_mine, Err(Error::NotMine)), "{not_mine:?}");

    let ctx = seller.ctx().await;
    let auction_coin = wallet::follow(&ctx, id)
        .await
        .unwrap()
        .auction
        .coin
        .coin_id();
    let bundle = wallet::cancel(&ctx, id, 3).await.unwrap();
    ctx.submit(bundle).await.unwrap();
    seller.wait_until_spent(auction_coin).await;
    seller.wait_for_coins().await;

    let followed = wallet::follow(&seller.ctx().await, id).await.unwrap();
    let view = wallet::view(&seller.ctx().await, &followed, now)
        .await
        .unwrap();
    assert_eq!(view.status, AuctionStatus::Cancelled);
    assert!(!view.can_bid && !view.can_cancel);
    assert!(
        held_nft(&seller, nft_id).await.is_some(),
        "the NFT came back"
    );
}

#[tokio::test]
async fn an_auction_without_a_bid_ends_unsold_and_the_nft_goes_back() {
    let mut seller = SimWallet::new(10_000).await;
    let other = seller.next(5_000).await;
    let nft_id = mint(&mut seller, Bytes32::new([0x77; 32])).await;
    let now = chain_time(&seller).await;
    let id = start(&mut seller, nft_id, now).await;

    // Not before its end time.
    let early = wallet::end(&other.ctx().await, id, 0, now).await;
    assert!(matches!(early, Err(Error::Invalid(_))), "{early:?}");

    let end_time = now + 3_600;
    seller
        .sim
        .lock()
        .await
        .set_next_timestamp(end_time + 120)
        .unwrap();
    let later = end_time + 200;
    // Anyone may end it, with no fee: here a wallet that is neither seller nor bidder.
    let ctx = other.ctx().await;
    let auction_coin = wallet::follow(&ctx, id)
        .await
        .unwrap()
        .auction
        .coin
        .coin_id();
    let bundle = wallet::end(&ctx, id, 0, later).await.unwrap();
    ctx.submit(bundle).await.unwrap();
    seller.wait_until_spent(auction_coin).await;

    let followed = wallet::follow(&seller.ctx().await, id).await.unwrap();
    let view = wallet::view(&seller.ctx().await, &followed, later)
        .await
        .unwrap();
    assert_eq!(view.status, AuctionStatus::Unsold);
    let mut back = false;
    for _ in 0..100 {
        if held_nft(&seller, nft_id).await.is_some() {
            back = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(back, "the NFT came back to the seller");
}

#[tokio::test]
async fn an_id_that_is_no_auction_is_refused() {
    let mut wallet = SimWallet::new(10_000).await;
    let nft_id = mint(&mut wallet, Bytes32::new([0x77; 32])).await;
    // An NFT's launcher is a launcher, but not an auction's.
    let read = crate::wallet::follow(&wallet.ctx().await, nft_id).await;
    assert!(matches!(read, Err(Error::NotAuction(_))), "{read:?}");
    let missing = crate::wallet::follow(&wallet.ctx().await, Bytes32::new([1; 32])).await;
    assert!(matches!(missing, Err(Error::NotAuction(_))), "{missing:?}");
}

/// Every coin spend of `bundle` with the conditions its puzzle outputs.
/// A coin spent, and the conditions its spend outputs.
type Spent = (Coin, Vec<Condition<NodePtr>>);

fn conditions(bundle: &SpendBundle) -> (SpendContext, Vec<Spent>) {
    let mut ctx = SpendContext::new();
    let mut all = Vec::new();
    for spend in &bundle.coin_spends {
        let puzzle = ctx.alloc(&spend.puzzle_reveal).unwrap();
        let solution = ctx.alloc(&spend.solution).unwrap();
        let output = run_puzzle(&mut ctx, puzzle, solution).unwrap();
        all.push((
            spend.coin,
            Vec::<Condition<NodePtr>>::from_clvm(&ctx, output).unwrap(),
        ));
    }
    (ctx, all)
}

/// Whether the CLVM at `node` holds the 32 bytes of `id`.
fn holds(ctx: &SpendContext, node: NodePtr, id: Bytes32) -> bool {
    let bytes = node_to_bytes(ctx, node).unwrap();
    bytes.windows(32).any(|window| window == id.as_ref())
}

fn asserts_concurrent(conditions: &[Condition<NodePtr>], coin_id: Bytes32) -> bool {
    conditions.iter().any(|condition| {
        matches!(condition, Condition::AssertConcurrentSpend(assert) if assert.coin_id == coin_id)
    })
}

#[tokio::test]
async fn the_wallets_spends_are_tied_to_the_auction() {
    let mut seller = SimWallet::new(10_000).await;
    let bidder = seller.next(20_000).await;
    let nft_id = mint(&mut seller, Bytes32::new([0x77; 32])).await;
    let now = chain_time(&seller).await;
    let nft_coin = held_nft(&seller, nft_id).await.unwrap();

    // Start: no coin spent twice; the launching coin asserts the NFT's spend; the NFT's send to
    // the lock carries the auction's id in its memo.
    let request = StartRequest {
        nft_id: hex::encode(nft_id),
        minimum_bid: "1000".to_string(),
        increment: Increment::Flat {
            amount: "100".to_string(),
        },
        end_time: now + 3_600,
        grace_period: 0,
        fee: Some("10".to_string()),
    };
    let ctx = seller.ctx().await;
    let (id, bundle) = wallet::start(&ctx, &request, now).await.unwrap();
    let (cctx, spends) = conditions(&bundle);
    let mut ids: Vec<Bytes32> = spends.iter().map(|(coin, _)| coin.coin_id()).collect();
    let count = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), count, "a coin is spent twice");
    let lock = auction_lock_p2_puzzle_hash(id);
    // The launching coin is the launcher's parent.
    let launcher = spends
        .iter()
        .find(|(coin, _)| coin.coin_id() == id)
        .unwrap()
        .0;
    let launching = spends
        .iter()
        .find(|(coin, _)| coin.coin_id() == launcher.parent_coin_info)
        .unwrap();
    assert!(asserts_concurrent(&launching.1, nft_coin));
    let (_, nft_conditions) = spends
        .iter()
        .find(|(coin, _)| coin.coin_id() == nft_coin)
        .unwrap();
    assert!(nft_conditions.iter().any(|condition| matches!(
        condition,
        Condition::CreateCoin(create) if match create.memos {
            Memos::Some(memos) => holds(&cctx, memos, id),
            Memos::None => false,
        }
    )));
    ctx.submit(bundle).await.unwrap();
    seller.wait_until_spent(nft_coin).await;

    // A bid: the wallet's spend asserts the bid's announcement and remarks the auction's id.
    let bctx = bidder.ctx().await;
    let followed = wallet::follow(&bctx, id).await.unwrap();
    let bid_to = bidder.wallet.change_p2_puzzle_hash().await.unwrap();
    let announcement = followed
        .auction
        .bid_announcement_id(AuctionBid::new(1_000, bid_to));
    let bundle = wallet::bid(&bctx, id, 1_000, 5, now).await.unwrap();
    let (cctx, spends) = conditions(&bundle);
    let own: Vec<_> = spends
        .iter()
        .filter(|(coin, _)| {
            coin.puzzle_hash != lock && coin.coin_id() != followed.auction.coin.coin_id()
        })
        .collect();
    assert!(!own.is_empty());
    assert!(own.iter().any(
        |(_, conditions)| conditions.iter().any(|condition| matches!(
            condition,
            Condition::AssertCoinAnnouncement(assert) if assert.announcement_id == announcement
        ))
    ));
    assert!(own.iter().any(
        |(_, conditions)| conditions.iter().any(|condition| matches!(
            condition,
            Condition::Remark(remark) if holds(&cctx, remark.rest, id)
        ))
    ));
    bctx.submit(bundle).await.unwrap();
    bidder
        .wait_until_spent(followed.auction.coin.coin_id())
        .await;

    // An end with a fee: the fee's spend asserts the auction coin's spend.
    let end_time = now + 3_600;
    seller
        .sim
        .lock()
        .await
        .set_next_timestamp(end_time + 120)
        .unwrap();
    let followed = wallet::follow(&ctx, id).await.unwrap();
    let auction_coin = followed.auction.coin.coin_id();
    let bundle = wallet::end(&ctx, id, 7, end_time + 200).await.unwrap();
    let (_, spends) = conditions(&bundle);
    assert!(spends.iter().any(|(coin, conditions)| {
        coin.coin_id() != auction_coin && asserts_concurrent(conditions, auction_coin)
    }));
}

#[tokio::test]
async fn the_nft_check_refuses_what_the_auction_does_not_hold() {
    let mut seller = SimWallet::new(10_000).await;
    let nft_id = mint(&mut seller, Bytes32::new([0x77; 32])).await;
    let other_nft = mint(&mut seller, Bytes32::new([0x77; 32])).await;
    let now = chain_time(&seller).await;
    let id = start(&mut seller, nft_id, now).await;
    let ctx = seller.ctx().await;
    let held = wallet::follow(&ctx, id).await.unwrap().locked_nft.unwrap();
    let check = async |coin_id: Bytes32| {
        let mut sctx = SpendContext::new();
        wallet::locked_nft(&ctx, &mut sctx, id, coin_id)
            .await
            .unwrap()
            .map(|nft| nft.coin.coin_id())
    };

    assert_eq!(check(held).await, Ok(held));
    let not_locked = held_nft(&seller, other_nft).await.unwrap();
    assert_eq!(
        check(not_locked).await.unwrap_err(),
        "the NFT is not held by this auction"
    );
    let xch = seller.wallet.db.selectable_xch_coins().await.unwrap()[0].coin_id();
    assert_eq!(
        check(xch).await.unwrap_err(),
        "the coin this auction names is not a standard NFT"
    );
    assert_eq!(
        check(Bytes32::new([1; 32])).await.unwrap_err(),
        "the NFT this auction names is not on the chain"
    );

    // Once the NFT has left the auction (here, a cancel), it is refused, and nobody can bid.
    let auction_coin = wallet::follow(&ctx, id)
        .await
        .unwrap()
        .auction
        .coin
        .coin_id();
    let bundle = wallet::cancel(&ctx, id, 0).await.unwrap();
    // A cancel's paying spend asserts the auction coin's spend too.
    let (_, spends) = conditions(&bundle);
    assert!(spends.iter().any(|(coin, conditions)| {
        coin.coin_id() != auction_coin && asserts_concurrent(conditions, auction_coin)
    }));
    ctx.submit(bundle).await.unwrap();
    seller.wait_until_spent(auction_coin).await;
    assert_eq!(
        check(held).await.unwrap_err(),
        "the NFT this auction names has moved"
    );
    let followed = wallet::follow(&ctx, id).await.unwrap();
    assert!(followed.locked_nft.is_none());
    let view = wallet::view(&ctx, &followed, now).await.unwrap();
    assert!(!view.can_bid && !view.can_end && !view.can_cancel);
}
