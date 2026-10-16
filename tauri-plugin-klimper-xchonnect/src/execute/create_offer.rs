// SPDX-License-Identifier: AGPL-3.0-or-later
//! `chia_createOffer` (TASK-142): the wallet makes and signs an offer, Sage's `WalletConnect`
//! method with its parameters and answer, and nothing is broadcast.
//!
//! What it is for: an **open loan offer**. A lender writes an option (`chia_mintOption`: the
//! collateral the lender locks, the strike the borrower repays, the expiry the due date) and
//! then offers that option together with the principal, asking for the collateral. Whoever
//! takes the offer (`chia_takeOffer`) holds the option and the principal; the maker receives
//! the collateral and, until the expiry, may be repaid the strike for it. Plain offers of XCH
//! and CATs for XCH and CATs are made the same way.
//!
//! The offer is built from the wallet's own coins with Sage's offer code: the same coin
//! selection, settlement payments, requested payments and nonce as Sage's `make_offer`
//! (`sage_wallet::Wallet::make_offer`), key-free. One difference, and it is the reason this
//! is not Sage's call itself: the offer's signatures go to the website, so they are
//! **released** (xchonnect spec 11.2) and only direct assertions bind them. Sage gives the
//! assertion of what the maker is paid to one spend and links the others to it by
//! `ASSERT_CONCURRENT_SPEND`; here **every** spend of the maker's asserts every requested
//! payment (and, with an option, the option's expiry), so no spend of the offer can be
//! used without the maker being paid. The offer then runs through the same path as every
//! other request Klimper builds: the guard on the signed conditions, Sage's summary against
//! wallet-kit's simulation, wallet-kit's submit with an intent, limits, one approval, one
//! signature.

use chia_wallet_sdk::{
    chia::puzzle_types::offer::{NotarizedPayment, Payment},
    driver::{
        Action, AssetInfo, CatAssetInfo, Id, Offer, OptionType, RequestedPayments, SpendContext,
        SpendKind, Spends,
    },
    prelude::{Conditions, Memos},
    puzzles::SETTLEMENT_PAYMENT_HASH,
};
use sage_database::CoinKind;
use sage_wallet::{Wallet, WalletError};
use serde::Serialize;

use super::options::{
    CHIA_TRANSFER_OPTION, OfferOnly, Part, Refusal, build_failed, checked_submit, hex_bytes,
    sage_view, too_many_failed,
};
use super::*;

/// The method name, as Sage's `WalletConnect` handler knows it.
pub const CHIA_CREATE_OFFER: &str = "chia_createOffer";

/// The most entries `offerAssets` and `requestAssets` may each hold: real offers name a few
/// assets; this only bounds what a website can make the wallet look up before it asks.
const MAX_ASSETS: usize = 16;

/// What `chia_createOffer` asks for, every asset once (amounts of one asset named twice are
/// added up, as Sage's `make_offer` does).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferParams {
    /// XCH and CATs given.
    pub offered: Vec<Part>,
    /// Options given, by launcher id.
    pub options: Vec<Bytes32>,
    /// XCH and CATs asked for.
    pub requested: Vec<Part>,
    /// Klimper's own fee ([`wallet_fee`]); the website's is read and ignored.
    pub fee: u64,
}

/// An asset as an entry of `offerAssets` / `requestAssets` names it.
enum Named {
    Fungible(AssetId),
    Option(Bytes32),
}

fn unsupported(what: &str) -> RpcError {
    error(
        codes::UNSUPPORTED_CONTENT,
        &format!("{what} cannot be offered or asked for here"),
        Some("unsupported_asset"),
    )
}

