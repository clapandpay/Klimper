// SPDX-License-Identifier: AGPL-3.0-or-later
//! `chia_takeRoute` (TASK-168): the wallet takes a whole trading route in one bundle, with
//! one approval, all or nothing: whole offers (`offer1…`) together with fills of partial
//! offers (CHIP-0052, `partial1…`), each fill with the amount the user pays.
//!
//! ```text
//! chia_takeRoute { offers?: ["offer1…"], fills?: [{ offer: "partial1…", amount }], fee? }
//!   -> { transactionId, status, expiresAt, fills: [{ paid, bought, nextOffer? }] }
//! ```
//!
//! A fill is an ordinary offer once the wallet has built it (`PartialOfferFill::offer`,
//! docs/partial-offers.md): what the partial coin releases, for the payment to the maker. So
//! a route is every leg joined into one offer (`Offer::extend`) and taken as `chia_takeOffer`
//! takes an aggregate: the user's side built key-free from the wallet's own coins, the sign
//! guard, Sage's summary against wallet-kit's simulation, wallet-kit's submit (sealed: only
//! the transaction id goes back), one approval, one signature.
//!
//! What this path adds to `chia_takeOffer`'s:
//!
//! * **The fill is the wallet's own construction.** The partial coin's spend is built here
//!   with the SDK's driver from the terms the text carries; the terms fix the coin's puzzle
//!   hash, and the coin is looked up on the chain, so a text cannot describe another coin.
//!   The website never writes that spend.
//! * **One unknown puzzle is allowed, by coin.** wallet-kit reads a CAT-wrapped partial coin
//!   as another party's CAT, and a bare one (a partial offer that offers XCH) as an unknown
//!   contract. Exactly the bare partial coins of this route's own fills are let through
//!   ([`allowed_unknown`]); any other unknown puzzle refuses whatever the website's setting,
//!   as for a take. wallet-kit's binding does not depend on that coin: the user's spends
//!   assert the settlement payments, as in any take.
//! * **The route expires.** The user's spends assert a time ([`EXPIRES_AFTER`]): a route
//!   that does not confirm (a leg was taken first, a maker clawed back) cannot be completed
//!   later by whoever saw it.
//! * **A fee per fill is capped and shown.** A partial offer may require a network fee for
//!   each fill, which the user's coins fund; the route's total is at most [`MAX_FILL_FEES`]
//!   and the approval shows it as its own line.
//! * **Fungible assets only.** A leg with an NFT, an option, a royalty or a revocable CAT is
//!   refused by name before anything is built: Sage's take handles those, this build does
//!   not, and wallet-kit cannot bind a payout inside a revocable CAT.
//!
//! A new method rather than a parameter of `chia_takeOffer`, so a Klimper without it
//! refuses the request as an unknown method instead of taking only the whole offers.

use chia_puzzle_types::cat::CatArgs;
use chia_sdk_next::PartialOfferAsset;
use chia_wallet_sdk::driver::{Action, FeeAction, Offer, SpendContext, Spends};
use serde::Serialize;
use tauri_plugin_klimper_partial_offers::{
    Error as PartialError, PartialOffer, is_partial_offer, wallet as partial_wallet,
};

use super::options::{build_failed, sage_view, too_many_failed};
use super::*;

/// The method name. Klimper's own.
pub const CHIA_TAKE_ROUTE: &str = "chia_takeRoute";

/// The most legs (whole offers and fills together) one route may have.
pub const MAX_LEGS: usize = 10;

/// How long after the request a route may still confirm, in seconds. The user's spends
/// assert it, so an unconfirmed route cannot be completed later.
pub const EXPIRES_AFTER: u64 = 300;

/// The most the fees per fill of one route may add up to, in mojos: what Klimper pays at
/// most as its own fee ([`MAX_FEE`]).
pub const MAX_FILL_FEES: u64 = MAX_FEE;

/// One fill a website asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillParam {
    /// The `partial1…` text.
    pub offer: String,
    /// What the user pays into it, in mojos of the asset the partial offer asks for.
    pub amount: u64,
}

