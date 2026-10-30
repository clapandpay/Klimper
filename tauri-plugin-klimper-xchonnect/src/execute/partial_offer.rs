// SPDX-License-Identifier: AGPL-3.0-or-later
//! `chia_createPartialOffer` (TASK-165): the wallet makes a partial offer (CHIP-0052,
//! `partial1…`) for a website, and answers only its text. Nothing is broadcast.
//!
//! A partial offer starts from a one-sided offer: the offered asset and one mojo of XCH, for
//! nothing. Handed to a website, that offer could be taken by anyone, so it never leaves the
//! wallet: Klimper makes it, signs it and turns it into the first partial coin here
//! (`chia_partial_offer::make_partial_offer`), and only the partial offer goes back. Its
//! security coin's spend asserts that the settlement coins pay into the partial coin, and its
//! signature is summed with the user's, so the user's spends in the text cannot be used any
//! other way (docs/partial-offers.md).
//!
//! The path is `chia_createOffer`'s (`create_offer.rs`): the one-sided offer is built from
//! the wallet's own coins with Sage's offer code (`make_offer`, as Klimper's own Make partial
//! offer screen builds it, `tauri-plugin-klimper-partial-offers`), key-free; then the sign
//! guard, Sage's summary against wallet-kit's simulation, wallet-kit's submit (released: the
//! user's signatures leave the wallet inside the text), one approval showing the partial
//! terms, one signature. The made offer is kept in the wallet's own partial offers, so My
//! partial offers lists it and can cancel it.
//!
//! A new method rather than a flag on `chia_createOffer`, so a Klimper without it refuses
//! the request as an unknown method instead of making a whole offer.

use chia_partial_offer::{PartialOffer, PartialOfferTerms, make_partial_offer};
use chia_wallet_sdk::signer::AggSigConstants;
use sage_wallet::{Offered, Requested};
use serde::Serialize;
use tauri_plugin_klimper_partial_offers::{
    Error as PartialError, mine, records::MakeRequest, store::Store, wallet as partial_wallet,
};

use super::create_offer::{Named, OfferView, named, offer_intent, unsupported};
use super::options::{OfferOnly, Part, build_failed, checked_submit, sage_view, too_many_failed};
use super::*;

/// The method name. Klimper's own: Sage's `WalletConnect` has no partial offers.
pub const CHIA_CREATE_PARTIAL_OFFER: &str = "chia_createPartialOffer";

/// What `chia_createPartialOffer` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartialParams {
    /// The asset and amount the offer sells, in mojos.
    pub offered: Part,
    /// The asset and amount it asks for, in mojos.
    pub requested: Part,
    /// The least of the requested asset one fill pays.
    pub min_fill_requested: u64,
    /// The network fee every fill must pay, in mojos of XCH; `None`: none.
    pub required_fee: Option<u64>,
    /// No fill is valid at or after this time (Unix seconds).
    pub expires_at: Option<u64>,
    /// Klimper's own fee ([`wallet_fee`]), paid by the first fill's bundle; the website's is
    /// read and ignored.
    pub fee: u64,
}

/// One side: exactly one `{ assetId?, amount }`, XCH or a CAT, the amount positive.
fn one_part(params: &Value, key: &str) -> Result<Part, RpcError> {
    let entry = match params.get(key).and_then(Value::as_array).map(Vec::as_slice) {
        Some([entry]) if entry.is_object() => entry,
        _ => return Err(invalid(key)),
    };
    let amount = amount_param(entry, "amount")
        .map_err(|_| invalid(key))?
        .filter(|amount| *amount > 0)
        .ok_or_else(|| invalid(key))?;
    match named(entry.get("assetId"), key)? {
        Named::Fungible(asset) => Ok(Part { asset, amount }),
        Named::Option(_) => Err(unsupported("an option")),
    }
}

/// A refusal of the partial terms, in the words of the check that refused them.
fn refused_terms(key: &str, message: &str) -> RpcError {
    error(codes::INVALID_PARAMS, message, Some(key))
}

fn partial_asset(part: Part) -> chia_partial_offer::PartialAsset {
    match part.asset {
        AssetId::Cat(asset_id) => chia_partial_offer::PartialAsset::cat(asset_id, None),
        _ => chia_partial_offer::PartialAsset::xch(),
    }
}

