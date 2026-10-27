// SPDX-License-Identifier: AGPL-3.0-or-later
//! `chia_createPartialOffer` end to end (TASK-165): a wallet makes a partial offer through
//! Klimper's own path — the key-free one-sided offer with Sage's offer code, the cross-check
//! against Sage's summary, wallet-kit's submit, one approval with the partial terms, the
//! driver's make — and only the `partial1…` text comes back. The text is read back with the
//! driver, kept in the wallet's own partial offers, and filled in two parts by a second
//! wallet on the simulated chain, the chain accepting each fill. The refusals run on the
//! same path: malformed params, terms the driver refuses, more than the wallet holds.

use std::cell::RefCell;

use chia_partial_offer::PartialOffer;
use serde_json::Value;
use tauri_plugin_klimper_partial_offers::{
    mine, records::Status, store::Store, wallet as partial_wallet,
};

use super::create_offer::OfferView;
use super::option_tests::{Run, Setup, chain_time, run, who};
use super::partial_offer::*;
use super::sage_sim::SimWallet;
use super::take_tests::{issue_amount, know_cats, sage_summary};
use super::*;

/// Make the partial offer with `maker`'s wallet as `chia_createPartialOffer` does after
/// reading its params, under `setup`. The screen's `offer` key and the made offer are kept
/// next to the run.
async fn make(
    maker: &SimWallet,
    params: &PartialParams,
    cats: &[Bytes32],
    setup: &Setup,
) -> (Run, Option<OfferView>, Option<PartialOffer>) {
    let built = match build_partial_offer(&maker.partial_ctx().await, params)
        .await
        .expect("Sage could not build the one-sided offer")
    {
        Ok(built) => built,
        Err(error) => {
            return (
                Run {
                    result: Err(error),
                    prompt: None,
                    sent: None,
                    loan: None,
                },
                None,
                None,
            );
        }
    };
    let summary = sage_summary(maker, cats, &built.spends).await;
    let w = who(maker).await;
    let seen = RefCell::new(None);
    let made = RefCell::new(None);
    let run = run(&w, setup, |ctx, signer, approver, _, _| {
        submit_partial_offer(
            ctx,
            signer,
            approver,
            &summary,
            built,
            params.fee,
            &|view| {
                *seen.borrow_mut() = Some(view.clone());
            },
        )
        .map(|(answer, offer)| {
            *made.borrow_mut() = Some(offer);
            answer
        })
    });
    (run, seen.into_inner(), made.into_inner())
}

/// The params as a website sends them, read as the method reads them.
fn params(json: &Value, now: u64) -> PartialParams {
    partial_params(&json.to_string(), now).expect("params refused")
}

/// The answer of an approved `chia_createPartialOffer`: the text, read back with the driver,
/// and nothing else than the contract's keys.
fn the_offer(run: &Run) -> (String, PartialOffer, Value) {
    let answer: Value = serde_json::from_str(run.ok()).unwrap();
    let keys: Vec<&String> = answer.as_object().unwrap().keys().collect();
    assert!(
        keys.iter()
            .all(|k| ["offer", "id", "coinId", "expiresAt"].contains(&k.as_str())),
        "{keys:?}"
    );
    let text = answer["offer"].as_str().expect("no offer").to_owned();
    assert!(text.starts_with("partial1"), "{text}");
    // Never an ordinary offer: the one-sided offer stays in the wallet.
    assert!(chia_wallet_sdk::driver::decode_offer(&text).is_err());
    let offer = PartialOffer::from_text(&text).unwrap();
    assert!(offer.is_first());
    assert_eq!(answer["coinId"], hex0x(offer.coin.coin_id()));
    assert_eq!(
        answer["id"],
        hex::encode(sage_wallet::sort_offer(offer.to_spend_bundle().unwrap()).name())
    );
    // Nothing was sent.
    assert!(run.sent.is_none());
    (text, offer, answer)
}

/// Fill `paid` of `text` from `taker`, send it, and wait for both wallets to see it.
async fn fill(
    taker: &mut SimWallet,
    maker: &mut SimWallet,
    text: &str,
    paid: u64,
    now: u64,
) -> tauri_plugin_klimper_partial_offers::records::TakeResult {
    let ctx = taker.partial_ctx().await;
    let (result, bundle) = partial_wallet::take(&ctx, text, paid, 0, now)
        .await
        .expect("the fill was refused");
    let spent: Vec<Bytes32> = bundle
        .coin_spends
        .iter()
        .map(|spend| spend.coin.coin_id())
        .collect();
    ctx.submit(bundle)
        .await
        .expect("the chain refused the fill");
    taker.wait_for_coins().await;
    maker.watch_coins(spent).await;
    maker.wait_for_coins().await;
    result
}

