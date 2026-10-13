// SPDX-License-Identifier: AGPL-3.0-or-later
//! Taking real offers end to end (TASK-137): one wallet makes an offer with Sage's offer
//! code, the other takes it through Klimper's own path — the maker-signature check, the
//! key-free build, the cross-check and wallet-kit's submit — and the completed bundle is
//! sent to the simulated chain and must settle. Every shape that refused a real take before
//! (zero-amount coins, a maker's fee) is in the matrix.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, HashSet},
};

use chia_bls::{PublicKey, SecretKey};
use chia_wallet_sdk::chia::{
    bls::{master_to_wallet_hardened, master_to_wallet_unhardened},
    puzzle_types::DeriveSynthetic,
};
use sage_api::{TransactionInput, TransactionOutput};
use sage_wallet::{CoinKind, Offered, Requested, RequestedCat};
use xchonnect_wallet_kit::permissions::{
    AssetLimit, DailySpend, DappPermissions, LimitStore, PermissionError,
};

use super::sage_sim::SimWallet;
use super::*;

/// What is traded: what the maker offers for what the maker asks.
#[derive(Clone, Copy, Debug)]
enum Trade {
    /// The maker gives 1000 of a CAT and asks 500 mojos: the user pays XCH.
    CatForXch,
    /// The maker gives 500 mojos and asks 1000 of a CAT: the user pays a CAT.
    XchForCat,
    /// The maker gives 1000 of one CAT and asks 1000 of another.
    CatForCat,
}

/// A maker, a taker on the same chain, and the maker's signed offer.
struct World {
    taker: SimWallet,
    offer: SpendBundle,
    /// The CAT the user receives, if any, and the one the user pays, if any.
    gets_cat: Option<Bytes32>,
    pays_cat: Option<Bytes32>,
}

async fn issue(wallet: &mut SimWallet) -> Bytes32 {
    let (spends, asset_id) = wallet.wallet.issue_cat(1_000, 0, None).await.unwrap();
    wallet.transact(spends).await;
    wallet.wait_for_coins().await;
    asset_id
}

async fn world(trade: Trade, maker_fee: u64, taker_xch: u64) -> World {
    let mut maker = SimWallet::new(3_000).await;
    let mut taker = maker.next(taker_xch.max(1)).await;
    let wants = |asset_id| {
        RequestedCat {
            amount: 1_000,
            hidden_puzzle_hash: None,
        }
        .clone_for(asset_id)
    };
    let (offered, requested, gets_cat, pays_cat) = match trade {
        Trade::CatForXch => {
            let a = issue(&mut maker).await;
            (
                Offered {
                    cats: indexmap::indexmap! { a => 1_000 },
                    fee: maker_fee,
                    ..Default::default()
                },
                Requested {
                    xch: 500,
                    ..Default::default()
                },
                Some(a),
                None,
            )
        }
        Trade::XchForCat => {
            let b = issue(&mut taker).await;
            (
                Offered {
                    xch: 500,
                    fee: maker_fee,
                    ..Default::default()
                },
                Requested {
                    cats: wants(b),
                    ..Default::default()
                },
                None,
                Some(b),
            )
        }
        Trade::CatForCat => {
            let a = issue(&mut maker).await;
            let b = issue(&mut taker).await;
            (
                Offered {
                    cats: indexmap::indexmap! { a => 1_000 },
                    fee: maker_fee,
                    ..Default::default()
                },
                Requested {
                    cats: wants(b),
                    ..Default::default()
                },
                Some(a),
                Some(b),
            )
        }
    };
    let offer = maker
        .wallet
        .make_offer(offered, requested, None)
        .await
        .unwrap();
    let offer = maker
        .wallet
        .sign_transaction(offer, &maker.agg_sig, maker.master_sk.clone(), true)
        .await
        .unwrap();
    World {
        taker,
        offer,
        gets_cat,
        pays_cat,
    }
}

trait CloneFor {
    fn clone_for(self, asset_id: Bytes32) -> indexmap::IndexMap<Bytes32, RequestedCat>;
}

impl CloneFor for RequestedCat {
    fn clone_for(self, asset_id: Bytes32) -> indexmap::IndexMap<Bytes32, RequestedCat> {
        indexmap::indexmap! { asset_id => self }
    }
}

