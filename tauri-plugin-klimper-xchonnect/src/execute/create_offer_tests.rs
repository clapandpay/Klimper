// SPDX-License-Identifier: AGPL-3.0-or-later
//! `chia_createOffer` end to end (TASK-142): a lender writes an option and offers it with a
//! principal for the collateral (an open loan offer), through Klimper's own path — the
//! key-free build with Sage's offer code, the cross-check against Sage's summary,
//! wallet-kit's submit with a verified intent kind, one approval — and a second wallet takes
//! the offer with `chia_takeOffer`; the chain must accept the take, the taker ends up holding
//! the option and the principal, the maker the collateral asked for. The refusals run on the
//! same path: an option the wallet does not hold, an expired one, more than the wallet has,
//! no option contracts on the connection, over the option limit, and malformed params. An
//! `expiresAt` from the website is asserted by every spend: taken before it, refused after.

use std::cell::RefCell;

use chia_wallet_sdk::prelude::Address;
use serde_json::Value;

use super::create_offer::*;
use super::option_tests::{Run, Setup, TERM, cat, chain_time, pass_time, run, who, written, xch};
use super::options::{CHIA_TRANSFER_OPTION, Part};
use super::sage_sim::SimWallet;
use super::take_tests::{issue_amount, know_cats, sage_summary, take, take_permissions};
use super::*;

/// `chia_createOffer` and the option contracts, as a connection with both turned on has.
fn offer_setup(now: u64) -> Setup {
    let mut setup = Setup::new(CHIA_CREATE_OFFER, now);
    setup
        .permissions
        .methods
        .push(CHIA_TRANSFER_OPTION.to_owned());
    setup
}