/// 1,000 of a CAT for 5,000 mojos of XCH, at least 250 mojos a fill: made over Xchonnect,
/// kept in My partial offers, filled a quarter and then the rest by a second wallet.
#[tokio::test]
async fn a_cat_for_xch_partial_offer_is_made_kept_and_filled_in_two_parts() {
    let mut alice = SimWallet::new(5_000).await;
    let mut bob = alice.next(20_000).await;
    let cat = issue_amount(&mut alice, 1_000).await;
    know_cats(&bob, &[cat]).await;
    let now = chain_time(&alice).await;

    let request = serde_json::json!({
        "walletId": 1,
        "offerAssets": [{ "assetId": hex::encode(cat), "amount": 1_000 }],
        "requestAssets": [{ "assetId": "xch", "amount": "5000" }],
        "minFillRequested": 250,
        "fee": 3,
    });
    let p = params(&request, now);
    let (run, view, made) = make(
        &alice,
        &p,
        &[cat],
        &Setup::new(CHIA_CREATE_PARTIAL_OFFER, now),
    )
    .await;
    let (text, offer, answer) = the_offer(&run);
    assert!(answer.get("expiresAt").is_none());

    // One approval, worded as a partial offer: what it gives and asks, the least fill both
    // ways, no fee per fill, no expiry.
    assert!(run.asked());
    let prompt = run.prompt.as_ref().unwrap();
    assert_eq!(prompt["intent"]["kind"], "offer.create");
    let view = view.expect("no offer on screen");
    let partial = view.partial.as_ref().expect("not shown as a partial offer");
    assert_eq!(partial.min_fill.amount, 250);
    assert_eq!(partial.min_fill.asset, AssetId::Xch);
    assert_eq!(partial.min_fill_offered.amount, 50);
    assert_eq!(partial.min_fill_offered.asset, AssetId::Cat(cat));
    assert!(partial.fee_per_fill.is_none());
    assert_eq!(view.gives[0].amount, 1_000);
    assert_eq!(view.asks[0].amount, 5_000);
    assert_eq!(view.expires_at, None);
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["partial"]["min_fill"]["amount"], "250");

    // The text read back: the terms asked, the maker this wallet.
    assert_eq!(offer.info.initial_offered(), 1_000);
    assert_eq!(offer.info.initial_requested(), 5_000);
    assert_eq!(offer.info.min_fill_requested(), 250);
    assert_eq!(offer.info.required_fee, None);
    assert_eq!(offer.info.expiration, None);
    let made = made.unwrap();
    assert_eq!(made.coin, offer.coin);
    assert!(
        alice
            .partial_ctx()
            .await
            .owns(offer.info.maker_puzzle_hash)
            .await
            .unwrap()
    );

    // Kept in the wallet's own partial offers, under its first coin's id.
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("partial.json"));
    keep_partial_offer(&store, &made, now);
    let mine = mine::list(&alice.partial_ctx().await, &store, now)
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].id, answer["coinId"].as_str().unwrap());
    assert_eq!(mine[0].view.status, Status::Open);

    // Bob fills a quarter, then the rest from the next offer.
    let first = fill(&mut bob, &mut alice, &text, 1_250, now).await;
    assert_eq!(
        (first.paid.as_str(), first.bought.as_str()),
        ("1250", "250")
    );
    assert_eq!(bob.wallet.db.cat_balance(cat).await.unwrap(), 250);
    let next = first.next_offer.expect("nothing left");
    let last = fill(&mut bob, &mut alice, &next, 3_750, now).await;
    assert_eq!(last.bought, "750");
    assert!(last.next_offer.is_none());
    assert_eq!(bob.wallet.db.cat_balance(cat).await.unwrap(), 1_000);

    // The maker's list follows it through both fills: 5,000 mojos paid for the 1,000.
    let mine = mine::list(&alice.partial_ctx().await, &store, now)
        .await
        .unwrap();
    assert_eq!(mine[0].view.status, Status::Filled);
    let paid: u64 = mine[0]
        .fills
        .iter()
        .map(|f| f.paid.parse::<u64>().unwrap())
        .sum();
    let bought: u64 = mine[0]
        .fills
        .iter()
        .map(|f| f.bought.parse::<u64>().unwrap())
        .sum();
    assert_eq!((mine[0].fills.len(), paid, bought), (2, 5_000, 1_000));
}