/// Sage's `summarize` for what the cross-check reads (XCH and CATs), from the taker's real
/// wallet database: each input's asset and amount, whether each output comes back to the
/// wallet, and the fee. In the app the same summary comes from `view_coin_spends`.
async fn summarise_like_sage(
    wallet: &sage_wallet::Wallet,
    spends: &[CoinSpend],
) -> TransactionSummary {
    let transaction = sage_wallet::Transaction::from_coin_spends(spends.to_vec()).unwrap();
    let mut inputs = Vec::new();
    for input in transaction.inputs {
        let coin = input.coin_spend.coin;
        let asset = match input.kind {
            CoinKind::Unknown if wallet.db.is_p2_puzzle_hash(coin.puzzle_hash).await.unwrap() => {
                Some(xch())
            }
            CoinKind::Cat { info } => Some(sage_api::Asset {
                asset_id: Some(hex::encode(info.asset_id)),
                ..xch()
            }),
            _ => None,
        };
        let mut outputs = Vec::new();
        for output in input.outputs {
            let p2 = output
                .kind
                .receiver_custody_p2_puzzle_hash()
                .unwrap_or(output.coin.puzzle_hash);
            outputs.push(TransactionOutput {
                coin_id: hex::encode(output.coin.coin_id()),
                amount: Amount::u64(output.coin.amount),
                address: String::new(),
                receiving: wallet.db.is_custody_p2_puzzle_hash(p2).await.unwrap(),
                burning: false,
            });
        }
        inputs.push(TransactionInput {
            coin_id: hex::encode(coin.coin_id()),
            amount: Amount::u64(coin.amount),
            address: String::new(),
            asset,
            outputs,
        });
    }
    TransactionSummary {
        fee: Amount::u64(transaction.fee),
        inputs,
    }
}

fn xch() -> sage_api::Asset {
    sage_api::Asset {
        asset_id: None,
        name: Some("Chia".into()),
        ticker: Some("XCH".into()),
        precision: 12,
        icon_url: None,
        description: None,
        is_sensitive_content: false,
        is_visible: true,
        revocation_address: None,
        kind: AssetKind::Token,
    }
}

/// Signs with the taker's keys, as `KlimperSigner` does once the wallet is unlocked.
struct TakerKeys(HashMap<PublicKey, SecretKey>);

impl kit::Signer for TakerKeys {
    fn sign(&self, public_key: &PublicKey, message: &[u8]) -> Result<Signature, kit::SignerError> {
        let secret = self
            .0
            .get(public_key)
            .ok_or(kit::SignerError::KeyUnavailable)?;
        Ok(chia_bls::sign(secret, message))
    }
}

/// Answers the approval and remembers that it was asked: the pop-up opened.
struct User {
    approves: bool,
    asked: Cell<bool>,
}

impl kit::Approver for User {
    fn approve(&self, _prompt: &kit::Prompt<'_>) -> bool {
        self.asked.set(true);
        self.approves
    }
}

/// Keeps the bundle it is handed; the test sends it to the simulated chain itself.
#[derive(Default)]
struct Kept(RefCell<Option<SpendBundle>>);

impl Broadcaster for Kept {
    fn submit(&self, bundle: &SpendBundle) -> Result<Broadcast, String> {
        *self.0.borrow_mut() = Some(bundle.clone());
        Ok(Broadcast {
            transaction_id: hex0x(bundle.name()),
            status: "submitted".to_owned(),
        })
    }
}

#[derive(Default)]
struct Limits(RefCell<DailySpend>);

impl LimitStore for Limits {
    fn load(&self) -> Result<DailySpend, PermissionError> {
        Ok(self.0.borrow().clone())
    }
    fn save(&self, record: &DailySpend) -> Result<(), PermissionError> {
        *self.0.borrow_mut() = record.clone();
        Ok(())
    }
}

/// What a take ended in.
struct Outcome {
    result: Result<String, RpcError>,
    asked: bool,
    sent: Option<SpendBundle>,
}

fn take_permissions() -> DappPermissions {
    DappPermissions {
        methods: vec![CHIA_TAKE_OFFER.to_owned()],
        exposed_keys: vec![],
        limits: BTreeMap::new(),
    }
}