/// What `chia_takeRoute` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteParams {
    pub offers: Vec<String>,
    pub fills: Vec<FillParam>,
    /// Klimper's own fee ([`wallet_fee`]); the website's is read and ignored.
    pub fee: u64,
}

fn refusal(code: i64, message: &str, reason: &str) -> RpcError {
    error(code, message, Some(reason))
}

/// A leg that holds something this path does not trade.
fn unsupported_asset() -> RpcError {
    refusal(
        codes::UNSUPPORTED_CONTENT,
        "a route trades XCH and CATs only",
        "unsupported_asset",
    )
}

/// `chia_takeRoute { offers?, fills?, fee? }`, read strictly: at least one leg, at most
/// [`MAX_LEGS`], every text non-empty and of its own kind, every amount positive.
pub fn route_params(params_json: &str) -> Result<RouteParams, RpcError> {
    let params: Value = serde_json::from_str(params_json).map_err(|_| invalid("json"))?;
    let fee = wallet_fee(&params)?;
    let list = |key: &str| match params.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items.clone()),
        Some(_) => Err(invalid(key)),
    };
    let text = |value: Option<&Value>, key: &str, partial: bool| {
        value
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty() && is_partial_offer(text) == partial)
            .map(str::to_owned)
            .ok_or_else(|| invalid(key))
    };
    let offers = list("offers")?
        .iter()
        .map(|item| text(Some(item), "offers", false))
        .collect::<Result<Vec<_>, _>>()?;
    let fills = list("fills")?
        .iter()
        .map(|item| {
            let amount = amount_param(item, "amount")
                .map_err(|_| invalid("fills"))?
                .filter(|amount| *amount > 0)
                .ok_or_else(|| invalid("fills"))?;
            Ok(FillParam {
                offer: text(item.get("offer"), "fills", true)?,
                amount,
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()?;
    let legs = offers.len() + fills.len();
    if legs == 0 {
        return Err(invalid("offers"));
    }
    if legs > MAX_LEGS {
        return Err(invalid("too_many_legs"));
    }
    Ok(RouteParams { offers, fills, fee })
}

// ---------------------------------------------------------------------------------------
// The legs
// ---------------------------------------------------------------------------------------

/// The spends of a leg that come from the website (a whole offer's maker spends, the
/// maker's spends a first fill carries) must be plain: standard XCH coins and CATs that are
/// not revocable. An option, a settlement coin or anything else is refused by name; an
/// unknown puzzle already was ([`check_offer`]).
fn plain_spends(spends: &[CoinSpend], ctx: &RequestContext<'_>) -> Result<(), RpcError> {
    let mut allocator = chia_wallet_sdk::prelude::Allocator::new();
    let (executed, _) = kit::simulate::execute(
        &mut allocator,
        spends,
        ctx.ownership,
        kit::simulate::DEFAULT_MAX_COST,
    )
    .map_err(|_| invalid("offer"))?;
    let plain = executed.iter().all(|spend| {
        matches!(
            spend.kind,
            kit::SpendKind::Standard
                | kit::SpendKind::Cat {
                    hidden_puzzle_hash: None,
                    ..
                }
        )
    });
    if plain {
        Ok(())
    } else {
        Err(unsupported_asset())
    }
}

/// The whole offers of a route, each decoded and checked on its own as `chia_takeOffer`
/// checks an offer (size, no coin twice, asks for something, known puzzles, the maker's
/// signature), and held to plain spends.
pub fn checked_offers(
    texts: &[String],
    ctx: &RequestContext<'_>,
) -> Result<Vec<SpendBundle>, RpcError> {
    texts
        .iter()
        .map(|text| {
            let offer = decode_offer_bundle(text)?;
            check_offer(&offer, ctx)?;
            let makers: Vec<CoinSpend> = offer
                .coin_spends
                .iter()
                .filter(|spend| spend.coin.parent_coin_info != Bytes32::default())
                .cloned()
                .collect();
            plain_spends(&makers, ctx)?;
            Ok(offer)
        })
        .collect()
}

/// One fill, read from its text and from the chain.
#[derive(Debug, Clone)]
pub struct FillLeg {
    /// The partial offer: on the chain already (no maker spends), or a first fill with the
    /// maker's signed spends that create its coin.
    pub offer: PartialOffer,
    /// What the user pays, trimmed to the least that buys as much.
    pub paid: u64,
}

fn revocable(asset: PartialOfferAsset) -> bool {
    matches!(
        asset,
        PartialOfferAsset::Cat {
            hidden_puzzle_hash: Some(_),
            ..
        }
    )
}

/// A CAT partial coin's lineage proof must be its parent's: the proof comes from the text,
/// and with a wrong one the fill could never confirm.
fn lineage_holds(offer: &PartialOffer) -> bool {
    let sdk = offer.offer();
    match (sdk.info.offered_asset, sdk.lineage_proof) {
        (PartialOfferAsset::Xch, None) => true,
        (PartialOfferAsset::Cat { asset_id, .. }, Some(proof)) => {
            let parent_puzzle_hash =
                CatArgs::curry_tree_hash(asset_id, proof.parent_inner_puzzle_hash.into());
            Coin::new(
                proof.parent_parent_coin_info,
                parent_puzzle_hash.into(),
                proof.parent_amount,
            )
            .coin_id()
                == sdk.coin.parent_coin_info
        }
        _ => false,
    }
}

/// The offer changed on the chain since the website read it: a chain fact, not a failed
/// build, and nothing about the wallet.
fn offer_changed() -> RpcError {
    refusal(
        codes::INVALID_PARAMS,
        "this partial offer changed; read it again",
        "offer_changed",
    )
}

/// Read one fill: the text, then one lookup of its coin on the chain. The coin must be the
/// live one (unspent), or not made yet with every coin it is made from unspent; anything
/// else is `offer_changed`. The amount is trimmed and held between the least and the most
/// one fill pays. Refusals come back as `Ok(Err(…))`; an error is the wallet or the network.
pub async fn read_fill(
    ctx: &partial_wallet::WalletCtx,
    param: &FillParam,
    now: u64,
) -> Result<Result<FillLeg, RpcError>, PartialError> {
    let Ok(read) = PartialOffer::from_text(&param.offer) else {
        return Ok(Err(invalid("fills")));
    };
    let info = *read.info();
    if revocable(info.offered_asset) || revocable(info.requested_asset) {
        return Ok(Err(unsupported_asset()));
    }
    if !lineage_holds(&read) {
        return Ok(Err(invalid("fills")));
    }
    // It must outlive the route's own expiry, or the route could never confirm.
    if info
        .expiration
        .is_some_and(|at| at <= now.saturating_add(EXPIRES_AFTER))
    {
        return Ok(Err(refusal(
            codes::INVALID_PARAMS,
            "this partial offer has expired or is about to",
            "offer_expired",
        )));
    }
    if read.maker_spends().coin_spends.len() > MAX_OFFER_SPENDS {
        return Ok(Err(invalid("offer_too_large")));
    }

    let peer = ctx.peer()?;
    let genesis = ctx.wallet.genesis_challenge;
    let offer = match peer
        .fetch_optional_coin(read.coin().coin_id(), genesis)
        .await?
    {
        // On the chain and unspent: the maker's spends are done, and are not sent again.
        Some(state) if state.spent_height.is_none() => read.on_chain(),
        // Not made yet: every coin it is made from must still be unspent.
        None if read.is_first() => {
            for input in read.input_coin_ids() {
                match peer.fetch_optional_coin(input, genesis).await? {
                    Some(state) if state.spent_height.is_none() => {}
                    _ => return Ok(Err(offer_changed())),
                }
            }
            read
        }
        // Spent (filled or clawed back), or a later coin that is not on the chain.
        _ => return Ok(Err(offer_changed())),
    };

    let paid = offer.trim_payment(param.amount);
    if paid < offer.min_fill_requested() {
        return Ok(Err(refusal(
            codes::INVALID_PARAMS,
            "less than the least one fill pays",
            "below_min_fill",
        )));
    }
    if paid > offer.max_fill_requested() {
        return Ok(Err(refusal(
            codes::INVALID_PARAMS,
            "more than this partial offer has left",
            "above_max_fill",
        )));
    }
    Ok(Ok(FillLeg { offer, paid }))
}

/// What is checked about the fills together, and about the maker's spends a first fill
/// carries: no partial offer twice, the fees per fill within [`MAX_FILL_FEES`], and a first
/// fill's maker spends held to what an offer's are (no coin twice, plain puzzles, the
/// maker's signature over them). A fill of a coin on the chain carries no spends and no
/// signature of the maker's: there is nothing of the website's in it.
pub fn check_fills(fills: &[FillLeg], ctx: &RequestContext<'_>) -> Result<(), RpcError> {
    let mut coins = HashSet::with_capacity(fills.len());
    if !fills
        .iter()
        .all(|fill| coins.insert(fill.offer.coin().coin_id()))
    {
        return Err(invalid("leg_twice"));
    }
    let fees = fills.iter().try_fold(0u64, |sum, fill| {
        sum.checked_add(fill.offer.info().required_fee.unwrap_or(0))
    });
    if fees.is_none_or(|fees| fees > MAX_FILL_FEES) {
        return Err(refusal(
            codes::INVALID_PARAMS,
            "the fees these partial offers require are too high",
            "fill_fee_too_high",
        ));
    }
    for fill in fills.iter().filter(|fill| fill.offer.is_first()) {
        let makers = fill.offer.maker_spends();
        let mut spent = HashSet::with_capacity(makers.coin_spends.len());
        if !makers
            .coin_spends
            .iter()
            .all(|spend| spent.insert(spend.coin.coin_id()))
        {
            return Err(invalid("offer_duplicate_coin"));
        }
        plain_spends(&makers.coin_spends, ctx)?;
        check_maker_signature(makers, &ctx.network)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// The build
// ---------------------------------------------------------------------------------------

/// What one fill does, for the answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FillResult {
    /// Paid into the partial offer, in mojos of the asset it asks for.
    pub paid: String,
    /// Released by it, in mojos of the asset it offers.
    pub bought: String,
    /// The partial offer that is left once this route confirms (`partial1…`); none when the
    /// fill took everything. Its coin does not exist before the route has confirmed.
    #[serde(rename = "nextOffer", skip_serializing_if = "Option::is_none")]
    pub next_offer: Option<String>,
}

/// What the approval shows about a route, next to what is paid and received.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RouteView {
    /// Whole offers taken.
    pub offers: usize,
    /// Partial offers filled.
    pub fills: usize,
    /// The network fees the filled partial offers require together, in mojos of XCH: paid by
    /// the user on top of Klimper's own fee, and inside what the route pays or receives.
    #[serde(serialize_with = "as_string")]
    pub fill_fees: u64,
    /// The route cannot confirm at or after this time (Unix seconds).
    pub expires_at: u64,
    /// The puzzle hashes (hex) of the partial coins wallet-kit lists as unknown contracts
    /// and Klimper verified itself: the screen does not warn about these.
    pub verified_partial_puzzles: Vec<String>,
}

fn as_string<S: serde::Serializer, T: ToString>(value: &T, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&value.to_string())
}

/// A route, built and not yet signed.
#[derive(Debug, Clone)]
pub struct BuiltRoute {
    /// Every leg's spends, then the user's (unsigned).
    pub spends: Vec<CoinSpend>,
    /// The legs' signatures summed: the makers' of whole offers and of first fills. The
    /// identity when no leg has one.
    pub legs_signature: Signature,
    /// The coins of the legs' own spends; every other spend is the user's.
    pub leg_coins: HashSet<Bytes32>,
    /// The partial coins' spends as the wallet built them, bare ones (a partial offer that
    /// offers XCH) marked: those are the unknown puzzles let through.
    pub partial_spends: Vec<(CoinSpend, bool)>,
    pub fills: Vec<FillResult>,
    pub view: RouteView,
}

/// Fungible assets only, on both sides of the joined offer.
fn fungible(route: &Offer) -> bool {
    let offered = route.offered_coins();
    let requested = route.requested_payments();
    offered.nfts.is_empty()
        && offered.options.is_empty()
        && requested.nfts.is_empty()
        && requested.options.is_empty()
        && offered
            .cats
            .values()
            .flatten()
            .all(|cat| cat.info.hidden_puzzle_hash.is_none())
        && requested.cats.keys().all(|asset_id| {
            route
                .asset_info()
                .cat(*asset_id)
                .is_none_or(|info| info.hidden_puzzle_hash.is_none())
        })
}

/// Build the route from the wallet's own coins, key-free: every leg joined into one offer
/// (never re-read from the joined spends, which would drop a coin a website's leg already
/// spends), the user's side as Sage's code builds a take (`select_spends_excluding` the
/// legs' coins, `complete_spends`), with the legs' requested payments, each fill's required
/// fee (left unreserved: the partial coin reserves it), Klimper's own fee, and the expiry.
///
/// Refusals come back as `Ok(Err(…))`; Sage's coin selection failing as an error, answered
/// `build_failed`.
pub async fn build_route(
    wallet: &sage_wallet::Wallet,
    offers: &[SpendBundle],
    fills: &[FillLeg],
    fee: u64,
    expires_at: u64,
) -> Result<Result<BuiltRoute, RpcError>, PartialError> {
    let mut ctx = SpendContext::new();
    let mut route: Option<Offer> = None;
    let join = |route: &mut Option<Offer>, leg: Offer| {
        if let Some(joined) = route {
            joined.extend(leg).is_ok()
        } else {
            *route = Some(leg);
            true
        }
    };
    for offer in offers {
        let Ok(leg) = Offer::from_spend_bundle(&mut ctx, offer) else {
            return Ok(Err(invalid("offer")));
        };
        if !join(&mut route, leg) {
            return Ok(Err(invalid("leg_twice")));
        }
    }
    let mut results = Vec::with_capacity(fills.len());
    let mut partial_spends = Vec::with_capacity(fills.len());
    let mut fill_fees = 0u64;
    for fill in fills {
        let filled = fill.offer.bundle().fill(&mut ctx, fill.paid)?;
        let coin = fill.offer.coin();
        let Some(spend) = filled
            .offer
            .spend_bundle()
            .coin_spends
            .iter()
            .find(|spend| spend.coin == coin)
            .cloned()
        else {
            return Ok(Err(invalid("fills")));
        };
        let bare = fill.offer.info().offered_asset == PartialOfferAsset::Xch;
        partial_spends.push((spend, bare));
        fill_fees = fill_fees.saturating_add(filled.required_fee);
        let next_offer = filled
            .child
            .map(|child| PartialOffer::from_coin(child)?.to_text())
            .transpose()?;
        results.push(FillResult {
            paid: filled.requested_amount.to_string(),
            bought: filled.offered_amount.to_string(),
            next_offer,
        });
        if !join(&mut route, filled.offer) {
            return Ok(Err(invalid("leg_twice")));
        }
    }
    let Some(route) = route else {
        return Ok(Err(invalid("offers")));
    };
    if !fungible(&route) {
        return Ok(Err(unsupported_asset()));
    }
    let leg_spends = &route.spend_bundle().coin_spends;
    if leg_spends.len() > MAX_OFFER_SPENDS {
        return Ok(Err(invalid("offer_too_large")));
    }
    let mut leg_coins = HashSet::with_capacity(leg_spends.len());
    if !leg_spends
        .iter()
        .all(|spend| leg_coins.insert(spend.coin.coin_id()))
    {
        return Ok(Err(invalid("leg_twice")));
    }

    let mut actions = route.requested_payments().actions();
    if fill_fees > 0 {
        actions.push(Action::Fee(FeeAction {
            amount: fill_fees,
            reserved: false,
        }));
    }
    if fee > 0 {
        actions.push(Action::fee(fee));
    }
    let mut spends = Spends::new(wallet.change_p2_puzzle_hash().await?);
    spends.add(route.offered_coins().clone());
    let excluded: Vec<Bytes32> = leg_coins.iter().copied().collect();
    wallet
        .select_spends_excluding(&mut ctx, &mut spends, &actions, &excluded)
        .await?;
    spends.conditions.required = spends
        .conditions
        .required
        .assert_before_seconds_absolute(expires_at);
    let deltas = spends.apply(&mut ctx, &actions)?;
    wallet.complete_spends(&mut ctx, &deltas, spends).await?;
    let joined = route.take(SpendBundle::new(ctx.take(), Signature::default()));

    let verified_partial_puzzles = partial_spends
        .iter()
        .filter(|(_, bare)| *bare)
        .map(|(spend, _)| hex::encode(spend.coin.puzzle_hash))
        .collect();
    Ok(Ok(BuiltRoute {
        spends: joined.coin_spends,
        legs_signature: joined.aggregated_signature,
        leg_coins,
        partial_spends,
        fills: results,
        view: RouteView {
            offers: offers.len(),
            fills: fills.len(),
            fill_fees,
            expires_at,
            verified_partial_puzzles,
        },
    }))
}

// ---------------------------------------------------------------------------------------
// The checks and the submit
// ---------------------------------------------------------------------------------------

/// Whether every puzzle wallet-kit does not recognise in the built route is a bare partial
/// coin of one of its own fills, spent with exactly the spend the wallet built: matched by
/// coin, never by puzzle hash alone. `Ok(true)`: there is at least one, so wallet-kit's
/// submit must be told to let unknown contracts through for this request; `Ok(false)`:
/// none. Any other unknown puzzle refuses, whatever the website's setting, as for a take.
fn allowed_unknown(built: &BuiltRoute, summary: &Summary) -> Result<bool, RpcError> {
    let unknown = || {
        error(
            codes::UNSUPPORTED_CONTENT,
            "unknown contract",
            Some("unknown_contract"),
        )
    };
    let bare: Vec<&CoinSpend> = built
        .partial_spends
        .iter()
        .filter(|(_, bare)| *bare)
        .map(|(spend, _)| spend)
        .collect();
    let mut allowed_hashes = HashSet::new();
    for info in &summary.spends {
        if info.kind != kit::SpendKind::Unknown {
            continue;
        }
        // Not the user's, a partial coin of this route, and in the route byte for byte as
        // the wallet built its spend.
        let built_spend = bare
            .iter()
            .find(|spend| hex::encode(spend.coin.coin_id()) == info.coin_id);
        let Some(built_spend) = built_spend.filter(|spend| {
            !info.owned && built.spends.iter().filter(|s| s == *spend).count() == 1
        }) else {
            return Err(unknown());
        };
        allowed_hashes.insert(hex::encode(built_spend.coin.puzzle_hash));
    }
    if !summary
        .unknown_puzzles
        .iter()
        .all(|hash| allowed_hashes.contains(hash))
    {
        return Err(unknown());
    }
    let found = !allowed_hashes.is_empty();
    Ok(found)
}

/// Everything after the build: the guard on the user's spends, wallet-kit's simulation
/// against Sage's summary, the one exception for this route's own bare partial coins, then
/// wallet-kit's submit with the legs' signature (always given, the identity if no leg has
/// one, so the request is read as one with another party and every user spend must be
/// bound), sealed. Answers the website's JSON.
#[allow(clippy::too_many_arguments)]
pub fn submit_route(
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    sage_summary: &TransactionSummary,
    built: BuiltRoute,
    fee: u64,
    on_route: &dyn Fn(&RouteView),
) -> Result<String, RpcError> {
    let user_spends: Vec<CoinSpend> = built
        .spends
        .iter()
        .filter(|spend| !built.leg_coins.contains(&spend.coin.coin_id()))
        .cloned()
        .collect();
    crate::sign_guard::check_spends(&user_spends, ctx.keys, &ctx.network)?;

    let kit_summary = kit::simulate(
        &built.spends,
        ctx.ownership,
        kit::simulate::DEFAULT_MAX_COST,
    )
    .map_err(RpcError::from)?;
    let allow_unknown_contracts = allowed_unknown(&built, &kit_summary)?;
    // Fungible legs only: nothing here may touch an option contract.
    if !kit_summary.options.is_empty() {
        return Err(unsupported_asset());
    }
    cross_check(sage_summary, &kit_summary)?;
    on_route(&built.view);

    // The request's context with the one exception, and nothing else of the website's
    // "unknown contracts" setting: on only when this route holds a bare partial coin the
    // wallet built the spend of, off otherwise even if the user allowed the website more.
    let ctx = RequestContext {
        dapp: ctx.dapp,
        network: ctx.network.clone(),
        session_chain_id: ctx.session_chain_id,
        permissions: ctx.permissions,
        allow_agg_sig_unsafe: ctx.allow_agg_sig_unsafe,
        allow_unknown_contracts,
        ownership: ctx.ownership,
        keys: ctx.keys,
        limits: ctx.limits,
        now: ctx.now,
        chain: ctx.chain,
        signed_coins: ctx.signed_coins,
    };
    let intent = take_intent(false);
    let submitted = kit::submit(
        SubmitRequest {
            method: CHIA_TAKE_ROUTE,
            spends: built.spends,
            other_signature: Some(built.legs_signature),
            intent: Some(&intent),
            wallet_built: true,
            wallet_fee: fee,
            // Sealed: only the transaction id goes back to the website.
            signatures_released: false,
        },
        &ctx,
        signer,
        approver,
        broadcaster,
    )?;
    let mut answer: Value = serde_json::from_str(&submitted.to_json())
        .unwrap_or_else(|_| Value::Object(serde_json::Map::default()));
    if let Some(fields) = answer.as_object_mut() {
        fields.insert(
            "expiresAt".to_owned(),
            serde_json::json!(built.view.expires_at),
        );
        fields.insert("fills".to_owned(), serde_json::json!(built.fills));
    }
    Ok(answer.to_string())
}

// ---------------------------------------------------------------------------------------
// The request
// ---------------------------------------------------------------------------------------

/// `chia_takeRoute`: read and check every leg, build, check, ask once, sign once, broadcast.
///
/// What counts as a failed build (the door that closes on a website probing the balance,
/// `failed_builds`): a whole offer the wallet refuses, as for `chia_takeOffer`, and a build
/// Sage could not fund. What a partial offer is on the chain right now (changed, expired,
/// the least and the most one fill pays) is the chain's news, not the wallet's, and is not
/// counted: a busy partial offer changes under every taker.
pub fn take_route(
    state: &AppState,
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    params_json: &str,
    on_route: &dyn Fn(&RouteView),
) -> Result<String, RpcError> {
    too_many_failed(ctx)?;
    let params = route_params(params_json)?;
    let fee = params.fee;
    let offers = checked_offers(&params.offers, ctx)
        .inspect_err(|_| failed_builds::record(ctx.dapp, ctx.now))?;

    let now = ctx.now;
    let sage = state.clone();
    let asked = params.fills.clone();
    let (wallet, fills) = tauri::async_runtime::block_on(async move {
        let (wallet, _store) = tauri_plugin_klimper_partial_offers::sage_ctx(&sage, false).await?;
        let mut fills = Vec::with_capacity(asked.len());
        for param in &asked {
            match read_fill(&wallet, param, now).await? {
                Ok(fill) => fills.push(fill),
                Err(refused) => return Ok((wallet, Err(refused))),
            }
        }
        Ok::<_, PartialError>((wallet, Ok(fills)))
    })
    .map_err(|e| {
        // The wallet or the network, not the request: no connection, no wallet open.
        tracing::info!("xchonnect: chia_takeRoute could not read its partial offers: {e}");
        refusal(
            codes::INVALID_PARAMS,
            "the wallet could not read these partial offers now",
            "chain_unavailable",
        )
    })?;
    let fills = fills?;
    check_fills(&fills, ctx).inspect_err(|_| failed_builds::record(ctx.dapp, ctx.now))?;

    let expires_at = now.saturating_add(EXPIRES_AFTER);
    let built = tauri::async_runtime::block_on(async move {
        build_route(&wallet.wallet, &offers, &fills, fee, expires_at).await
    })
    .map_err(|e| build_failed(CHIA_TAKE_ROUTE, ctx, &e.to_string()))?
    .inspect_err(|_| failed_builds::record(ctx.dapp, ctx.now))?;

    let summary = sage_view(state, &built.spends)?;
    submit_route(
        ctx,
        signer,
        approver,
        broadcaster,
        &summary,
        built,
        fee,
        on_route,
    )
}