/// 8,000 mojos of XCH for 2,000 of a CAT, at least 100 of it a fill, a fee of 5 mojos per
/// fill and an expiry: the answer names the expiry, and two fills settle.
#[tokio::test]
async fn an_xch_for_cat_partial_offer_with_a_fee_per_fill_and_an_expiry_is_filled_twice() {
    let mut alice = SimWallet::new(10_000).await;
    let mut bob = alice.next(3_000).await;
    let cat = issue_amount(&mut bob, 2_000).await;
    know_cats(&alice, &[cat]).await;
    let now = chain_time(&alice).await;
    let expires_at = now + 10_000;

    let request = serde_json::json!({
        "offerAssets": [{ "amount": 8_000 }],
        "requestAssets": [{ "assetId": format!("0x{}", hex::encode(cat)), "amount": 2_000 }],
        "minFillRequested": "100",
        "requiredFee": 5,
        "expiresAt": expires_at,
    });
    let p = params(&request, now);
    let (run, view, _) = make(
        &alice,
        &p,
        &[cat],
        &Setup::new(CHIA_CREATE_PARTIAL_OFFER, now),
    )
    .await;
    let (text, offer, answer) = the_offer(&run);
    assert_eq!(answer["expiresAt"], expires_at);
    assert_eq!(offer.info.expiration, Some(expires_at));
    assert_eq!(offer.info.required_fee, Some(5));
    assert_eq!(offer.info.min_fill_requested(), 100);
    let view = view.unwrap();
    assert_eq!(view.expires_at, Some(expires_at));
    let partial = view.partial.unwrap();
    assert_eq!(partial.min_fill.asset, AssetId::Cat(cat));
    assert_eq!(partial.min_fill_offered.amount, 400);
    assert_eq!(partial.fee_per_fill.unwrap().amount, 5);

    let first = fill(&mut bob, &mut alice, &text, 500, now).await;
    assert_eq!(first.bought, "2000");
    let next = first.next_offer.unwrap();
    let last = fill(&mut bob, &mut alice, &next, 1_500, now).await;
    assert_eq!(last.bought, "6000");
    assert!(last.next_offer.is_none());
    assert_eq!(bob.wallet.db.cat_balance(cat).await.unwrap(), 0);
    let followed = partial_wallet::view(&alice.partial_ctx().await, &text, now)
        .await
        .unwrap();
    assert_eq!(followed.status, Status::Filled);
    assert_eq!(followed.fills.len(), 2);
}

/// More than the wallet holds is Sage's coin selection failing: `build_failed`.
#[tokio::test]
async fn more_than_the_wallet_holds_is_not_built() {
    let alice = SimWallet::new(5_000).await;
    let now = chain_time(&alice).await;
    let request = serde_json::json!({
        "offerAssets": [{ "amount": 1_000_000 }],
        "requestAssets": [{ "assetId": hex::encode([0x61; 32]), "amount": 10 }],
        "minFillRequested": 1,
    });
    let p = params(&request, now);
    assert!(
        build_partial_offer(&alice.partial_ctx().await, &p)
            .await
            .is_err()
    );
}