/// Take `offer` with the taker's wallet the way `take_offer` does after reading its
/// params: the maker's signature, the key-free build, Sage's summary, then `submit_take`.
async fn take(
    taker: &SimWallet,
    offer: &SpendBundle,
    fee: u64,
    permissions: &DappPermissions,
    approves: bool,
) -> Outcome {
    let network = kit::Network::Testnet11;
    if let Err(error) = check_maker_signature(offer, &network) {
        return Outcome {
            result: Err(error),
            asked: false,
            sent: None,
        };
    }
    let (spends, maker_signature) = build_take(&taker.wallet, offer.clone(), fee)
        .await
        .expect("Sage could not build the take");

    let db = &taker.wallet.db;
    let mut keys = HashMap::new();
    for hardened in [false, true] {
        let max = db
            .max_derivation_index(hardened)
            .await
            .unwrap()
            .unwrap_or(0);
        for index in 0..=max {
            let secret = if hardened {
                master_to_wallet_hardened(&taker.master_sk, index)
            } else {
                master_to_wallet_unhardened(&taker.master_sk, index)
            }
            .derive_synthetic();
            keys.insert(secret.public_key(), secret);
        }
    }
    let signer = TakerKeys(keys);
    let public_keys: HashSet<PublicKey> = signer.0.keys().copied().collect();
    let ownership = kit::Ownership {
        p2_puzzle_hashes: db
            .custody_p2_puzzle_hashes()
            .await
            .unwrap()
            .into_iter()
            .collect(),
    };
    let spends = match bind_user_spends(spends, &ownership) {
        Ok(spends) => spends,
        Err(error) => {
            return Outcome {
                result: Err(error),
                asked: false,
                sent: None,
            };
        }
    };
    let summary = summarise_like_sage(&taker.wallet, &spends).await;
    let limits = Limits::default();
    let ctx = RequestContext {
        dapp: "pengui.test",
        network,
        session_chain_id: "testnet11",
        permissions,
        allow_agg_sig_unsafe: false,
        allow_unknown_contracts: false,
        ownership: &ownership,
        keys: &public_keys,
        limits: &limits,
        now: 1_760_000_000,
        chain: None,
    };
    let user = User {
        approves,
        asked: Cell::new(false),
    };
    let kept = Kept::default();
    let result = submit_take(
        &ctx,
        &signer,
        &user,
        &kept,
        &summary,
        spends,
        maker_signature,
    );
    Outcome {
        result,
        asked: user.asked.get(),
        sent: kept.0.into_inner(),
    }
}

/// The whole take: the user is asked, the bundle goes out, the chain accepts it, and the
/// taker holds what the offer promised and paid what it asked plus the fee.
async fn takes_and_settles(trade: Trade, maker_fee: u64, taker_fee: u64) {
    let World {
        mut taker,
        offer,
        gets_cat,
        pays_cat,
    } = world(trade, maker_fee, 3_000).await;
    let xch_before = taker.wallet.db.xch_balance().await.unwrap();

    let outcome = take(&taker, &offer, taker_fee, &take_permissions(), true).await;
    let answer = outcome.result.unwrap_or_else(|error| {
        panic!("{trade:?} maker fee {maker_fee} taker fee {taker_fee}: refused: {error:?}")
    });
    assert!(outcome.asked, "the user was never asked");
    assert!(answer.contains("transactionId"), "{answer}");
    let bundle = outcome.sent.expect("nothing was sent");

    taker
        .push_bundle(bundle)
        .await
        .expect("the chain refused the take");
    taker.wait_for_coins().await;

    let paid_xch = if matches!(trade, Trade::CatForXch) {
        500
    } else {
        0
    };
    let got_xch = if matches!(trade, Trade::XchForCat) {
        500
    } else {
        0
    };
    assert_eq!(
        taker.wallet.db.xch_balance().await.unwrap(),
        xch_before + got_xch - paid_xch - u128::from(taker_fee),
        "{trade:?}: XCH after the take"
    );
    if let Some(cat) = gets_cat {
        assert_eq!(
            taker.wallet.db.cat_balance(cat).await.unwrap(),
            1_000,
            "{trade:?}"
        );
    }
    if let Some(cat) = pays_cat {
        assert_eq!(
            taker.wallet.db.cat_balance(cat).await.unwrap(),
            0,
            "{trade:?}"
        );
    }
}