/// `assetId` as Sage's `make_offer` reads it: absent, `""` or `"xch"` is XCH; a 32-byte hex
/// id (with or without `0x`) a CAT; an `option1…` id (bech32m, Sage's option id) an option.
/// An NFT (`nft1…`) or anything else bech32m-shaped is refused as not supported here.
fn named(value: Option<&Value>, key: &str) -> Result<Named, RpcError> {
    match value {
        None | Some(Value::Null) => Ok(Named::Fungible(AssetId::Xch)),
        Some(Value::String(s)) if s.is_empty() || s.eq_ignore_ascii_case("xch") => {
            Ok(Named::Fungible(AssetId::Xch))
        }
        Some(Value::String(s)) => {
            if let Some(bytes) = unhex(s)
                && let Ok(id) = Bytes32::try_from(bytes.as_slice())
            {
                return Ok(Named::Fungible(AssetId::Cat(id)));
            }
            match chia_wallet_sdk::utils::Address::decode(s) {
                Ok(address) if address.prefix == "option" => Ok(Named::Option(address.puzzle_hash)),
                Ok(address) if address.prefix == "nft" => Err(unsupported("an NFT")),
                _ => Err(invalid(key)),
            }
        }
        Some(_) => Err(invalid(key)),
    }
}

/// One side of the offer: `[{ assetId?, amount }]`, at least one entry, every amount
/// positive, an option's amount 1.
fn side(params: &Value, key: &str) -> Result<(Vec<Part>, Vec<Bytes32>), RpcError> {
    let entries = params
        .get(key)
        .and_then(Value::as_array)
        .filter(|entries| !entries.is_empty() && entries.len() <= MAX_ASSETS)
        .ok_or_else(|| invalid(key))?;
    let mut fungible: BTreeMap<AssetId, u64> = BTreeMap::new();
    let mut options: Vec<Bytes32> = Vec::new();
    for entry in entries {
        if !entry.is_object() {
            return Err(invalid(key));
        }
        let amount = amount_param(entry, "amount")
            .map_err(|_| invalid(key))?
            .filter(|amount| *amount > 0)
            .ok_or_else(|| invalid(key))?;
        match named(entry.get("assetId"), key)? {
            Named::Fungible(asset) => {
                let total = fungible.entry(asset).or_default();
                *total = total.checked_add(amount).ok_or_else(|| invalid(key))?;
            }
            Named::Option(launcher_id) => {
                // One option is one singleton: amount 1, named once.
                if amount != 1 || options.contains(&launcher_id) {
                    return Err(invalid(key));
                }
                options.push(launcher_id);
            }
        }
    }
    let parts = fungible
        .into_iter()
        .map(|(asset, amount)| Part { asset, amount })
        .collect();
    Ok((parts, options))
}

/// `chia_createOffer { offerAssets, requestAssets, fee? }`, read strictly. Other keys
/// (Pengui sends `walletId`) are ignored, as Sage's handler ignores them.
pub fn offer_params(params_json: &str) -> Result<OfferParams, RpcError> {
    let params: Value = serde_json::from_str(params_json).map_err(|_| invalid("json"))?;
    let (offered, options) = side(&params, "offerAssets")?;
    let (requested, requested_options) = side(&params, "requestAssets")?;
    // Asking for an option needs its terms from the chain before the user is asked, and
    // no website needs it yet.
    if !requested_options.is_empty() {
        return Err(unsupported("an option"));
    }
    // Giving and asking for the same asset is a slip, never a trade anyone means.
    if requested
        .iter()
        .any(|asked| offered.iter().any(|given| given.asset == asked.asset))
    {
        return Err(invalid("requestAssets"));
    }
    Ok(OfferParams {
        offered,
        options,
        requested,
        fee: wallet_fee(&params)?,
    })
}

// ---------------------------------------------------------------------------------------
// What the approval shows
// ---------------------------------------------------------------------------------------

/// An option's terms, as the wallet database holds them for an option the wallet holds,
/// in wallet-kit's `terms` shape (`summary.options[].terms`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OfferedTerms {
    #[serde(serialize_with = "hex_bytes")]
    pub creator_puzzle_hash: Bytes32,
    pub expiration_seconds: u64,
    pub collateral: kit::option::OptionLeg,
    pub strike: kit::option::OptionLeg,
}