#[test]
fn partial_params_are_read_strictly() {
    let now = 1_800_000_000;
    let cat_id = "5151515151515151515151515151515151515151515151515151515151515151";
    let cat_entry = serde_json::json!({ "assetId": cat_id, "amount": 1_000 });
    let xch_entry = serde_json::json!({ "amount": "4000" });
    let good = serde_json::json!({
        "walletId": 1,
        "offerAssets": [cat_entry],
        "requestAssets": [xch_entry],
        "minFillRequested": 400,
        "requiredFee": "0",
        "expiresAt": now + 60,
        "fee": 7,
    });
    let p = partial_params(&good.to_string(), now).unwrap();
    assert_eq!(p.offered.asset, AssetId::Cat(Bytes32::new([0x51; 32])));
    assert_eq!(p.offered.amount, 1_000);
    assert_eq!(p.requested.asset, AssetId::Xch);
    assert_eq!(p.requested.amount, 4_000);
    assert_eq!(p.min_fill_requested, 400);
    // A required fee of 0 is none.
    assert_eq!(p.required_fee, None);
    assert_eq!(p.expires_at, Some(now + 60));
    // The website's fee is read (and must be well formed), never used.
    assert_eq!(p.fee, own_fee());

    let option = chia_wallet_sdk::prelude::Address::new(Bytes32::new([0x77; 32]), "option".into())
        .encode()
        .unwrap();
    let with = |key: &str, value: Value| {
        let mut request = good.clone();
        request[key] = value;
        request
    };
    let without = |key: &str| {
        let mut request = good.clone();
        request.as_object_mut().unwrap().remove(key);
        request
    };
    for (bad, why) in [
        (serde_json::json!("not an object"), "offerAssets"),
        (without("offerAssets"), "offerAssets"),
        (with("offerAssets", serde_json::json!([])), "offerAssets"),
        // Two assets on a side.
        (
            with("offerAssets", serde_json::json!([cat_entry, xch_entry])),
            "offerAssets",
        ),
        (
            with("requestAssets", serde_json::json!([xch_entry, xch_entry])),
            "requestAssets",
        ),
        (
            with("offerAssets", serde_json::json!([{ "amount": 0 }])),
            "offerAssets",
        ),
        (
            with(
                "offerAssets",
                serde_json::json!([{ "assetId": "zz", "amount": 1 }]),
            ),
            "offerAssets",
        ),
        (
            with(
                "requestAssets",
                serde_json::json!([{ "assetId": option, "amount": 1 }]),
            ),
            "unsupported_asset",
        ),
        // The same asset on both sides.
        (
            with("requestAssets", serde_json::json!([cat_entry])),
            "requestAssets",
        ),
        // The least fill: missing, 0, above what is requested, buying nothing.
        (without("minFillRequested"), "minFillRequested"),
        (
            with("minFillRequested", serde_json::json!(0)),
            "minFillRequested",
        ),
        (
            with("minFillRequested", serde_json::json!(4_001)),
            "minFillRequested",
        ),
        (
            with("minFillRequested", serde_json::json!("lots")),
            "minFillRequested",
        ),
        (
            with(
                "offerAssets",
                serde_json::json!([{ "assetId": cat_id, "amount": 1 }]),
            ),
            "minFillRequested",
        ),
        (with("requiredFee", serde_json::json!(-1)), "requiredFee"),
        // The expiry: 0, past, now, not a number.
        (with("expiresAt", serde_json::json!(0)), "expiresAt"),
        (with("expiresAt", serde_json::json!(now - 1)), "expiresAt"),
        (with("expiresAt", serde_json::json!(now)), "expiresAt"),
        (with("expiresAt", serde_json::json!("soon")), "expiresAt"),
        (with("fee", serde_json::json!(-1)), "fee"),
    ] {
        let error = partial_params(&bad.to_string(), now).unwrap_err();
        // Invalid params, as `chia_createOffer` refuses them; an option or NFT as content it
        // does not support, as there.
        let code = if why == "unsupported_asset" {
            codes::UNSUPPORTED_CONTENT
        } else {
            codes::INVALID_PARAMS
        };
        assert_eq!(error.code, code, "{bad}");
        assert!(error.data.unwrap_or_default().contains(why), "{bad}: {why}");
    }
    // A least fill that buys nothing says so.
    let error = partial_params(
        &with(
            "offerAssets",
            serde_json::json!([{ "assetId": cat_id, "amount": 1 }]),
        )
        .to_string(),
        now,
    )
    .unwrap_err();
    assert!(error.message.contains("buys nothing"), "{}", error.message);
    assert!(partial_params("{", now).is_err());
}

#[test]
fn create_partial_offer_is_wallet_built_and_granted_like_create_offer() {
    assert!(is_wallet_built(CHIA_CREATE_PARTIAL_OFFER));
    assert!(WALLET_BUILT_METHODS.contains(&CHIA_CREATE_PARTIAL_OFFER));
    assert!(!super::options::OPTION_METHODS.contains(&CHIA_CREATE_PARTIAL_OFFER));
    assert!(
        crate::pairing::granted_methods()
            .iter()
            .any(|m| m == CHIA_CREATE_PARTIAL_OFFER)
    );
    assert_eq!(METHODS_REVISION, 3);
}