#[tokio::test]
async fn paying_xch_for_a_cat_settles() {
    Box::pin(takes_and_settles(Trade::CatForXch, 0, 0)).await;
}

#[tokio::test]
async fn paying_xch_with_a_fee_settles() {
    Box::pin(takes_and_settles(Trade::CatForXch, 0, 100)).await;
}

#[tokio::test]
async fn an_offer_whose_maker_pays_a_fee_settles() {
    Box::pin(takes_and_settles(Trade::CatForXch, 250, 0)).await;
}

#[tokio::test]
async fn both_fees_settle() {
    Box::pin(takes_and_settles(Trade::CatForXch, 250, 100)).await;
}

#[tokio::test]
async fn paying_a_cat_for_xch_settles() {
    Box::pin(takes_and_settles(Trade::XchForCat, 0, 0)).await;
}

#[tokio::test]
async fn paying_a_cat_for_xch_with_both_fees_settles() {
    Box::pin(takes_and_settles(Trade::XchForCat, 250, 100)).await;
}

#[tokio::test]
async fn a_cat_for_a_cat_settles() {
    Box::pin(takes_and_settles(Trade::CatForCat, 0, 0)).await;
}

#[tokio::test]
async fn a_cat_for_a_cat_with_both_fees_settles() {
    Box::pin(takes_and_settles(Trade::CatForCat, 250, 100)).await;
}

#[tokio::test]
async fn a_take_the_wallet_cannot_pay_is_not_built() {
    // 500 mojos asked, 100 held: Sage cannot build it, so nobody is asked.
    let World { taker, offer, .. } = Box::pin(world(Trade::CatForXch, 0, 100)).await;
    assert!(build_take(&taker.wallet, offer, 0).await.is_err());
}

#[tokio::test]
async fn a_rejected_take_sends_nothing() {
    let World { taker, offer, .. } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    let outcome = take(&taker, &offer, 0, &take_permissions(), false).await;
    assert!(outcome.asked);
    assert_eq!(outcome.result.unwrap_err().code, codes::USER_REJECTED);
    assert!(outcome.sent.is_none());
}

#[tokio::test]
async fn a_website_without_the_permission_is_refused_before_asking() {
    let World { taker, offer, .. } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    let mut permissions = take_permissions();
    permissions.methods.clear();
    let outcome = take(&taker, &offer, 0, &permissions, true).await;
    assert!(!outcome.asked);
    assert_eq!(outcome.result.unwrap_err().code, codes::UNAUTHORIZED);
    assert!(outcome.sent.is_none());
}

#[tokio::test]
async fn a_take_over_the_websites_limit_is_refused_before_asking() {
    let World { taker, offer, .. } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    let mut permissions = take_permissions();
    permissions.limits.insert(
        AssetId::Xch,
        AssetLimit {
            per_request: Some(100),
            per_day: None,
        },
    );
    let outcome = take(&taker, &offer, 0, &permissions, true).await;
    assert!(!outcome.asked);
    assert!(outcome.result.is_err());
    assert!(outcome.sent.is_none());
}

#[tokio::test]
async fn an_offer_with_a_forged_signature_is_refused_before_asking() {
    let World {
        taker, mut offer, ..
    } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    offer.aggregated_signature = chia_bls::sign(&SecretKey::from_seed(&[9; 32]), b"forged");
    let outcome = take(&taker, &offer, 0, &take_permissions(), true).await;
    assert!(!outcome.asked);
    let error = outcome.result.unwrap_err();
    assert_eq!(error.code, codes::INVALID_PARAMS);
    assert!(error.data.unwrap_or_default().contains("offer_signature"));
}

#[tokio::test]
async fn an_offer_without_a_signature_is_refused_before_asking() {
    let World {
        taker, mut offer, ..
    } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    offer.aggregated_signature = Signature::default();
    let outcome = take(&taker, &offer, 0, &take_permissions(), true).await;
    assert!(!outcome.asked);
    assert!(outcome.result.is_err());
}
