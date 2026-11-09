// SPDX-License-Identifier: AGPL-3.0-or-later
//! Auctions end to end on Sage wallets and a simulated chain: started by one wallet, bid on by
//! two (the first outbid and refunded in full), ended, the NFT in the winner's wallet and the bid
//! with the seller; cancelled before a bid, the NFT back with the seller; an id that is no
//! auction refused.
#![allow(clippy::unwrap_used)]

use chia_wallet_sdk::{
    chia::{protocol::Bytes32, puzzle_types::nft::NftMetadata},
    driver::SpendContext,
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