/// `chia_createPartialOffer { offerAssets, requestAssets, minFillRequested, requiredFee?,
/// expiresAt?, fee? }`, read strictly. Other keys are ignored, as `chia_createOffer` ignores
/// them. The terms are checked as the partial driver checks them before anything is built.
pub fn partial_params(params_json: &str, now: u64) -> Result<PartialParams, RpcError> {
    let params: Value = serde_json::from_str(params_json).map_err(|_| invalid("json"))?;
    let offered = one_part(&params, "offerAssets")?;
    let requested = one_part(&params, "requestAssets")?;
    if offered.asset == requested.asset {
        return Err(invalid("requestAssets"));
    }
    let min_fill_requested =
        amount_param(&params, "minFillRequested")?.ok_or_else(|| invalid("minFillRequested"))?;
    if min_fill_requested == 0 || min_fill_requested > requested.amount {
        return Err(refused_terms(
            "minFillRequested",
            "the least one fill pays is between 1 and the amount requested",
        ));
    }
    let required_fee = amount_param(&params, "requiredFee")?.filter(|fee| *fee > 0);
    let expires_at = match amount_param(&params, "expiresAt")? {
        // Already past: an offer nobody could fill.
        Some(at) if at <= now => return Err(invalid("expiresAt")),
        at => at,
    };
    let fee = wallet_fee(&params)?;
    // The driver's own check (the maker comes from the wallet later; it is not checked here).
    PartialOfferTerms {
        offered: partial_asset(offered),
        offered_amount: offered.amount,
        requested: partial_asset(requested),
        requested_amount: requested.amount,
        min_fill_requested,
        expiration: expires_at,
        required_fee,
        maker_puzzle_hash: Bytes32::default(),
    }
    .check()
    .map_err(|e| refused_terms("minFillRequested", &e.to_string()))?;
    Ok(PartialParams {
        offered,
        requested,
        min_fill_requested,
        required_fee,
        expires_at,
        fee,
    })
}

// ---------------------------------------------------------------------------------------
// What the approval shows
// ---------------------------------------------------------------------------------------

/// The terms of each fill, in Klimper's `offer` key next to what the offer gives and asks:
/// from the request as the wallet checked it, the same figures the partial puzzle uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PartialView {
    /// The least of the requested asset one fill pays.
    pub min_fill: crate::trade::Leg,
    /// What that least fill buys of the offered asset (rounded down, as the puzzle does).
    pub min_fill_offered: crate::trade::Leg,
    /// The network fee every fill must pay, in XCH; `None`: none.
    pub fee_per_fill: Option<crate::trade::Leg>,
}

fn leg(asset: AssetId, amount: u64) -> crate::trade::Leg {
    crate::trade::Leg {
        asset,
        amount: u128::from(amount),
    }
}

// ---------------------------------------------------------------------------------------
// The build
// ---------------------------------------------------------------------------------------

/// The one-sided offer, built and not yet signed, and the terms it becomes a partial offer
/// with.
#[derive(Debug, Clone)]
pub struct BuiltPartial {
    /// The user's spends of the one-sided offer: what the user signs.
    pub spends: Vec<CoinSpend>,
    pub terms: PartialOfferTerms,
    /// The network's signature constants, for the security coin's signature.
    pub agg_sig: AggSigConstants,
    pub view: OfferView,
}

/// The request in the words of Klimper's own Make partial offer (`make_partial_offer`).
fn make_request(params: &PartialParams) -> MakeRequest {
    let asset_id = |part: Part| match part.asset {
        AssetId::Cat(asset_id) => Some(hex::encode(asset_id)),
        _ => None,
    };
    MakeRequest {
        offered_asset_id: asset_id(params.offered),
        offered_amount: params.offered.amount.to_string(),
        requested_asset_id: asset_id(params.requested),
        requested_amount: params.requested.amount.to_string(),
        min_fill_requested: params.min_fill_requested.to_string(),
        required_fee: params.required_fee.map(|fee| fee.to_string()),
        expires_at: params.expires_at,
    }
}

/// Make the one-sided offer from the wallet's own coins, key-free: the terms as Klimper's
/// own Make partial offer reads them (the maker is the wallet's change puzzle, refused
/// unless standard; a CAT's revocation from the wallet), and Sage's `make_offer` of the
/// offered asset plus one mojo of XCH for nothing, with Klimper's own fee.
///
/// The terms refused by the partial driver come back as a refusal (`Ok(Err(…))`); Sage's
/// coin selection failing (more than the wallet holds) as an error, answered `build_failed`.
pub async fn build_partial_offer(
    ctx: &partial_wallet::WalletCtx,
    params: &PartialParams,
) -> Result<Result<BuiltPartial, RpcError>, PartialError> {
    let terms = match partial_wallet::terms(ctx, &make_request(params)).await {
        Ok(terms) => terms,
        Err(PartialError::Partial(e)) => {
            return Ok(Err(refused_terms("minFillRequested", &e.to_string())));
        }
        Err(PartialError::Invalid(message)) => {
            return Ok(Err(refused_terms("partial_offer", &message)));
        }
        Err(e) => return Err(e),
    };
    let (xch, cat) = terms.one_sided_offer();
    let offered = Offered {
        xch,
        cats: cat.into_iter().collect(),
        fee: params.fee,
        ..Default::default()
    };
    let unsigned = ctx
        .wallet
        .make_offer(offered, Requested::default(), None)
        .await?;

    let min_fill_offered = terms.price_data().quote(params.min_fill_requested);
    Ok(Ok(BuiltPartial {
        spends: unsigned.coin_spends,
        terms,
        agg_sig: ctx.agg_sig,
        view: OfferView {
            // What leaves for the settlement puzzle: the offered amount, and the mojo of XCH
            // that funds the security coin.
            gives: vec![
                leg(params.offered.asset, params.offered.amount),
                leg(AssetId::Xch, 1),
            ],
            options: Vec::new(),
            asks: vec![leg(params.requested.asset, params.requested.amount)],
            expires_at: params.expires_at,
            partial: Some(PartialView {
                min_fill: leg(params.requested.asset, params.min_fill_requested),
                min_fill_offered: leg(params.offered.asset, min_fill_offered),
                fee_per_fill: params.required_fee.map(|fee| leg(AssetId::Xch, fee)),
            }),
        },
    }))
}

