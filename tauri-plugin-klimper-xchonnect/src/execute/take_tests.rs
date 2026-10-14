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
use sage_wallet::{Offered, Requested, RequestedCat};
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
    issue_amount(wallet, 1_000).await
}

/// A new CAT of `amount` in `wallet`.
pub(super) async fn issue_amount(wallet: &mut SimWallet, amount: u64) -> Bytes32 {
    let (spends, asset_id) = wallet.wallet.issue_cat(amount, 0, None).await.unwrap();
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

/// Put `cats` in `wallet`'s database, as Sage does once it has looked a CAT up (no network
/// in tests).
pub(super) async fn know_cats(wallet: &SimWallet, cats: &[Bytes32]) {
    for &asset_id in cats {
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
}

/// A real Sage on testnet11 with the taker's wallet open: the summary for the cross-check
/// comes from its own `view_coin_spends`, the call `take_offer` makes. The CATs the take
/// touches are put in the taker's database first, so Sage reads them there and never asks
/// Dexie (no network in tests).
pub(super) async fn sage_summary(
    taker: &SimWallet,
    cats: &[Bytes32],
    spends: &[CoinSpend],
) -> TransactionSummary {
    know_cats(taker, cats).await;
    let dir = tempfile::tempdir().unwrap();
    let mut sage = sage::Sage::new(dir.path(), true);
    "testnet11".clone_into(&mut sage.config.network.default_network);
    // Logged in watch-only: Sage holds the taker's public key and no secret, so this also
    // shows that building and reading a take needs no key.
    let fingerprint = sage
        .keychain
        .add_public_key(&taker.master_sk.public_key())
        .unwrap();
    sage.config.global.fingerprint = Some(fingerprint);
    sage.wallet = Some(taker.wallet.clone());
    sage.view_coin_spends(ViewCoinSpends {
        coin_spends: spends.iter().map(to_json).collect(),
    })
    .await
    .expect("Sage could not summarise the take")
    .summary
}

/// Signs with the taker's keys, as `KlimperSigner` does once the wallet is unlocked.
pub(super) struct TakerKeys(pub HashMap<PublicKey, SecretKey>);

impl kit::Signer for TakerKeys {
    fn sign(&self, public_key: &PublicKey, message: &[u8]) -> Result<Signature, kit::SignerError> {
        let secret = self
            .0
            .get(public_key)
            .ok_or(kit::SignerError::KeyUnavailable)?;
        Ok(chia_bls::sign(secret, message))
    }
}

/// Answers the approval and remembers that it was asked (the pop-up opened) and the trade
/// overview the screen would have shown, computed as `UiApprover` computes it.
pub(super) struct User {
    approves: bool,
    offers: usize,
    asked: Cell<bool>,
    trade: RefCell<Option<crate::trade::TradeOverview>>,
}

impl kit::Approver for User {
    fn approve(&self, prompt: &kit::Prompt<'_>) -> bool {
        self.asked.set(true);
        *self.trade.borrow_mut() = crate::trade::of_prompt(prompt, Some(self.offers));
        self.approves
    }
}

/// Keeps the bundle it is handed; the test sends it to the simulated chain itself.
#[derive(Default)]
pub(super) struct Kept(pub RefCell<Option<SpendBundle>>);

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
pub(super) struct Limits(RefCell<DailySpend>);

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
pub(super) struct Outcome {
    pub result: Result<String, RpcError>,
    pub asked: bool,
    pub sent: Option<SpendBundle>,
    /// What the approval screen showed as the trade, if it was asked.
    pub trade: Option<crate::trade::TradeOverview>,
    /// The loan the screen showed, for a loan the user lends (TASK-140).
    pub loan: Option<super::options::LoanView>,
}

impl Outcome {
    fn refused(error: RpcError) -> Self {
        Outcome {
            result: Err(error),
            asked: false,
            sent: None,
            trade: None,
            loan: None,
        }
    }
}

pub(super) fn take_permissions() -> DappPermissions {
    DappPermissions {
        methods: vec![CHIA_TAKE_OFFER.to_owned()],
        exposed_keys: vec![],
        limits: BTreeMap::new(),
    }
}

/// Every key of a simulated wallet, as `KlimperSigner` holds them once the wallet is unlocked,
/// and the puzzle hashes wallet-kit counts as the wallet's.
pub(super) async fn wallet_keys(
    wallet: &SimWallet,
) -> (TakerKeys, HashSet<PublicKey>, kit::Ownership) {
    let db = &wallet.wallet.db;
    let mut keys = HashMap::new();
    for hardened in [false, true] {
        let max = db
            .max_derivation_index(hardened)
            .await
            .unwrap()
            .unwrap_or(0);
        for index in 0..=max {
            let secret = if hardened {
                master_to_wallet_hardened(&wallet.master_sk, index)
            } else {
                master_to_wallet_unhardened(&wallet.master_sk, index)
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
    (signer, public_keys, ownership)
}

/// Take `offer` with the taker's wallet the way `take_offer` does after reading its
/// params: the offer's checks (size, puzzles, the makers' signature), the key-free build,
/// Sage's summary, then `submit_take`.
pub(super) async fn take(
    taker: &SimWallet,
    cats: &[Bytes32],
    offer: &SpendBundle,
    fee: u64,
    permissions: &DappPermissions,
    approves: bool,
) -> Outcome {
    let network = kit::Network::Testnet11;
    let (signer, public_keys, ownership) = wallet_keys(taker).await;
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
    if let Err(error) = check_offer(offer, &ctx) {
        return Outcome::refused(error);
    }
    let offers = crate::trade::offers_in(offer);
    let (spends, maker_signature) = build_take(&taker.wallet, offer.clone(), fee)
        .await
        .expect("Sage could not build the take");
    let spends = match bind_user_spends(spends, &ownership) {
        Ok(spends) => spends,
        Err(error) => return Outcome::refused(error),
    };
    let summary = sage_summary(taker, cats, &spends).await;
    let user = User {
        approves,
        offers,
        asked: Cell::new(false),
        trade: RefCell::new(None),
    };
    let kept = Kept::default();
    let loan = Cell::new(None);
    let result = submit_take(
        &ctx,
        &signer,
        &user,
        &kept,
        &summary,
        spends,
        &maker_coins(offer),
        maker_signature,
        &|view, _| loan.set(Some(*view)),
    );
    Outcome {
        result,
        asked: user.asked.get(),
        sent: kept.0.into_inner(),
        trade: user.trade.into_inner(),
        loan: loan.get(),
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

    let cats: Vec<Bytes32> = gets_cat.into_iter().chain(pays_cat).collect();
    let outcome = take(&taker, &cats, &offer, taker_fee, &take_permissions(), true).await;
    let answer = outcome.result.unwrap_or_else(|error| {
        panic!("{trade:?} maker fee {maker_fee} taker fee {taker_fee}: refused: {error:?}")
    });
    assert!(outcome.asked, "the user was never asked");
    assert!(answer.contains("transactionId"), "{answer}");
    // The screen's trade overview: one offer, what it asks for what it gives, the user's
    // own fee apart, and a price (TASK-139).
    let overview = outcome.trade.expect("no trade overview");
    assert_eq!(overview.offers, Some(1), "{trade:?}");
    // Sold for XCH, Sage pays the user's fee out of the XCH the offer pays the user: no
    // XCH of the user's moves, so the screen shows the XCH that arrives, already less the
    // fee, and no fee of the user's (TASK-139 audit, H1: a fee another party's coin funds
    // is never the user's, and never shown as received).
    let from_proceeds = matches!(trade, Trade::XchForCat);
    let shown_fee = if from_proceeds { 0 } else { taker_fee };
    assert_eq!(overview.fee, shown_fee, "{trade:?}");
    let (pays, gets) = match trade {
        Trade::CatForXch => (AssetId::Xch, AssetId::Cat(gets_cat.unwrap())),
        Trade::XchForCat => (AssetId::Cat(pays_cat.unwrap()), AssetId::Xch),
        Trade::CatForCat => (
            AssetId::Cat(pays_cat.unwrap()),
            AssetId::Cat(gets_cat.unwrap()),
        ),
    };
    let leg = |asset, amount| crate::trade::Leg { asset, amount };
    let (paid, got) = match trade {
        Trade::CatForXch => (500, 1_000),
        Trade::XchForCat => (1_000, 500 - u128::from(taker_fee)),
        Trade::CatForCat => (1_000, 1_000),
    };
    assert_eq!(overview.pays, [leg(pays, paid)], "{trade:?}");
    assert_eq!(overview.receives, [leg(gets, got)], "{trade:?}");
    assert!(overview.price.is_some(), "{trade:?}");
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
    let World {
        taker,
        offer,
        gets_cat,
        ..
    } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    let cats: Vec<Bytes32> = gets_cat.into_iter().collect();
    let outcome = take(&taker, &cats, &offer, 0, &take_permissions(), false).await;
    assert!(outcome.asked);
    assert_eq!(outcome.result.unwrap_err().code, codes::USER_REJECTED);
    assert!(outcome.sent.is_none());
}

#[tokio::test]
async fn a_website_without_the_permission_is_refused_before_asking() {
    let World {
        taker,
        offer,
        gets_cat,
        ..
    } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    let cats: Vec<Bytes32> = gets_cat.into_iter().collect();
    let mut permissions = take_permissions();
    permissions.methods.clear();
    let outcome = take(&taker, &cats, &offer, 0, &permissions, true).await;
    assert!(!outcome.asked);
    assert_eq!(outcome.result.unwrap_err().code, codes::UNAUTHORIZED);
    assert!(outcome.sent.is_none());
}

#[tokio::test]
async fn a_take_over_the_websites_limit_is_refused_before_asking() {
    let World {
        taker,
        offer,
        gets_cat,
        ..
    } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    let cats: Vec<Bytes32> = gets_cat.into_iter().collect();
    let mut permissions = take_permissions();
    permissions.limits.insert(
        AssetId::Xch,
        AssetLimit {
            per_request: Some(100),
            per_day: None,
        },
    );
    let outcome = take(&taker, &cats, &offer, 0, &permissions, true).await;
    assert!(!outcome.asked);
    assert!(outcome.result.is_err());
    assert!(outcome.sent.is_none());
}

#[tokio::test]
async fn an_offer_with_a_forged_signature_is_refused_before_asking() {
    let World {
        taker,
        mut offer,
        gets_cat,
        ..
    } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    let cats: Vec<Bytes32> = gets_cat.into_iter().collect();
    offer.aggregated_signature = chia_bls::sign(&SecretKey::from_seed(&[9; 32]), b"forged");
    let outcome = take(&taker, &cats, &offer, 0, &take_permissions(), true).await;
    assert!(!outcome.asked);
    let error = outcome.result.unwrap_err();
    assert_eq!(error.code, codes::INVALID_PARAMS);
    assert!(error.data.unwrap_or_default().contains("offer_signature"));
}

#[tokio::test]
async fn an_offer_without_a_signature_is_refused_before_asking() {
    let World {
        taker,
        mut offer,
        gets_cat,
        ..
    } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    let cats: Vec<Bytes32> = gets_cat.into_iter().collect();
    offer.aggregated_signature = Signature::default();
    let outcome = take(&taker, &cats, &offer, 0, &take_permissions(), true).await;
    assert!(!outcome.asked);
    assert!(outcome.result.is_err());
}

/// `check_offer` on its own, with an empty wallet: it reads only the offer.
pub(super) fn checked(offer: &SpendBundle) -> Result<(), RpcError> {
    let permissions = take_permissions();
    let ownership = kit::Ownership::default();
    let keys = HashSet::new();
    let limits = Limits::default();
    let ctx = RequestContext {
        dapp: "pengui.test",
        network: kit::Network::Testnet11,
        session_chain_id: "testnet11",
        permissions: &permissions,
        allow_agg_sig_unsafe: false,
        allow_unknown_contracts: true,
        ownership: &ownership,
        keys: &keys,
        limits: &limits,
        now: 1_760_000_000,
        chain: None,
    };
    check_offer(offer, &ctx)
}

pub(super) fn reason(error: &RpcError) -> String {
    error.data.clone().unwrap_or_default()
}

#[tokio::test]
async fn a_real_offer_passes_the_checks() {
    let World { offer, .. } = Box::pin(world(Trade::XchForCat, 250, 3_000)).await;
    checked(&offer).unwrap();
}

#[tokio::test]
async fn an_offer_that_asks_for_nothing_is_refused_before_building() {
    let World { mut offer, .. } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    // The maker's spends alone, still correctly signed: a gift.
    offer
        .coin_spends
        .retain(|spend| spend.coin.parent_coin_info != Bytes32::default());
    let error = checked(&offer).unwrap_err();
    assert_eq!(error.code, codes::UNSUPPORTED_CONTENT);
    assert!(reason(&error).contains("nothing_requested"));
}

#[tokio::test]
async fn an_oversized_offer_is_refused_before_anything_runs() {
    let World { mut offer, .. } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    let first = offer.coin_spends[0].clone();
    offer
        .coin_spends
        .extend(std::iter::repeat_n(first, MAX_OFFER_SPENDS));
    let error = checked(&offer).unwrap_err();
    assert_eq!(error.code, codes::INVALID_PARAMS);
    assert!(reason(&error).contains("offer_too_large"));
}

#[tokio::test]
async fn an_unknown_contract_on_the_makers_side_is_refused_whatever_the_setting() {
    let World { mut offer, .. } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    // `1`: a puzzle that returns its solution, which wallet-kit does not recognise.
    let identity = Bytes32::from(chia_wallet_sdk::prelude::tree_hash_atom(&[1]));
    offer.coin_spends.push(CoinSpend::new(
        Coin::new(Bytes32::new([7; 32]), identity, 1),
        Program::from(vec![0x01]),
        Program::from(vec![0x80]),
    ));
    // `checked` allows unknown contracts for the website; a take refuses them anyway.
    let error = checked(&offer).unwrap_err();
    assert_eq!(error.code, codes::UNSUPPORTED_CONTENT);
    assert!(reason(&error).contains("unknown_contract"));
}

#[tokio::test]
async fn the_maker_signature_is_checked_for_the_requests_network() {
    // A testnet offer's signature commits to testnet's AGG_SIG data: under mainnet's it
    // must not verify, or a signature could be replayed across networks.
    let World { offer, .. } = Box::pin(world(Trade::CatForXch, 0, 3_000)).await;
    check_maker_signature(&offer, &kit::Network::Testnet11).unwrap();
    let error = check_maker_signature(&offer, &kit::Network::Mainnet).unwrap_err();
    assert!(reason(&error).contains("offer_signature"));
}

/// What Sage builds for `chia_send` — XCH and a CAT, each with a fee — is signed with the
/// wallet's keys, and every one of those signatures fixes its spend's conditions, so the
/// guard of TASK-141 lets it through. (The takes above run through it in `submit_take`.)
#[tokio::test]
async fn sages_sends_fix_their_conditions() {
    let mut wallet = SimWallet::new(3_000).await;
    let asset_id = issue(&mut wallet).await;
    let keys: HashSet<PublicKey> = wallet_keys(&wallet).await.1;
    let to = Bytes32::new([9; 32]);
    let xch = wallet
        .wallet
        .send_xch(vec![(to, 500)], 100, vec![], None)
        .await
        .unwrap();
    let cat = wallet
        .wallet
        .send_cat(asset_id, vec![(to, 400)], 100, true, vec![], None)
        .await
        .unwrap();
    for spends in [xch, cat] {
        // The spends are signed with the wallet's keys: the check is not vacuous.
        let mut allocator = chia_wallet_sdk::prelude::Allocator::new();
        let (executed, _) = kit::simulate::execute(
            &mut allocator,
            &spends,
            &kit::Ownership::default(),
            kit::simulate::DEFAULT_MAX_COST,
        )
        .unwrap();
        assert!(
            executed
                .iter()
                .flat_map(|spend| &spend.conditions)
                .any(|condition| {
                    condition
                        .clone()
                        .into_agg_sig()
                        .is_some_and(|agg| keys.contains(&agg.public_key))
                })
        );
        crate::sign_guard::check_spends(&spends, &keys, &kit::Network::Testnet11).unwrap();
    }
}

/// Put `key`'s standard puzzle into the maker's standard spend, where its signature does not
/// reach: as the (unused) solution of the quoted delegated puzzle. The maker's signature
/// still verifies.
fn tuck_standard_puzzle(offer: &mut SpendBundle, key: PublicKey) {
    use chia_puzzle_types::standard::StandardSolution;
    use chia_wallet_sdk::driver::{Layer, Puzzle, SpendContext, StandardLayer};
    use chia_wallet_sdk::prelude::{FromClvm, NodePtr};

    let mut ctx = SpendContext::new();
    let spend = offer
        .coin_spends
        .iter_mut()
        .filter(|spend| spend.coin.parent_coin_info != Bytes32::default())
        .find(|spend| {
            let puzzle = ctx.alloc(&spend.puzzle_reveal).unwrap();
            let puzzle = Puzzle::parse(&ctx, puzzle);
            matches!(StandardLayer::parse_puzzle(&ctx, puzzle), Ok(Some(_)))
        })
        .expect("the maker spends a standard coin");
    let solution = ctx.alloc(&spend.solution).unwrap();
    let mut solution = StandardSolution::<NodePtr, NodePtr>::from_clvm(&ctx, solution).unwrap();
    solution.solution = StandardLayer::new(key).construct_puzzle(&mut ctx).unwrap();
    let solution = ctx.alloc(&solution).unwrap();
    spend.solution = ctx.serialize(&solution).unwrap();
}

/// The offer's own spends are the website's, not the wallet's: one of the user's standard
/// puzzles inside one of them is answered exactly as any other key there would be, so a take
/// says nothing about which keys the wallet holds.
#[tokio::test]
async fn a_makers_spend_holding_a_users_key_is_answered_as_any_other() {
    let mut answers = Vec::new();
    for users_key in [true, false] {
        let World {
            taker,
            mut offer,
            gets_cat,
            pays_cat,
        } = Box::pin(world(Trade::XchForCat, 0, 3_000)).await;
        let key = if users_key {
            *wallet_keys(&taker).await.1.iter().next().unwrap()
        } else {
            SecretKey::from_seed(&[8; 32]).public_key()
        };
        tuck_standard_puzzle(&mut offer, key);
        check_maker_signature(&offer, &kit::Network::Testnet11).unwrap();
        let cats: Vec<Bytes32> = gets_cat.into_iter().chain(pays_cat).collect();
        let outcome = take(&taker, &cats, &offer, 0, &take_permissions(), false).await;
        answers.push((
            outcome.asked,
            outcome.result.map_err(|error| (error.code, error.data)),
            outcome.sent.is_none(),
        ));
    }
    assert_eq!(answers[0], answers[1]);
    assert!(answers[0].0, "the user was asked, as for any take");
}