/// One option the offer gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OfferedOption {
    #[serde(serialize_with = "hex_bytes")]
    pub launcher_id: Bytes32,
    pub terms: OfferedTerms,
}

/// Klimper's own key `offer` on the approval of `chia_createOffer`, next to `trade`: what
/// the offer gives and what it asks, from the wallet's own build (the requested payments are
/// the ones every spend of the user's asserts), nothing from the website. Shown with
/// `makes_offer`: nothing is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfferView {
    /// XCH and CATs given.
    pub gives: Vec<crate::trade::Leg>,
    /// Options given, with their terms.
    pub options: Vec<OfferedOption>,
    /// What whoever takes the offer pays the user.
    pub asks: Vec<crate::trade::Leg>,
    /// The offer cannot be taken from this time on (Unix seconds): the earliest expiry of
    /// the options it gives, so nobody can buy an option that can no longer be used.
    pub expires_at: Option<u64>,
}

fn leg(asset: AssetId, amount: u64) -> kit::option::OptionLeg {
    kit::option::OptionLeg { asset, amount }
}

// ---------------------------------------------------------------------------------------
// The build
// ---------------------------------------------------------------------------------------

/// The offer, built and not yet signed.
#[derive(Debug, Clone)]
pub struct BuiltOffer {
    /// The user's spends, the ones the user signs.
    pub spends: Vec<CoinSpend>,
    /// The whole offer as it goes out: the user's spends and the placeholders of what it
    /// asks for, completed by the user's signature.
    pub offer_spends: Vec<CoinSpend>,
    pub view: OfferView,
}

/// The terms of an option the wallet holds and may give, or why not: it holds none with
/// this id (`unknown_option`), or it can no longer be used (`option_expired`).
async fn offered_option(
    wallet: &Wallet,
    launcher_id: Bytes32,
    now: u64,
) -> Result<Result<OfferedOption, RpcError>, WalletError> {
    let unknown = || Ok(Err(Refusal::UnknownOption.rpc()));
    if wallet.db.spendable_option(launcher_id).await?.is_none() {
        return unknown();
    }
    let Some(underlying) = wallet.db.option_underlying(launcher_id).await? else {
        return unknown();
    };
    if now >= underlying.seconds {
        return Ok(Err(Refusal::Expired.rpc()));
    }
    let strike = match underlying.strike_type {
        OptionType::Xch { amount } => leg(AssetId::Xch, amount),
        OptionType::Cat { asset_id, amount } => leg(AssetId::Cat(asset_id), amount),
        // A revocable CAT or an NFT strike is not one the approval can show.
        _ => return Ok(Err(unsupported("this option"))),
    };
    let collateral = match wallet.db.underlying_coin_kind(launcher_id).await? {
        Some(CoinKind::Xch) => AssetId::Xch,
        Some(CoinKind::Cat) => {
            let Some(info) = wallet.db.offer_option_info(launcher_id).await? else {
                return unknown();
            };
            let Some(cat) = wallet.db.cat_coin(info.underlying_coin_hash).await? else {
                return unknown();
            };
            if cat.info.hidden_puzzle_hash.is_some() {
                return Ok(Err(unsupported("this option")));
            }
            AssetId::Cat(cat.info.asset_id)
        }
        _ => return unknown(),
    };
    Ok(Ok(OfferedOption {
        launcher_id,
        terms: OfferedTerms {
            creator_puzzle_hash: underlying.creator_puzzle_hash,
            expiration_seconds: underlying.seconds,
            collateral: leg(collateral, underlying.amount),
            strike,
        },
    }))
}

/// Add `conditions` to every spend of the user's own coins in `spends` (XCH, CATs and
/// options), before Sage's `complete_spends` turns them into coin spends.
fn add_to_every_spend(spends: &mut Spends, conditions: &Conditions) {
    let xch = spends.xch.items.iter_mut().map(|item| &mut item.kind);
    let cats = spends
        .cats
        .values_mut()
        .flat_map(|cat| cat.items.iter_mut().map(|item| &mut item.kind));
    let options = spends
        .options
        .values_mut()
        .flat_map(|option| option.lineage.iter_mut().map(|item| &mut item.kind));
    for kind in xch.chain(cats).chain(options) {
        if let SpendKind::Conditions(spend) = kind {
            spend.add_conditions(conditions.clone());
        }
    }
}

