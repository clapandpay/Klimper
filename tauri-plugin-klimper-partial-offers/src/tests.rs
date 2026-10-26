// SPDX-License-Identifier: AGPL-3.0-or-later
//! Partial offers between two Sage wallets on a simulated chain: made by one, viewed and filled
//! in parts by the other, followed, listed and cancelled. Real wallet databases, real sync,
//! real spends, through the same functions the commands call.
#![allow(clippy::unwrap_used)]

use chia_wallet_sdk::chia::protocol::Bytes32;

use crate::{
    Error, mine,
    records::{MakeRequest, Status},
    sage_sim::SimWallet,
    store::Store,
    wallet,
};

/// A clock before any expiry in these tests.
const NOW: u64 = 1_800_000_000;

/// A new CAT of `amount` in `wallet`.
async fn issue(wallet: &mut SimWallet, amount: u64) -> Bytes32 {
    let (spends, asset_id) = wallet.wallet.issue_cat(amount, 0, None).await.unwrap();
    wallet.transact(spends).await;
    wallet.wait_for_coins().await;
    asset_id
}

/// Put `asset_id` in `wallet`'s database, as Sage does once it has looked a CAT up (there is
/// no network in tests to look it up on).
async fn know_cat(wallet: &SimWallet, asset_id: Bytes32) {
    if wallet.wallet.db.asset(asset_id).await.unwrap().is_none() {
        wallet
            .wallet
            .db
            .insert_asset(sage_database::Asset {
                hash: asset_id,
                name: None,
                ticker: None,
                precision: 3,
                icon_url: None,
                description: None,
                is_sensitive_content: false,
                is_visible: true,
                hidden_puzzle_hash: None,
                kind: sage_database::AssetKind::Token,
            })
            .await
            .unwrap();
    }
}

fn request(
    offered: Option<Bytes32>,
    offered_amount: u64,
    requested: Option<Bytes32>,
    requested_amount: u64,
) -> MakeRequest {
    MakeRequest {
        offered_asset_id: offered.map(hex::encode),
        offered_amount: offered_amount.to_string(),
        requested_asset_id: requested.map(hex::encode),
        requested_amount: requested_amount.to_string(),
        min_fill_requested: (requested_amount / 20).max(1).to_string(),
        required_fee: None,
        expires_at: None,
    }
}

/// Makes a partial offer in `maker` and remembers it in `store`; returns its text and id.
async fn make(maker: &SimWallet, store: &Store, req: &MakeRequest) -> (String, String) {
    let ctx = maker.ctx().await;
    let terms = wallet::terms(&ctx, req).await.unwrap();
    let (offer, _one_sided) = wallet::make(&ctx, &terms).await.unwrap();
    let id = mine::remember(store, &offer, NOW).unwrap();
    (offer.to_text().unwrap(), id)
}

/// Takes `paid` of `text` from `taker`, sends it, and waits for both wallets to see it.
async fn take(
    taker: &mut SimWallet,
    maker: &mut SimWallet,
    text: &str,
    paid: u64,
    fee: u64,
) -> crate::records::TakeResult {
    let ctx = taker.ctx().await;
    let (result, bundle) = wallet::take(&ctx, text, paid, fee, NOW).await.unwrap();
    let spent: Vec<Bytes32> = bundle
        .coin_spends
        .iter()
        .map(|spend| spend.coin.coin_id())
        .collect();
    ctx.submit(bundle).await.unwrap();
    taker.wait_for_coins().await;
    // The maker learns of the payment when it looks at the coins its offer spent.
    maker.watch_coins(spent).await;
    maker.wait_for_coins().await;
    result
}