// ---------------------------------------------------------------------------------------
// The request
// ---------------------------------------------------------------------------------------

/// `chia_createPartialOffer`: build, check, ask once, sign once, make the partial offer; the
/// website receives `{ offer: "partial1…", id, coinId, expiresAt? }`. Nothing is broadcast,
/// and the one-sided offer never leaves the wallet.
pub fn create_partial_offer(
    state: &AppState,
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    params_json: &str,
    on_offer: &dyn Fn(&OfferView),
) -> Result<String, RpcError> {
    too_many_failed(ctx)?;
    let params = partial_params(params_json, ctx.now)?;
    let fee = params.fee;
    let sage = state.clone();
    let (built, store) = tauri::async_runtime::block_on(async move {
        let (wallet, store) = tauri_plugin_klimper_partial_offers::sage_ctx(&sage, false).await?;
        Ok::<_, PartialError>((build_partial_offer(&wallet, &params).await?, store))
    })
    .map_err(|e| build_failed(CHIA_CREATE_PARTIAL_OFFER, ctx, &e.to_string()))?;
    let built = built.inspect_err(|_| failed_builds::record(ctx.dapp, ctx.now))?;
    let summary = sage_view(state, &built.spends)?;
    let (answer, offer) =
        submit_partial_offer(ctx, signer, approver, &summary, built, fee, on_offer)?;
    keep_partial_offer(&store, &offer, ctx.now);
    Ok(answer)
}

/// Keeps the made offer in the wallet's own partial offers, as Klimper's Make partial offer
/// does: My partial offers lists it, follows its fills and can cancel it. The website already
/// holds the text, so a failure here is only logged.
pub(super) fn keep_partial_offer(store: &Store, offer: &PartialOffer, now: u64) {
    if let Err(error) = mine::remember(store, offer, now) {
        tracing::warn!("xchonnect: the partial offer could not be kept in the wallet: {error}");
    }
}

/// After the build: the guard on the signed conditions, the cross-check, wallet-kit's submit
/// with released signatures, and the signed one-sided offer turned into the partial offer.
/// Answers the website's JSON and the offer, for the wallet to keep.
pub fn submit_partial_offer(
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    sage_summary: &TransactionSummary,
    built: BuiltPartial,
    fee: u64,
    on_offer: &dyn Fn(&OfferView),
) -> Result<(String, PartialOffer), RpcError> {
    crate::sign_guard::check_spends(&built.spends, ctx.keys, &ctx.network)?;
    on_offer(&built.view);
    let expires_at = built.view.expires_at;
    let submitted = checked_submit(
        ctx,
        signer,
        approver,
        &OfferOnly,
        sage_summary,
        CHIA_CREATE_PARTIAL_OFFER,
        built.spends,
        &offer_intent(false, fee),
        // The user's signatures leave the wallet inside the partial offer: released (spec
        // 11.2). The one-sided offer itself does not.
        true,
    )?;
    let offer = make_partial_offer(&submitted.bundle, &built.terms, &built.agg_sig)
        .map_err(|e| refused_terms("partial_offer", &e.to_string()))?;
    let text = offer
        .to_text()
        .map_err(|e| refused_terms("partial_offer", &e.to_string()))?;
    let bundle = offer
        .to_spend_bundle()
        .map_err(|e| refused_terms("partial_offer", &e.to_string()))?;
    // `id` as `chia_createOffer` gives it: the name of the sorted bundle, plain hex.
    // `coinId`: the first partial coin, as nodexch's `partial.coin_id` and My partial offers
    // name it.
    let mut answer = serde_json::json!({
        "offer": text,
        "id": hex::encode(sage_wallet::sort_offer(bundle).name()),
        "coinId": hex0x(offer.coin.coin_id()),
    });
    if let Some(at) = expires_at {
        answer["expiresAt"] = serde_json::json!(at);
    }
    Ok((answer.to_string(), offer))
}