/// Make the offer from the wallet's own coins, key-free, with Sage's offer code (see the
/// module): the offered assets paid to the settlement puzzle, the fee, the requested
/// payments to the wallet's change address under the nonce of the coins it spends, and
/// every spend asserting every requested payment and the expiry.
///
/// Sage's coin selection leaves out the coins of a transaction Sage holds as pending. An
/// asset the wallet does not hold enough of fails here (`build_failed`).
pub async fn build_offer(
    wallet: &Wallet,
    params: &OfferParams,
    now: u64,
) -> Result<Result<BuiltOffer, RpcError>, WalletError> {
    let mut options = Vec::with_capacity(params.options.len());
    for launcher_id in &params.options {
        match offered_option(wallet, *launcher_id, now).await? {
            Ok(option) => options.push(option),
            Err(refusal) => return Ok(Err(refusal)),
        }
    }
    let expires_at = options.iter().map(|o| o.terms.expiration_seconds).min();

    let mut ctx = SpendContext::new();
    let receive = wallet.change_p2_puzzle_hash().await?;
    let settlement = Bytes32::from(SETTLEMENT_PAYMENT_HASH);
    let mut actions = vec![Action::fee(params.fee)];
    for part in &params.offered {
        let id = match part.asset {
            AssetId::Xch => Id::Xch,
            AssetId::Cat(asset_id) => Id::Existing(asset_id),
            _ => return Ok(Err(invalid("offerAssets"))),
        };
        actions.push(Action::send(id, settlement, part.amount, Memos::None));
    }
    for launcher_id in &params.options {
        actions.push(Action::send(
            Id::Existing(*launcher_id),
            settlement,
            1,
            Memos::None,
        ));
    }
    let mut spends = wallet.prepare_spends_for_selection(&mut ctx, &[]).await?;
    wallet
        .select_spends(&mut ctx, &mut spends, &actions)
        .await?;
    let nonce = Offer::nonce(spends.non_settlement_coin_ids());

    // What whoever takes the offer pays the wallet, as Sage's `make_offer` asks it.
    let hint = ctx.hint(receive)?;
    let mut requested = RequestedPayments::new();
    let mut asset_info = AssetInfo::new();
    for part in &params.requested {
        let payment = NotarizedPayment::new(nonce, vec![Payment::new(receive, part.amount, hint)]);
        match part.asset {
            AssetId::Xch => requested.xch.push(payment),
            AssetId::Cat(asset_id) => {
                let hidden = wallet.fetch_offer_cat_hidden_puzzle_hash(asset_id).await?;
                asset_info.insert_cat(asset_id, CatAssetInfo::new(hidden))?;
                requested.cats.entry(asset_id).or_default().push(payment);
            }
            _ => return Ok(Err(invalid("requestAssets"))),
        }
    }
    let mut asserted = Conditions::new().extend(requested.assertions(&mut ctx, &asset_info)?);
    if let Some(at) = expires_at {
        asserted = asserted.assert_before_seconds_absolute(at);
    }
    let deltas = spends.apply(&mut ctx, &actions)?;
    add_to_every_spend(&mut spends, &asserted);
    wallet.complete_spends(&mut ctx, &deltas, spends).await?;
    let user_spends = ctx.take();
    let offer = Offer::from_input_spend_bundle(
        &mut ctx,
        SpendBundle::new(user_spends.clone(), Signature::default()),
        requested,
        asset_info,
    )?;
    let offer_spends = offer.to_spend_bundle(&mut ctx)?.coin_spends;

    let legs = |parts: &[Part]| -> Vec<crate::trade::Leg> {
        parts
            .iter()
            .map(|p| crate::trade::Leg {
                asset: p.asset,
                amount: u128::from(p.amount),
            })
            .collect()
    };
    Ok(Ok(BuiltOffer {
        spends: user_spends,
        offer_spends,
        view: OfferView {
            gives: legs(&params.offered),
            options,
            asks: legs(&params.requested),
            expires_at,
        },
    }))
}