#[tokio::test]
async fn a_cat_for_xch_offer_is_filled_in_parts_followed_and_listed() {
    let mut maker = SimWallet::new(2_000).await;
    let mut taker = maker.next(20_000).await;
    let cat = issue(&mut maker, 1_000).await;
    know_cat(&taker, cat).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("partial.json"));

    // 1,000 of the CAT for 5,000 mojos of XCH, at least 250 mojos a fill.
    let (text, id) = make(&maker, &store, &request(Some(cat), 1_000, None, 5_000)).await;
    assert!(text.starts_with("partial1"));

    let seen = wallet::view(&taker.ctx().await, &text, NOW).await.unwrap();
    assert_eq!(seen.status, Status::Open);
    assert!(!seen.is_mine);
    assert!(seen.first);
    assert_eq!(seen.remaining_offered, "1000");
    assert_eq!(seen.remaining_requested, "5000");
    assert_eq!(seen.min_fill_requested, "250");
    assert_eq!(seen.min_fill_offered, "50");
    assert!(seen.maker_address.starts_with("txch1"));
    assert!(
        wallet::view(&maker.ctx().await, &text, NOW)
            .await
            .unwrap()
            .is_mine
    );

    // Too little, then a quarter.
    let refused = wallet::take(&taker.ctx().await, &text, 249, 0, NOW).await;
    assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
    let xch_before = taker.wallet.db.xch_balance().await.unwrap();
    let first = take(&mut taker, &mut maker, &text, 1_250, 0).await;
    assert_eq!(
        (first.paid.as_str(), first.bought.as_str()),
        ("1250", "250")
    );
    assert_eq!(taker.wallet.db.cat_balance(cat).await.unwrap(), 250);
    assert_eq!(
        taker.wallet.db.xch_balance().await.unwrap(),
        xch_before - 1_250
    );
    let next = first.next_offer.unwrap();

    // The first text, read again, follows to the rest.
    let again = wallet::view(&taker.ctx().await, &text, NOW).await.unwrap();
    assert_eq!(again.status, Status::PartlyFilled);
    assert_eq!(again.remaining_offered, "750");
    assert_eq!(again.fills.len(), 1);
    assert_eq!(again.offer, next);
    assert!(!again.first);

    // Taking from the old text is refused before anything is signed.
    let stale = wallet::take(&taker.ctx().await, &text, 1_000, 0, NOW).await;
    assert!(matches!(stale, Err(Error::OfferChanged)), "{stale:?}");

    // The rest, with a network fee.
    let last = take(&mut taker, &mut maker, &next, 3_750, 10).await;
    assert_eq!(last.bought, "750");
    assert!(last.next_offer.is_none());
    assert_eq!(taker.wallet.db.cat_balance(cat).await.unwrap(), 1_000);

    let ended = wallet::view(&maker.ctx().await, &text, NOW).await.unwrap();
    assert_eq!(ended.status, Status::Filled);
    assert_eq!(ended.remaining_offered, "0");
    assert_eq!(ended.fills.len(), 2);

    // The maker's list: one offer, two fills, filled, and the maker paid.
    let mine = mine::list(&maker.ctx().await, &store, NOW).await.unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].id, id);
    assert_eq!(mine[0].fills.len(), 2);
    assert_eq!(mine[0].view.status, Status::Filled);
    assert_eq!(store.get(&id).unwrap().unwrap().ended, Some(Status::Filled));
}

#[tokio::test]
async fn xch_for_a_cat_is_filled_once_and_the_rest_clawed_back() {
    let mut maker = SimWallet::new(10_000).await;
    let mut taker = maker.next(3_000).await;
    let cat = issue(&mut taker, 2_000).await;
    know_cat(&maker, cat).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("partial.json"));

    // 8,000 mojos of XCH for 2,000 of the CAT.
    let (text, id) = make(&maker, &store, &request(None, 8_000, Some(cat), 2_000)).await;
    let first = take(&mut taker, &mut maker, &text, 500, 0).await;
    assert_eq!(first.bought, "2000");
    maker.wait_for_cat_balance(cat, 500).await;

    // The list follows it to the rest; the maker cancels with a fee and gets the rest back.
    let listed = mine::list(&maker.ctx().await, &store, NOW).await.unwrap();
    assert_eq!(listed[0].view.status, Status::PartlyFilled);
    assert_eq!(listed[0].view.remaining_offered, "6000");
    let xch_before = maker.wallet.db.xch_balance().await.unwrap();
    let stored = mine::latest(&store, &id).unwrap();
    let ctx = maker.ctx().await;
    let (bundle, _) = wallet::cancel(&ctx, &stored.latest, 5, NOW).await.unwrap();
    let rest = bundle
        .coin_spends
        .iter()
        .map(|spend| spend.coin.coin_id())
        .collect();
    ctx.submit(bundle).await.unwrap();
    maker.watch_coins(rest).await;
    maker.wait_for_coins().await;
    assert_eq!(
        maker.wallet.db.xch_balance().await.unwrap(),
        xch_before + 6_000 - 5
    );
    let listed = mine::list(&maker.ctx().await, &store, NOW).await.unwrap();
    assert_eq!(listed[0].view.status, Status::Cancelled);

    // A taker cannot cancel it, and a cancelled offer cannot be taken.
    let taken = wallet::take(&taker.ctx().await, &stored.latest, 500, 0, NOW).await;
    assert!(
        matches!(taken, Err(Error::Closed(Status::Cancelled))),
        "{taken:?}"
    );
}