/// Make the offer with `maker`'s wallet as `chia_createOffer` does after reading its params,
/// under `setup`; the screen's `offer` key is kept next to the run.
async fn make(
    maker: &SimWallet,
    params: &OfferParams,
    cats: &[Bytes32],
    setup: &Setup,
) -> (Run, Option<OfferView>) {
    let refused = |error| {
        (
            Run {
                result: Err(error),
                prompt: None,
                sent: None,
                loan: None,
            },
            None,
        )
    };
    if let Err(error) = offer_granted(params, &setup.permissions) {
        return refused(error);
    }
    let built = match build_offer(&maker.wallet, params, setup.now)
        .await
        .expect("Sage could not build the offer")
    {
        Ok(built) => built,
        Err(error) => return refused(error),
    };
    let summary = sage_summary(maker, cats, &built.spends).await;
    let w = who(maker).await;
    let seen = RefCell::new(None);
    let run = run(&w, setup, |ctx, signer, approver, _, _| {
        submit_offer(
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
    });
    (run, seen.into_inner())
}

fn offer_params(offered: Vec<Part>, options: Vec<Bytes32>, requested: Vec<Part>) -> OfferParams {
    OfferParams {
        offered,
        options,
        requested,
        fee: 0,
        expires_at: None,
    }
}

/// The signed offer of an approved `chia_createOffer`.
fn the_offer(run: &Run) -> SpendBundle {
    let answer: Value = serde_json::from_str(run.ok()).unwrap();
    let offer = answer["offer"].as_str().expect("no offer").to_owned();
    assert!(offer.starts_with("offer1"));
    let bundle = chia_wallet_sdk::driver::decode_offer(&offer).unwrap();
    // The id is Sage's: the name of the sorted offer, plain hex.
    assert_eq!(
        answer["id"],
        hex::encode(sage_wallet::sort_offer(bundle.clone()).name())
    );
    // Nothing was sent: the signatures go back to the website.
    assert!(run.sent.is_none());
    bundle
}

/// Send `amount` of `asset_id` from `from` to `to`'s wallet, signed by `from`, and wait
/// until both wallets see it.
async fn give_cat(from: &mut SimWallet, to: &mut SimWallet, asset_id: Bytes32, amount: u64) {
    let to_ph = to.wallet.change_p2_puzzle_hash().await.unwrap();
    let spends = from
        .wallet
        .send_cat(asset_id, vec![(to_ph, amount)], 0, true, vec![], None)
        .await
        .unwrap();
    let bundle = from
        .wallet
        .sign_transaction(
            SpendBundle::new(spends, Signature::default()),
            &from.agg_sig,
            from.master_sk.clone(),
            false,
        )
        .await
        .unwrap();
    from.push_bundle(bundle.clone()).await.unwrap();
    from.wait_for_coins().await;
    know_cats(to, &[asset_id]).await;
    to.watch_coins(
        bundle
            .additions()
            .unwrap()
            .iter()
            .map(Coin::coin_id)
            .collect(),
    )
    .await;
    to.wait_for_coins().await;
}

/// The taker's take of `offer`, as `chia_takeOffer` does, sent to the chain; the maker's
/// wallet is told of the coins it created (the simulated chain only reports coins a wallet
/// asks for), and the tests wait for each wallet with [`until`].
async fn taken(
    taker: &mut SimWallet,
    maker: &mut SimWallet,
    cats: &[Bytes32],
    offer: &SpendBundle,
) {
    know_cats(taker, cats).await;
    let outcome = take(taker, cats, offer, 0, &take_permissions(), true).await;
    if let Err(error) = &outcome.result {
        panic!("the take was refused: {error:?}");
    }
    assert!(outcome.asked);
    let bundle = outcome.sent.expect("nothing sent");
    taker
        .push_bundle(bundle.clone())
        .await
        .expect("the chain refused the take");
    let created: Vec<Bytes32> = bundle
        .additions()
        .unwrap()
        .iter()
        .map(Coin::coin_id)
        .collect();
    maker.watch_coins(created).await;
}

/// Let `wallet` sync until `done` holds (each sync event at most 10 s apart).
async fn until(wallet: &mut SimWallet, done: impl AsyncFn(&SimWallet) -> bool) {
    for _ in 0..50 {
        if done(wallet).await {
            return;
        }
        wallet.next_event().await;
    }
    panic!("the wallet never got there");
}

async fn holds(wallet: &SimWallet, launcher_id: Bytes32) -> bool {
    wallet
        .wallet
        .db
        .spendable_option(launcher_id)
        .await
        .unwrap()
        .is_some()
}

/// The open loan: Alice lends 300 mojos against 1000 of a CAT, repaid with 315 mojos before
/// the expiry. She writes the option (locking 1000 of the CAT herself) and offers it with
/// the principal for the borrower's 1000; Bob takes it and holds the option and the
/// principal, Alice the 1000 he paid.
#[tokio::test]
async fn an_open_loan_offer_of_an_option_and_xch_for_a_cat_is_taken() {
    let mut alice = SimWallet::new(5_000).await;
    let mut bob = alice.next(2_000).await;
    let collateral = issue_amount(&mut alice, 2_000).await;
    give_cat(&mut alice, &mut bob, collateral, 1_000).await;
    let (launcher_id, expiry) =
        written(&mut alice, cat(collateral, 1_000), xch(315), &[collateral]).await;
    let alice_xch = alice.wallet.db.xch_balance().await.unwrap();
    let bob_xch = bob.wallet.db.xch_balance().await.unwrap();

    let now = chain_time(&alice).await;
    let params = offer_params(
        vec![xch(300)],
        vec![launcher_id],
        vec![cat(collateral, 1_000)],
    );
    let (run, view) = make(&alice, &params, &[collateral], &offer_setup(now)).await;
    let offer = the_offer(&run);

    let prompt = run.prompt.as_ref().expect("never asked");
    // Released: the offer's signatures go to the website, so only direct assertions bind.
    assert_eq!(prompt["linking"], "direct");
    assert_eq!(prompt["intent"]["kind"], "option.transfer");
    assert_eq!(prompt["intent"]["kind_verified"], true);
    let transfer = super::option_tests::effect(&run, "transfer");
    assert_eq!(transfer["from_user"], true);
    assert_eq!(transfer["to_settlement"], true);
    assert_eq!(transfer["launcher_id"], hex::encode(launcher_id));
    // The screen's own key: the option with its terms, the principal, what is asked.
    let view = view.expect("no offer on the screen");
    assert_eq!(view.options.len(), 1);
    let terms = view.options[0].terms;
    assert_eq!(terms.collateral.asset, AssetId::Cat(collateral));
    assert_eq!(terms.collateral.amount, 1_000);
    assert_eq!(terms.strike.asset, AssetId::Xch);
    assert_eq!(terms.strike.amount, 315);
    assert_eq!(terms.expiration_seconds, expiry);
    assert_eq!(
        terms.creator_puzzle_hash,
        alice.wallet.change_p2_puzzle_hash().await.unwrap()
    );
    assert_eq!(view.gives.len(), 1);
    assert_eq!(view.gives[0].amount, 300);
    assert_eq!(view.asks[0].asset, AssetId::Cat(collateral));
    assert_eq!(view.expires_at, Some(expiry));
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(
        json["options"][0]["terms"]["collateral"]["asset"]["type"],
        "cat"
    );

    taken(&mut bob, &mut alice, &[collateral], &offer).await;
    until(&mut bob, async |w| {
        holds(w, launcher_id).await && w.wallet.db.xch_balance().await.unwrap() == bob_xch + 300
    })
    .await;
    until(&mut alice, async |w| {
        w.wallet.db.cat_balance(collateral).await.unwrap() == 2_000
    })
    .await;
    // Bob holds the option and the principal; the collateral he paid is Alice's.
    assert!(
        bob.wallet
            .db
            .spendable_option(launcher_id)
            .await
            .unwrap()
            .is_some(),
        "the taker does not hold the option"
    );
    assert_eq!(bob.wallet.db.cat_balance(collateral).await.unwrap(), 0);
    assert_eq!(bob.wallet.db.xch_balance().await.unwrap(), bob_xch + 300);
    assert_eq!(
        alice.wallet.db.xch_balance().await.unwrap(),
        alice_xch - 300
    );
    // The borrower's 1000 next to her own locked 1000 (Sage still counts locked collateral:
    // the writer may claim it back).
    assert_eq!(
        alice.wallet.db.cat_balance(collateral).await.unwrap(),
        2_000
    );
}

/// Collateral in XCH, the principal in a CAT, the collateral asked in XCH.
#[tokio::test]
async fn an_option_and_a_cat_offered_for_xch_is_taken() {
    let mut alice = SimWallet::new(5_000).await;
    let mut bob = alice.next(3_000).await;
    let principal = issue_amount(&mut alice, 500).await;
    let strike = issue_amount(&mut alice, 10).await;
    let (launcher_id, _) = written(&mut alice, xch(1_000), cat(strike, 10), &[strike]).await;
    let alice_xch = alice.wallet.db.xch_balance().await.unwrap();

    let now = chain_time(&alice).await;
    let params = offer_params(
        vec![cat(principal, 400)],
        vec![launcher_id],
        vec![xch(1_000)],
    );
    let (run, view) = make(&alice, &params, &[principal], &offer_setup(now)).await;
    let offer = the_offer(&run);
    assert_eq!(
        view.unwrap().options[0].terms.collateral.asset,
        AssetId::Xch
    );

    let bob_xch = bob.wallet.db.xch_balance().await.unwrap();
    taken(&mut bob, &mut alice, &[principal], &offer).await;
    until(&mut bob, async |w| {
        holds(w, launcher_id).await && w.wallet.db.cat_balance(principal).await.unwrap() == 400
    })
    .await;
    until(&mut alice, async |w| {
        w.wallet.db.xch_balance().await.unwrap() == alice_xch + 1_000
            && w.wallet.db.cat_balance(principal).await.unwrap() == 100
    })
    .await;
    assert!(
        bob.wallet
            .db
            .spendable_option(launcher_id)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(bob.wallet.db.cat_balance(principal).await.unwrap(), 400);
    assert_eq!(bob.wallet.db.xch_balance().await.unwrap(), bob_xch - 1_000);
    assert_eq!(
        alice.wallet.db.xch_balance().await.unwrap(),
        alice_xch + 1_000
    );
    assert_eq!(alice.wallet.db.cat_balance(principal).await.unwrap(), 100);
}

/// A plain offer, no option: XCH for a CAT, the label `offer.create`; no option contracts
/// needed on the connection.
#[tokio::test]
async fn a_plain_xch_for_cat_offer_is_taken() {
    let mut alice = SimWallet::new(5_000).await;
    let mut bob = alice.next(1_000).await;
    let wanted = issue_amount(&mut bob, 700).await;
    know_cats(&alice, &[wanted]).await;
    let now = chain_time(&alice).await;
    let params = offer_params(vec![xch(2_000)], vec![], vec![cat(wanted, 700)]);
    let (run, view) = make(
        &alice,
        &params,
        &[wanted],
        &Setup::new(CHIA_CREATE_OFFER, now),
    )
    .await;
    let offer = the_offer(&run);
    assert_eq!(
        run.prompt.as_ref().unwrap()["intent"]["kind"],
        "offer.create"
    );
    assert_eq!(view.unwrap().expires_at, None);
    taken(&mut bob, &mut alice, &[wanted], &offer).await;
    until(&mut alice, async |w| {
        w.wallet.db.cat_balance(wanted).await.unwrap() == 700
    })
    .await;
    assert_eq!(bob.wallet.db.cat_balance(wanted).await.unwrap(), 0);
    assert_eq!(alice.wallet.db.cat_balance(wanted).await.unwrap(), 700);
}

/// A plain offer with the website's `expiresAt` (what an agent asks for): the approval and
/// the answer show it, and the offer is taken before it.
#[tokio::test]
async fn an_offer_with_expires_at_is_taken_before_it() {
    let mut alice = SimWallet::new(5_000).await;
    let mut bob = alice.next(1_000).await;
    let wanted = issue_amount(&mut bob, 700).await;
    know_cats(&alice, &[wanted]).await;
    let now = chain_time(&alice).await;
    let mut params = offer_params(vec![xch(2_000)], vec![], vec![cat(wanted, 700)]);
    params.expires_at = Some(now + 600);
    let (run, view) = make(
        &alice,
        &params,
        &[wanted],
        &Setup::new(CHIA_CREATE_OFFER, now),
    )
    .await;
    let offer = the_offer(&run);
    assert_eq!(view.unwrap().expires_at, Some(now + 600));
    let answer: Value = serde_json::from_str(run.ok()).unwrap();
    assert_eq!(answer["expiresAt"], now + 600);
    taken(&mut bob, &mut alice, &[wanted], &offer).await;
    until(&mut alice, async |w| {
        w.wallet.db.cat_balance(wanted).await.unwrap() == 700
    })
    .await;
}

/// Past the website's `expiresAt` the chain refuses the take: every spend of the maker's
/// asserts it, not only the approval screen.
#[tokio::test]
async fn an_offer_past_its_expires_at_cannot_be_taken() {
    let alice = SimWallet::new(5_000).await;
    let mut bob = alice.next(1_000).await;
    let wanted = issue_amount(&mut bob, 700).await;
    know_cats(&alice, &[wanted]).await;
    let now = chain_time(&alice).await;
    let mut params = offer_params(vec![xch(2_000)], vec![], vec![cat(wanted, 700)]);
    params.expires_at = Some(now + 600);
    let (run, _) = make(
        &alice,
        &params,
        &[wanted],
        &Setup::new(CHIA_CREATE_OFFER, now),
    )
    .await;
    let offer = the_offer(&run);

    pass_time(&alice, 601).await;
    know_cats(&bob, &[wanted]).await;
    let outcome = take(&bob, &[wanted], &offer, 0, &take_permissions(), true).await;
    let bundle = outcome.sent.expect("nothing sent");
    // Straight to the simulated chain: `push_bundle` only queues it in the wallet. Refused
    // for the expiry, nothing else.
    let refused = alice.sim.lock().await.new_transaction(bundle);
    let error = format!(
        "{:?}",
        refused.expect_err("the chain took an expired offer")
    );
    assert!(error.contains("AssertBeforeSecondsAbsolute"), "{error}");
}

// ---------------------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------------------

/// An `expiresAt` that is already past would make an offer nobody can take: refused before
/// the prompt.
#[tokio::test]
async fn an_expires_at_in_the_past_is_refused() {
    let alice = SimWallet::new(5_000).await;
    let now = chain_time(&alice).await;
    let mut params = offer_params(
        vec![xch(100)],
        vec![],
        vec![cat(Bytes32::new([0x63; 32]), 1)],
    );
    params.expires_at = Some(now);
    let (run, _) = make(&alice, &params, &[], &Setup::new(CHIA_CREATE_OFFER, now)).await;
    assert!(!run.asked());
    assert!(run.reason().contains("expiresAt"), "{}", run.reason());
}

#[tokio::test]
async fn an_option_the_wallet_does_not_hold_is_refused() {
    let alice = SimWallet::new(5_000).await;
    let now = chain_time(&alice).await;
    let params = offer_params(vec![], vec![Bytes32::new([0x42; 32])], vec![xch(10)]);
    let (run, _) = make(&alice, &params, &[], &offer_setup(now)).await;
    assert!(!run.asked());
    assert!(run.reason().contains("unknown_option"), "{}", run.reason());
}

#[tokio::test]
async fn an_expired_option_is_refused_and_a_live_one_needs_option_contracts() {
    let mut alice = SimWallet::new(5_000).await;
    let (launcher_id, expiry) = written(
        &mut alice,
        xch(1_000),
        cat(Bytes32::new([0x59; 32]), 10),
        &[],
    )
    .await;
    let params = offer_params(vec![], vec![launcher_id], vec![xch(10)]);

    // Without the option contracts on the connection: not_granted, before the prompt.
    let now = chain_time(&alice).await;
    let (run, _) = make(&alice, &params, &[], &Setup::new(CHIA_CREATE_OFFER, now)).await;
    assert!(!run.asked());
    assert!(run.reason().contains("not_granted"), "{}", run.reason());

    // Over the day's option limit: 4029, before the prompt.
    let mut setup = offer_setup(now);
    setup.options_today = kit::DEFAULT_OPTIONS_PER_DAY;
    let (run, _) = make(&alice, &params, &[], &setup).await;
    assert!(!run.asked());
    assert!(run.reason().contains("options_per_day"), "{}", run.reason());

    pass_time(&alice, 2 * TERM).await;
    let now = chain_time(&alice).await;
    assert!(now > expiry);
    let (run, _) = make(&alice, &params, &[], &offer_setup(now)).await;
    assert!(!run.asked());
    assert!(run.reason().contains("option_expired"), "{}", run.reason());
}

/// More than the wallet holds, of XCH or of a CAT it has none of: Sage's coin selection
/// fails, which `chia_createOffer` answers as `build_failed`.
#[tokio::test]
async fn more_than_the_wallet_holds_is_not_built() {
    let alice = SimWallet::new(5_000).await;
    let now = chain_time(&alice).await;
    let params = offer_params(
        vec![xch(1_000_000)],
        vec![],
        vec![cat(Bytes32::new([0x61; 32]), 1)],
    );
    assert!(build_offer(&alice.wallet, &params, now).await.is_err());
    let params = offer_params(vec![cat(Bytes32::new([0x62; 32]), 5)], vec![], vec![xch(1)]);
    assert!(build_offer(&alice.wallet, &params, now).await.is_err());
}

#[test]
fn offer_params_are_read_like_sages_and_strictly() {
    let cat_id = "5151515151515151515151515151515151515151515151515151515151515151";
    let option = Address::new(Bytes32::new([0x77; 32]), "option".to_owned())
        .encode()
        .unwrap();
    let good = serde_json::json!({
        "walletId": 1,
        "offerAssets": [
            { "assetId": option, "amount": 1 },
            { "assetId": "", "amount": "300" },
            { "assetId": null, "amount": 200 },
        ],
        "requestAssets": [{ "assetId": cat_id, "amount": "1000" }],
        "fee": 7,
        "expiresAt": "1900000000",
    });
    let params = super::create_offer::offer_params(&good.to_string()).unwrap();
    assert_eq!(params.options, vec![Bytes32::new([0x77; 32])]);
    assert_eq!(params.offered, vec![xch(500)]);
    assert_eq!(params.requested, vec![cat(Bytes32::new([0x51; 32]), 1_000)]);
    // The website's fee is read (and must be well formed), never used.
    assert_eq!(params.fee, crate::execute::own_fee());
    assert_eq!(params.expires_at, Some(1_900_000_000));
    // `0x`, `XCH`, no `assetId`: all read.
    let alt = serde_json::json!({
        "offerAssets": [{ "assetId": "XCH", "amount": 1 }],
        "requestAssets": [{ "assetId": format!("0x{cat_id}"), "amount": 2 }],
    });
    let alt = super::create_offer::offer_params(&alt.to_string()).unwrap();
    // No `expiresAt`: open until taken or cancelled.
    assert_eq!(alt.expires_at, None);

    let nft = Address::new(Bytes32::new([0x78; 32]), "nft".to_owned())
        .encode()
        .unwrap();
    let xch_entry = serde_json::json!({ "amount": 1 });
    let cat_entry = serde_json::json!({ "assetId": cat_id, "amount": 1 });
    let option_entry = |amount: u64| serde_json::json!({ "assetId": option, "amount": amount });
    let two =
        |a: &Value, b: &Value| serde_json::json!({ "offerAssets": [a], "requestAssets": [b] });
    for (bad, why) in [
        (
            serde_json::json!({ "requestAssets": [cat_entry] }),
            "offerAssets",
        ),
        (
            serde_json::json!({ "offerAssets": [], "requestAssets": [cat_entry] }),
            "offerAssets",
        ),
        (
            serde_json::json!({ "offerAssets": [xch_entry] }),
            "requestAssets",
        ),
        (
            two(&serde_json::json!({ "amount": 0 }), &cat_entry),
            "offerAssets",
        ),
        (
            two(
                &serde_json::json!({ "assetId": "zz", "amount": 1 }),
                &cat_entry,
            ),
            "offerAssets",
        ),
        (two(&option_entry(2), &cat_entry), "offerAssets"),
        (
            serde_json::json!({ "offerAssets": [option_entry(1), option_entry(1)], "requestAssets": [cat_entry] }),
            "offerAssets",
        ),
        (two(&cat_entry, &cat_entry), "requestAssets"),
        (two(&xch_entry, &option_entry(1)), "unsupported_asset"),
        (
            two(
                &serde_json::json!({ "assetId": nft, "amount": 1 }),
                &xch_entry,
            ),
            "unsupported_asset",
        ),
        (
            serde_json::json!({ "offerAssets": [xch_entry], "requestAssets": [cat_entry], "fee": -1 }),
            "fee",
        ),
        (
            serde_json::json!({ "offerAssets": vec![xch_entry.clone(); 17], "requestAssets": [cat_entry] }),
            "offerAssets",
        ),
        (
            serde_json::json!({ "offerAssets": [xch_entry], "requestAssets": [cat_entry], "expiresAt": 0 }),
            "expiresAt",
        ),
        (
            serde_json::json!({ "offerAssets": [xch_entry], "requestAssets": [cat_entry], "expiresAt": -5 }),
            "expiresAt",
        ),
        (
            serde_json::json!({ "offerAssets": [xch_entry], "requestAssets": [cat_entry], "expiresAt": "soon" }),
            "expiresAt",
        ),
    ] {
        let error = super::create_offer::offer_params(&bad.to_string()).unwrap_err();
        assert!(error.data.unwrap_or_default().contains(why), "{bad}");
    }
}

#[test]
fn create_offer_is_wallet_built_and_granted_like_a_take() {
    assert!(is_wallet_built(CHIA_CREATE_OFFER));
    assert!(WALLET_BUILT_METHODS.contains(&CHIA_CREATE_OFFER));
    assert!(!super::options::OPTION_METHODS.contains(&CHIA_CREATE_OFFER));
    assert!(
        crate::pairing::granted_methods()
            .iter()
            .any(|m| m == CHIA_CREATE_OFFER)
    );
}