// ---------------------------------------------------------------------------------------
// The request
// ---------------------------------------------------------------------------------------

/// `chia_createOffer`: build, check, ask once, sign once; the signed offer goes back to the
/// website as `{ offer, id }`, as Sage's `WalletConnect` answers it. Nothing is broadcast.
///
/// Giving an option needs the connection's option contracts too (the option leaves the
/// wallet like a `chia_transferOption`), or the request is refused as `not_granted`.
pub fn create_offer(
    state: &AppState,
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    params_json: &str,
    on_offer: &dyn Fn(&OfferView),
) -> Result<String, RpcError> {
    too_many_failed(ctx)?;
    let params = offer_params(params_json)?;
    offer_granted(&params, ctx.permissions)?;
    let now = ctx.now;
    let fee = params.fee;
    let built = on_sage(state, async move |sage| {
        Ok(build_offer(&*sage.wallet()?, &params, now).await?)
    })
    .map_err(|e| build_failed(CHIA_CREATE_OFFER, ctx, &e))?
    .inspect_err(|_| failed_builds::record(ctx.dapp, ctx.now))?;
    let summary = sage_view(state, &built.spends)?;
    submit_offer(ctx, signer, approver, &summary, built, fee, on_offer)
}

/// Giving an option needs the connection's option contracts (the option leaves the wallet
/// like a `chia_transferOption`); `chia_createOffer` itself was checked by the caller.
pub fn offer_granted(
    params: &OfferParams,
    permissions: &kit::permissions::DappPermissions,
) -> Result<(), RpcError> {
    if !params.options.is_empty() && !permissions.allows_method(CHIA_TRANSFER_OPTION) {
        return Err(error(
            codes::UNAUTHORIZED,
            "this connection may not offer option contracts; allow them in Klimper",
            Some("not_granted"),
        ));
    }
    Ok(())
}

/// What an offer claims: with options, `option.transfer`, which wallet-kit verifies (each
/// option leaves the user, here to the settlement puzzle, whoever takes the offer); without,
/// `offer.create`, a label. The fee is claimed: the offer's own spends are the whole
/// request, so its fee is the user's. Who is paid is not claimed: what the user gives goes
/// to the settlement puzzle, and the approval shows it from the simulation.
fn offer_intent(options: bool, fee: u64) -> Intent {
    Intent {
        kind: Some(
            if options {
                "option.transfer"
            } else {
                "offer.create"
            }
            .to_owned(),
        ),
        recipients: None,
        fee: Some(fee),
        net: None,
    }
}

/// After the build: the guard on the signed conditions, the cross-check, wallet-kit's submit
/// with released signatures (the offer goes to the website), and the signed offer.
pub fn submit_offer(
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    sage_summary: &TransactionSummary,
    built: BuiltOffer,
    fee: u64,
    on_offer: &dyn Fn(&OfferView),
) -> Result<String, RpcError> {
    crate::sign_guard::check_spends(&built.spends, ctx.keys, &ctx.network)?;
    on_offer(&built.view);
    let intent = offer_intent(!built.view.options.is_empty(), fee);
    let submitted = checked_submit(
        ctx,
        signer,
        approver,
        &OfferOnly,
        sage_summary,
        CHIA_CREATE_OFFER,
        built.spends,
        &intent,
        // The signed spends go to the website as an offer: released (spec 11.2).
        true,
    )?;
    let offer = SpendBundle::new(built.offer_spends, submitted.bundle.aggregated_signature);
    let id = hex::encode(sage_wallet::sort_offer(offer.clone()).name());
    let offer = chia_wallet_sdk::driver::encode_offer(&offer).map_err(|_| invalid("offer"))?;
    Ok(serde_json::json!({ "offer": offer, "id": id }).to_string())
}