#[tokio::test]
async fn a_sent_cancel_reads_as_cancelling_until_the_chain_has_it() {
    let mut maker = SimWallet::new(10_000).await;
    let mut taker = maker.next(10_000).await;
    let cat = issue(&mut maker, 1_000).await;
    know_cat(&taker, cat).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("partial.json"));
    let (text, id) = make(&maker, &store, &request(Some(cat), 1_000, None, 5_000)).await;
    let first = take(&mut taker, &mut maker, &text, 1_250, 0).await;
    let next = first.next_offer.unwrap();

    // A cancel signed and noted as sent, not on the chain yet.
    let ctx = maker.ctx().await;
    let stored = mine::latest(&store, &id).unwrap();
    let (bundle, coin_id) = wallet::cancel(&ctx, &stored.latest, 5, NOW).await.unwrap();
    mine::cancel_sent(&store, &id, "0x01".to_string(), coin_id, NOW).unwrap();
    let listed = mine::list(&ctx, &store, NOW).await.unwrap();
    assert_eq!(listed[0].view.status, Status::Cancelling);
    assert_eq!(listed[0].cancel.as_ref().unwrap().transaction_id, "0x01");
    let seen = wallet::view(&ctx, &next, NOW).await.unwrap();
    let seen = mine::with_pending_cancel(&store, &next, seen).unwrap();
    assert_eq!(seen.status, Status::Cancelling);

    // Confirmed: cancelled, and the note is gone. The simulated chain takes a moment.
    ctx.submit(bundle).await.unwrap();
    let mut listed = Vec::new();
    for _ in 0..100 {
        listed = mine::list(&maker.ctx().await, &store, NOW).await.unwrap();
        if listed[0].view.status != Status::Cancelling {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(listed[0].view.status, Status::Cancelled);
    assert!(listed[0].cancel.is_none());
}

#[tokio::test]
async fn a_fill_before_the_cancel_confirms_makes_the_offer_live_again() {
    let mut maker = SimWallet::new(10_000).await;
    let mut taker = maker.next(10_000).await;
    let cat = issue(&mut maker, 1_000).await;
    know_cat(&taker, cat).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("partial.json"));
    let (text, id) = make(&maker, &store, &request(Some(cat), 1_000, None, 5_000)).await;
    let next = take(&mut taker, &mut maker, &text, 1_250, 0)
        .await
        .next_offer
        .unwrap();

    // The cancel is signed and noted as sent, but a taker's fill reaches the chain first.
    let ctx = maker.ctx().await;
    let (_, coin_id) = wallet::cancel(&ctx, &next, 0, NOW).await.unwrap();
    mine::cancel_sent(&store, &id, "0x01".to_string(), coin_id, NOW).unwrap();
    take(&mut taker, &mut maker, &next, 1_250, 0).await;
    let listed = mine::list(&maker.ctx().await, &store, NOW).await.unwrap();
    assert_eq!(listed[0].view.status, Status::PartlyFilled);
    assert!(listed[0].cancel.is_none());
}

#[tokio::test]
async fn an_offer_cancelled_before_any_fill_cannot_be_taken() {
    let mut maker = SimWallet::new(10_000).await;
    let taker = maker.next(10_000).await;
    let cat = issue(&mut maker, 1_000).await;
    know_cat(&taker, cat).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("partial.json"));
    let (text, _) = make(&maker, &store, &request(Some(cat), 1_000, None, 5_000)).await;

    // Only the maker can cancel it.
    let theirs = wallet::cancel(&taker.ctx().await, &text, 0, NOW).await;
    assert!(matches!(theirs, Err(Error::NotMine)), "{theirs:?}");

    let ctx = maker.ctx().await;
    let (bundle, _) = wallet::cancel(&ctx, &text, 0, NOW).await.unwrap();
    ctx.submit(bundle).await.unwrap();
    maker.wait_for_coins().await;
    assert_eq!(maker.wallet.db.cat_balance(cat).await.unwrap(), 1_000);

    let seen = wallet::view(&taker.ctx().await, &text, NOW).await.unwrap();
    assert_eq!(seen.status, Status::Cancelled);
    let taken = wallet::take(&taker.ctx().await, &text, 1_000, 0, NOW).await;
    assert!(
        matches!(taken, Err(Error::Closed(Status::Cancelled))),
        "{taken:?}"
    );
}

#[tokio::test]
async fn a_required_fee_is_paid_on_top_and_an_expired_offer_is_refused() {
    let mut maker = SimWallet::new(2_000).await;
    let mut taker = maker.next(50_000).await;
    let cat = issue(&mut maker, 1_000).await;
    know_cat(&taker, cat).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("partial.json"));
    let req = MakeRequest {
        required_fee: Some("100".to_string()),
        expires_at: Some(NOW + 3_600),
        ..request(Some(cat), 1_000, None, 5_000)
    };
    let (text, _) = make(&maker, &store, &req).await;

    let seen = wallet::view(&taker.ctx().await, &text, NOW).await.unwrap();
    assert_eq!(seen.required_fee.as_deref(), Some("100"));
    assert_eq!(seen.expires_at, Some(NOW + 3_600));
    let later = wallet::view(&taker.ctx().await, &text, NOW + 3_600)
        .await
        .unwrap();
    assert_eq!(later.status, Status::Expired);
    let expired = wallet::take(&taker.ctx().await, &text, 1_000, 0, NOW + 3_600).await;
    assert!(
        matches!(expired, Err(Error::Closed(Status::Expired))),
        "{expired:?}"
    );

    let xch_before = taker.wallet.db.xch_balance().await.unwrap();
    let _ = take(&mut taker, &mut maker, &text, 1_000, 0).await;
    assert_eq!(
        taker.wallet.db.xch_balance().await.unwrap(),
        xch_before - 1_000 - 100
    );
    assert_eq!(taker.wallet.db.cat_balance(cat).await.unwrap(), 200);
}
