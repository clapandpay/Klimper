// SPDX-License-Identifier: AGPL-3.0-or-later
//! Wallet-side execution (xchonnect-wiki 04, decided 2026-10-05): Klimper builds, signs
//! once and broadcasts; a website receives only the result.
//!
//! Three pieces, each small:
//!
//! * [`SageBroadcaster`]: hands a signed bundle to Klimper's own light-wallet peers through
//!   Sage's `send_transaction_immediately` — not to a third-party node, so no outside
//!   service links the user's address to the transaction — and records it as pending.
//! * [`send`]: `chia_send`, Sage's `WalletConnect` method. Sage builds the spend from the
//!   wallet's own coins (`auto_submit: false`), and it is then run through the same path as
//!   any other spend — wallet-kit's simulation, policy, limits, one approval, one batch
//!   signature — with an intent Klimper derives from the request itself, so the bundle is
//!   proven to pay exactly what was asked to exactly whom.
//! * [`take_offer`]: `chia_takeOffer`, Sage's `WalletConnect` method too (TASK-137). The
//!   website sends only the offer; Sage takes it from the wallet's own coins with the same
//!   offer code as `WalletConnect`, and the bundle runs through the same path as a send,
//!   completed by the maker's signature from the offer.
//! * [`cross_check`]: Sage's own summary of a spend (from the wallet database) against
//!   wallet-kit's independent simulation. If they disagree about what leaves the wallet —
//!   the user's coins spent, what goes to others, the fee — the request is refused and the
//!   user is never asked to approve a figure one of the two engines contradicts.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use chia_bls::Signature;
use chia_protocol::{Bytes32, Coin, CoinSpend, Program, SpendBundle};
use sage_api::wallet_connect::SendTransactionImmediately;
use sage_api::{
    Amount, AssetKind, CoinJson, CoinSpendJson, SendCat, SendXch, TransactionSummary,
    ViewCoinSpends,
};
use serde_json::Value;
use xchonnect_core::rpc::codes;
use xchonnect_wallet_kit::{
    self as kit, AssetId, Broadcast, Broadcaster, Intent, RecipientClaim, RequestContext, RpcError,
    SubmitRequest, Summary,
};

use crate::wallet::AppState;

/// The method name, as Sage's `WalletConnect` handler knows it.
pub const CHIA_SEND: &str = "chia_send";

/// The method name, as Sage's `WalletConnect` handler knows it.
pub const CHIA_TAKE_OFFER: &str = "chia_takeOffer";

/// Methods Klimper builds itself, granted at pairing next to wallet-kit's defaults. Each
/// still asks the user before anything is signed.
pub const WALLET_BUILT_METHODS: [&str; 2] = [CHIA_SEND, CHIA_TAKE_OFFER];

/// Whether Klimper builds this request from the wallet's own coins (and so signs it with
/// every key of the wallet, not only the ones the website was shown): the methods above and
/// the option methods ([`options::OPTION_METHODS`], TASK-140).
pub fn is_wallet_built(method: &str) -> bool {
    WALLET_BUILT_METHODS.contains(&method) || options::OPTION_METHODS.contains(&method)
}

pub mod options;

/// The revision of [`WALLET_BUILT_METHODS`] a new pairing is granted. 1: `chia_takeOffer`.
pub const METHODS_REVISION: u32 = 1;

/// Bring a connection paired under an older revision up to date, once: a connection that
/// may use `chia_send` may also take offers, which ask the user the same way. Without this
/// a take from such a connection is refused as `not_granted` until the user pairs again
/// (TASK-137). A connection whose `chia_send` the user turned off gets nothing. Returns
/// whether the methods changed, so the website can be told.
pub fn upgrade_methods(record: &mut crate::store::SessionRecord) -> bool {
    if record.methods_revision >= METHODS_REVISION {
        return false;
    }
    record.methods_revision = METHODS_REVISION;
    let methods = &mut record.permissions.methods;
    if methods.iter().any(|m| m == CHIA_SEND) && !methods.iter().any(|m| m == CHIA_TAKE_OFFER) {
        methods.push(CHIA_TAKE_OFFER.to_owned());
        return true;
    }
    false
}

fn error(code: i64, message: &str, reason: Option<&str>) -> RpcError {
    RpcError {
        code,
        message: message.to_owned(),
        data: reason.map(|r| serde_json::json!({ "reason": r }).to_string()),
    }
}

fn invalid(reason: &str) -> RpcError {
    error(codes::INVALID_PARAMS, "invalid params", Some(reason))
}

fn hex0x(bytes: impl AsRef<[u8]>) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    hex::decode(text.strip_prefix("0x").unwrap_or(text)).ok()
}

/// Run a Sage call from the request thread (a plain OS thread, see `requests::answer`).
fn on_sage<T>(
    state: &AppState,
    call: impl AsyncFnOnce(&sage::Sage) -> sage::Result<T>,
) -> Result<T, String> {
    let state = state.clone();
    tauri::async_runtime::block_on(async move {
        let sage = state.lock().await;
        call(&sage).await.map_err(|e| e.to_string())
    })
}

// ---------------------------------------------------------------------------------------
// Conversions between chia_protocol and Sage's JSON shapes.
// ---------------------------------------------------------------------------------------

fn to_json(cs: &CoinSpend) -> CoinSpendJson {
    CoinSpendJson {
        coin: CoinJson {
            parent_coin_info: hex0x(cs.coin.parent_coin_info),
            puzzle_hash: hex0x(cs.coin.puzzle_hash),
            amount: Amount::u64(cs.coin.amount),
        },
        puzzle_reveal: hex0x(cs.puzzle_reveal.as_ref()),
        solution: hex0x(cs.solution.as_ref()),
    }
}

fn from_json(cs: &CoinSpendJson) -> Option<CoinSpend> {
    Some(CoinSpend {
        coin: Coin {
            parent_coin_info: Bytes32::try_from(unhex(&cs.coin.parent_coin_info)?.as_slice())
                .ok()?,
            puzzle_hash: Bytes32::try_from(unhex(&cs.coin.puzzle_hash)?.as_slice()).ok()?,
            amount: cs.coin.amount.to_u64()?,
        },
        puzzle_reveal: Program::from(unhex(&cs.puzzle_reveal)?),
        solution: Program::from(unhex(&cs.solution)?),
    })
}

// ---------------------------------------------------------------------------------------
// Broadcasting through Klimper's own peers.
// ---------------------------------------------------------------------------------------

/// Submits through Sage's peer connections (Sage records the transaction as pending), after
/// writing what wallet-kit reports into the wallet's record of signed coins (xchonnect spec
/// 11.2, [`Broadcaster::record`]) and keeping a sealed bundle's bytes to send it again.
pub struct SageBroadcaster<'a> {
    state: AppState,
    recorder: crate::signed_coins::Recorder<'a>,
}

impl std::fmt::Debug for SageBroadcaster<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SageBroadcaster").finish_non_exhaustive()
    }
}

impl<'a> SageBroadcaster<'a> {
    pub fn new(state: AppState, record: &'a crate::signed_coins::SignedCoinsHandle) -> Self {
        SageBroadcaster {
            state,
            recorder: crate::signed_coins::Recorder::new(record),
        }
    }
}

impl Broadcaster for SageBroadcaster<'_> {
    fn record(&self, record: &kit::BroadcastRecord) -> Result<(), String> {
        self.recorder.record(record)
    }

    fn submit(&self, bundle: &SpendBundle) -> Result<Broadcast, String> {
        self.recorder.before_send(bundle);
        send_to_sage(&self.state, bundle)
    }
}

/// Send a sealed bundle Klimper already broadcast once again, byte for byte, through Sage's
/// peers (xchonnect spec 11.2: never re-signed). `None`: no such bundle is kept.
pub fn resend_sealed(
    state: &AppState,
    record: &crate::signed_coins::SignedCoinsHandle,
    bundle_id: &Bytes32,
) -> crate::error::Result<Option<Result<Broadcast, String>>> {
    crate::signed_coins::resend(record, bundle_id, |bundle| send_to_sage(state, bundle))
}

/// Hand `bundle` to Sage's peers as it is.
fn send_to_sage(state: &AppState, bundle: &SpendBundle) -> Result<Broadcast, String> {
    let request = SendTransactionImmediately {
        spend_bundle: sage_api::wallet_connect::SpendBundle {
            coin_spends: bundle
                .coin_spends
                .iter()
                .map(|cs| sage_api::wallet_connect::CoinSpend {
                    coin: sage_api::wallet_connect::Coin {
                        parent_coin_info: hex0x(cs.coin.parent_coin_info),
                        puzzle_hash: hex0x(cs.coin.puzzle_hash),
                        amount: Amount::u64(cs.coin.amount),
                    },
                    puzzle_reveal: hex0x(cs.puzzle_reveal.as_ref()),
                    solution: hex0x(cs.solution.as_ref()),
                })
                .collect(),
            aggregated_signature: hex0x(bundle.aggregated_signature.to_bytes()),
        },
    };
    let response = on_sage(state, async move |sage| {
        sage.send_transaction_immediately(request).await
    })?;
    let transaction_id = hex0x(bundle.name());
    match response.status {
        // In a peer's mempool, and recorded by the wallet as pending.
        1 => Ok(Broadcast {
            transaction_id,
            status: "pending".to_owned(),
        }),
        // No peer said yes or no. It may still propagate; the dApp is told so.
        3 => Ok(Broadcast {
            transaction_id,
            status: "unknown".to_owned(),
        }),
        status => Err(response.error.unwrap_or_else(|| format!("status {status}"))),
    }
}

// ---------------------------------------------------------------------------------------
// Sage's summary against wallet-kit's simulation.
// ---------------------------------------------------------------------------------------

/// What leaves the user's wallet, per asset, in the two engines' terms.
#[derive(Debug, Default, PartialEq, Eq)]
struct Outflow {
    /// The user's coins spent.
    spent: BTreeMap<AssetId, u128>,
    /// Paid by the user's spends to others.
    paid: BTreeMap<AssetId, u128>,
}

fn sage_asset(asset: Option<&sage_api::Asset>) -> Option<AssetId> {
    let asset = asset?;
    if asset.kind != AssetKind::Token {
        return None;
    }
    match asset.asset_id.as_deref() {
        None | Some("xch" | "") => Some(AssetId::Xch),
        Some(id) => Bytes32::try_from(unhex(id)?.as_slice())
            .ok()
            .map(AssetId::Cat),
    }
}

/// Refuse when the wallet database and the simulation disagree about what leaves the
/// wallet. Only coins wallet-kit counts as the user's are compared — the other inputs of
/// a multi-party bundle are someone else's — and only XCH and CATs, the assets both engines
/// value. An option singleton is left out (TASK-140): Sage does not see the eve of an option a
/// request mints as the wallet's coin, and where an option goes is checked by wallet-kit's own
/// option effects and the request's verified intent instead.
///
/// Collateral its creator takes back was locked, not part of the balance: wallet-kit counts
/// only what does not return to the user as sent (xchonnect rc.5, TASK-78), so the same is
/// taken from Sage's summary for that input. What is paid to others is compared in full.
pub fn cross_check(sage: &TransactionSummary, kit_summary: &Summary) -> Result<(), RpcError> {
    let owned: std::collections::HashMap<&str, &kit::simulate::SpendKind> = kit_summary
        .spends
        .iter()
        .filter(|s| s.owned)
        .map(|s| (s.coin_id.as_str(), &s.kind))
        .collect();

    let mut from_sage = Outflow::default();
    for input in &sage.inputs {
        let id = input.coin_id.strip_prefix("0x").unwrap_or(&input.coin_id);
        let Some(kind) = owned.get(id) else {
            continue;
        };
        let Some(asset) = sage_asset(input.asset.as_ref()) else {
            continue;
        };
        let amount = u128::from(input.amount.to_u64().unwrap_or(u64::MAX));
        let spent = match kind {
            kit::simulate::SpendKind::OptionUnderlying {
                path: kit::option::UnderlyingPath::Clawback,
                ..
            } => {
                let returned: u128 = input
                    .outputs
                    .iter()
                    .filter(|o| o.receiving)
                    .map(|o| u128::from(o.amount.to_u64().unwrap_or(u64::MAX)))
                    .sum();
                amount.saturating_sub(returned)
            }
            _ => amount,
        };
        *from_sage.spent.entry(asset).or_default() += spent;
        for output in input.outputs.iter().filter(|o| !o.receiving) {
            let amount = u128::from(output.amount.to_u64().unwrap_or(u64::MAX));
            *from_sage.paid.entry(asset).or_default() += amount;
        }
    }

    let mut from_kit = Outflow::default();
    for delta in &kit_summary.assets {
        if delta.sent > 0 && !matches!(delta.asset, AssetId::Option(_)) {
            from_kit.spent.insert(delta.asset, delta.sent);
        }
    }
    for output in kit_summary
        .outputs
        .iter()
        .filter(|o| !o.to_user && !matches!(o.asset, AssetId::Option(_)))
    {
        *from_kit.paid.entry(output.asset).or_default() += u128::from(output.amount);
    }
    // Zero on either side is no amount at all: an offer take creates zero-amount coins (a
    // CAT ring moves the value on to the settlement payment), and an entry of 0 on one side
    // only must not read as a disagreement (TASK-137).
    for outflow in [&mut from_sage, &mut from_kit] {
        outflow.spent.retain(|_, v| *v > 0);
        outflow.paid.retain(|_, v| *v > 0);
    }

    let fee_agrees = kit_summary.implied_fee.map(u128::from) == sage.fee.to_u64().map(u128::from);
    if from_sage != from_kit || !fee_agrees {
        tracing::warn!("xchonnect: the wallet database and the simulation disagree; refusing");
        return Err(error(
            codes::UNAUTHORIZED,
            "the wallet could not confirm what this transaction does",
            Some("crosscheck_mismatch"),
        ));
    }
    Ok(())
}

/// Sage's summary of `spends`, and wallet-kit's simulation of them, compared.
pub fn check_spends(
    state: &AppState,
    ctx: &RequestContext<'_>,
    spends: &[CoinSpend],
) -> Result<(), RpcError> {
    let kit_summary = kit::simulate(spends, ctx.ownership, kit::simulate::DEFAULT_MAX_COST)
        .map_err(RpcError::from)?;
    let request = ViewCoinSpends {
        coin_spends: spends.iter().map(to_json).collect(),
    };
    let sage_summary = on_sage(state, async move |sage| {
        sage.view_coin_spends(request).await
    })
    .map_err(|e| {
        tracing::warn!("xchonnect: the wallet could not summarise a spend: {e}");
        error(
            codes::UNSUPPORTED_CONTENT,
            "the wallet could not read this transaction",
            Some("crosscheck_unavailable"),
        )
    })?;
    cross_check(&sage_summary.summary, &kit_summary)
}

// ---------------------------------------------------------------------------------------
// chia_send
// ---------------------------------------------------------------------------------------

/// `safeAmount` in Sage's `WalletConnect` schema: a non-negative integer, number or string.
fn amount_param(params: &Value, key: &str) -> Result<Option<u64>, RpcError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(Some).ok_or_else(|| invalid(key)),
        Some(Value::String(s)) => s.parse().map(Some).map_err(|_| invalid(key)),
        Some(_) => Err(invalid(key)),
    }
}

/// The most fee Klimper pays in a request it builds and broadcasts itself: 0.001 XCH
/// (`1_000_000_000` mojos), well above what a Chia transaction needs to get in, even in a
/// full mempool. A larger default fee set in the wallet is taken as this much.
pub const MAX_FEE: u64 = 1_000_000_000;

/// The fee of Klimper's own requests, in mojos: the wallet's default fee, as the user set it
/// in the wallet's settings (the webview hands it over, [`set_own_fee`]). 0 until it has.
static OWN_FEE: AtomicU64 = AtomicU64::new(0);

/// The user's default fee, in mojos, for the requests Klimper builds itself; at most
/// [`MAX_FEE`].
pub fn set_own_fee(mojos: u64) {
    OWN_FEE.store(capped_fee(mojos), Ordering::SeqCst);
}

fn capped_fee(mojos: u64) -> u64 {
    mojos.min(MAX_FEE)
}

/// The fee of Klimper's own requests now ([`set_own_fee`]).
pub fn own_fee() -> u64 {
    OWN_FEE.load(Ordering::SeqCst)
}

/// The fee of a request Klimper builds and broadcasts sealed: Klimper's own choice, the
/// user's default fee ([`set_own_fee`]), and never a figure a website picked (xchonnect spec
/// 11.1.1 and 11.2: wallet-kit credits a sealed bundle's fee as the wallet's, and a host
/// MUST NOT build a sealed bundle with a fee from a dApp's parameters). A website's `fee`
/// is still read, so a malformed one is refused as before, and then ignored: not even a
/// capped hint. It is passed to wallet-kit as `SubmitRequest::wallet_fee`, and the approval
/// shows the fee the bundle pays.
fn wallet_fee(params: &Value) -> Result<u64, RpcError> {
    fee_of(params, own_fee())
}

/// [`wallet_fee`] with the wallet's fee given: the website's `fee` checked, then dropped.
fn fee_of(params: &Value, own: u64) -> Result<u64, RpcError> {
    amount_param(params, "fee")?;
    Ok(own)
}

/// `chia_send { address, amount, fee?, assetId?, memos? }`: Sage builds the send from the
/// wallet's own coins, Klimper checks, asks once, signs once and broadcasts.
pub fn send(
    state: &AppState,
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    params_json: &str,
) -> Result<String, RpcError> {
    let params: Value = serde_json::from_str(params_json).map_err(|_| invalid("json"))?;
    let address = params
        .get("address")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("address"))?
        .to_owned();
    let amount = amount_param(&params, "amount")?.ok_or_else(|| invalid("amount"))?;
    if amount == 0 {
        return Err(invalid("amount"));
    }
    let fee = wallet_fee(&params)?;
    // A send that cannot be built is answered without a prompt, which tells the dApp the
    // wallet cannot afford it. A few are normal; a run of them is a search for the balance.
    if failed_builds::too_many(ctx.dapp, ctx.now) {
        return Err(error(
            codes::SPENDABLE_BALANCE_EXCEEDED,
            "the wallet could not build this send",
            Some("build_failed"),
        ));
    }
    let memos: Vec<String> = params
        .get("memos")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|m| m.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let asset = match params.get("assetId").and_then(Value::as_str) {
        None | Some("" | "xch") => AssetId::Xch,
        Some(id) => AssetId::Cat(
            Bytes32::try_from(unhex(id).ok_or_else(|| invalid("assetId"))?.as_slice())
                .map_err(|_| invalid("assetId"))?,
        ),
    };

    // Build from the wallet's own coins; nothing is signed or submitted here.
    let to_address = address.clone();
    let cat = match asset {
        AssetId::Cat(id) => Some(id),
        AssetId::Xch => None,
        _ => return Err(invalid("assetId")),
    };
    let built = on_sage(state, async move |sage| match cat {
        None => {
            sage.send_xch(SendXch {
                address: to_address,
                amount: Amount::u64(amount),
                fee: Amount::u64(fee),
                memos,
                clawback: None,
                auto_submit: false,
            })
            .await
        }
        Some(id) => {
            sage.send_cat(SendCat {
                asset_id: hex::encode(id),
                address: to_address,
                amount: Amount::u64(amount),
                fee: Amount::u64(fee),
                include_hint: true,
                memos,
                clawback: None,
                auto_submit: false,
            })
            .await
        }
    })
    .map_err(|e| {
        // Sage's own words stay here: they name balances and coins, and a dApp could probe
        // the wallet with them without the user ever seeing a prompt.
        tracing::info!("xchonnect: chia_send could not be built: {e}");
        failed_builds::record(ctx.dapp, ctx.now);
        error(
            codes::SPENDABLE_BALANCE_EXCEEDED,
            "the wallet could not build this send",
            Some("build_failed"),
        )
    })?;
    let puzzle_hash = on_sage(state, async move |sage| sage.parse_address(address))
        .map_err(|_| invalid("address"))?;

    let spends: Vec<CoinSpend> = built
        .coin_spends
        .iter()
        .map(from_json)
        .collect::<Option<_>>()
        .ok_or_else(|| invalid("coin_spends"))?;

    // The send is proven to do what was asked: exactly this payment to exactly this
    // recipient (none to anyone else) and exactly this fee. A send to one of the user's
    // own addresses pays nobody else.
    let to_self = ctx.ownership.p2_puzzle_hashes.contains(&puzzle_hash);
    let intent = Intent {
        kind: Some(if to_self { "send.self" } else { "send" }.to_owned()),
        recipients: Some(if to_self {
            Vec::new()
        } else {
            vec![RecipientClaim {
                asset,
                puzzle_hash,
                amount,
            }]
        }),
        fee: Some(fee),
        net: None,
    };

    // Sage spends the wallet's coins with quoted conditions; held to it all the same.
    crate::sign_guard::check_spends(&spends, ctx.keys, &ctx.network)?;
    let kit_summary = kit::simulate(&spends, ctx.ownership, kit::simulate::DEFAULT_MAX_COST)
        .map_err(RpcError::from)?;
    cross_check(&built.summary, &kit_summary)?;

    // Klimper broadcasts the bundle itself and answers only its id and status: sealed
    // (xchonnect spec 11.2). The caller's broadcaster records its coins.
    let submitted = kit::submit(
        SubmitRequest {
            method: CHIA_SEND,
            spends,
            other_signature: None,
            intent: Some(&intent),
            wallet_built: true,
            // Klimper's own fee ([`wallet_fee`]), which the bundle reserves.
            wallet_fee: fee,
            signatures_released: false,
        },
        ctx,
        signer,
        approver,
        broadcaster,
    )?;
    Ok(submitted.to_json())
}

// ---------------------------------------------------------------------------------------
// chia_takeOffer
// ---------------------------------------------------------------------------------------

/// The offer a website sent, decoded: the maker's spends and the maker's signature. An offer
/// that does not decode is refused before the wallet is asked anything.
fn decode_offer_bundle(offer: &str) -> Result<SpendBundle, RpcError> {
    chia_wallet_sdk::driver::decode_offer(offer).map_err(|_| invalid("offer"))
}

/// The most coin spends an offer may carry. Real offers have a handful (a few coins and
/// the placeholders for what they ask); this only bounds what a website can make the wallet
/// read before anyone is asked.
const MAX_OFFER_SPENDS: usize = 64;

/// What is checked about an offer before the wallet builds anything from it:
///
/// * its size ([`MAX_OFFER_SPENDS`]);
/// * that no coin of the maker's side is spent twice: an aggregate holding one offer twice
///   (TASK-139) is a bundle the chain refuses, and the user would approve a take that
///   cannot happen. (The asked-for placeholders share one coin id by design.);
/// * that it asks for something: an offer with no requested payment (a gift) gives the user
///   no settlement payment to tie their spends to, so the take could never be signed safely;
/// * the maker's spends run within wallet-kit's cost budget and are all puzzles wallet-kit
///   recognises. An unknown one (an NFT, a DID, an option) is refused whatever the website's
///   "unknown contracts" setting: the approval could not say what is being traded;
/// * the maker's signature covers the maker's spends ([`check_maker_signature`]).
fn check_offer(offer: &SpendBundle, ctx: &RequestContext<'_>) -> Result<(), RpcError> {
    if offer.coin_spends.len() > MAX_OFFER_SPENDS {
        return Err(invalid("offer_too_large"));
    }
    let (placeholders, makers): (Vec<CoinSpend>, Vec<CoinSpend>) = offer
        .coin_spends
        .iter()
        .cloned()
        .partition(|spend| spend.coin.parent_coin_info == Bytes32::default());
    let mut coins = std::collections::HashSet::with_capacity(makers.len());
    if !makers
        .iter()
        .all(|spend| coins.insert(spend.coin.coin_id()))
    {
        return Err(invalid("offer_duplicate_coin"));
    }
    if placeholders.is_empty() {
        return Err(error(
            codes::UNSUPPORTED_CONTENT,
            "an offer that asks for nothing cannot be taken here",
            Some("nothing_requested"),
        ));
    }
    let mut allocator = chia_wallet_sdk::prelude::Allocator::new();
    let (executed, _) = kit::simulate::execute(
        &mut allocator,
        &makers,
        ctx.ownership,
        kit::simulate::DEFAULT_MAX_COST,
    )
    .map_err(|_| invalid("offer"))?;
    if executed
        .iter()
        .any(|spend| spend.kind == kit::SpendKind::Unknown)
    {
        return Err(error(
            codes::UNSUPPORTED_CONTENT,
            "unknown contract",
            Some("unknown_contract"),
        ));
    }
    check_maker_signature(offer, &ctx.network)
}

/// `AGG_SIG` constants of the request's network, as wallet-kit's policy computes them.
pub(crate) fn agg_sig_constants(
    network: &kit::Network,
) -> chia_wallet_sdk::signer::AggSigConstants {
    use chia_wallet_sdk::types::{MAINNET_CONSTANTS, TESTNET11_CONSTANTS};
    chia_wallet_sdk::signer::AggSigConstants::new(match network {
        kit::Network::Mainnet => MAINNET_CONSTANTS.agg_sig_me_additional_data,
        kit::Network::Testnet11 => TESTNET11_CONSTANTS.agg_sig_me_additional_data,
        kit::Network::Custom { agg_sig_me, .. } => *agg_sig_me,
    })
}

/// The maker's signature must verify over the maker's own spends **before** the user is
/// asked: an offer with a missing or forged signature can never be completed, and the user
/// would approve a take that is then refused. Wallet-kit checks the whole bundle again after
/// signing; this only moves a broken offer's refusal in front of the prompt.
fn check_maker_signature(offer: &SpendBundle, network: &kit::Network) -> Result<(), RpcError> {
    use chia_wallet_sdk::signer::RequiredSignature;
    // The maker's own spends. The others are placeholders for what the maker asks (a coin
    // with no parent, as `Offer::from_spend_bundle` reads them): they are never signed, and a
    // CAT placeholder cannot even be run without its ring.
    let makers: Vec<CoinSpend> = offer
        .coin_spends
        .iter()
        .filter(|spend| spend.coin.parent_coin_info != Bytes32::default())
        .cloned()
        .collect();
    if makers.is_empty() {
        return Err(invalid("offer"));
    }
    let mut allocator = chia_wallet_sdk::prelude::Allocator::new();
    let required =
        RequiredSignature::from_coin_spends(&mut allocator, &makers, &agg_sig_constants(network))
            .map_err(|_| invalid("offer"))?;
    let mut pairs = Vec::with_capacity(required.len());
    for requirement in required {
        match requirement {
            RequiredSignature::Bls(bls) => pairs.push((bls.public_key, bls.message())),
            // No wallet Klimper knows makes secp offers; refused rather than half-checked.
            RequiredSignature::Secp(_) => {
                return Err(error(
                    codes::UNSUPPORTED_CONTENT,
                    "this offer is not supported",
                    Some("secp_signature"),
                ));
            }
        }
    }
    let verified = chia_bls::aggregate_verify(
        &offer.aggregated_signature,
        pairs.iter().map(|(key, message)| (key, message.as_slice())),
    );
    if !verified {
        return Err(error(
            codes::INVALID_PARAMS,
            "the offer's signature does not match it",
            Some("offer_signature"),
        ));
    }
    Ok(())
}

/// Change the conditions of one of the user's own, still unsigned spends: a plain standard
/// spend (`(() (q . conditions) ())`, as Sage builds them), a CAT whose inner puzzle is one,
/// or an option contract whose owner's puzzle is one (TASK-140: the option an exercise melts).
/// Returns `false`, leaving the spend as it was, for any other shape. Only the tests use it,
/// to make a website's tampered spend: Klimper hands Sage's spends to wallet-kit as Sage
/// builds them (xchonnect spec 11.2, sealed requests).
#[cfg(test)]
fn edit_user_conditions(
    allocator: &mut chia_wallet_sdk::prelude::Allocator,
    spend: &mut CoinSpend,
    edit: impl FnOnce(&mut Vec<chia_wallet_sdk::prelude::Condition<chia_wallet_sdk::prelude::NodePtr>>),
) -> bool {
    use chia_puzzle_types::{cat::CatSolution, standard::StandardSolution};
    use chia_wallet_sdk::driver::CatLayer;
    use chia_wallet_sdk::prelude::{
        Condition, FromClvm, Layer, NodePtr, Puzzle, StandardLayer, ToClvm,
    };

    type Standard = StandardSolution<(u8, NodePtr), NodePtr>;

    // The standard solution's condition list, edited and rebuilt; `None` if it is not the
    // plain shape.
    fn edited(
        allocator: &mut chia_wallet_sdk::prelude::Allocator,
        solution: NodePtr,
        edit: impl FnOnce(&mut Vec<Condition<NodePtr>>),
    ) -> Option<NodePtr> {
        let standard = Standard::from_clvm(allocator, solution).ok()?;
        if standard.original_public_key.is_some() || standard.delegated_puzzle.0 != 1 {
            return None;
        }
        let mut conditions =
            Vec::<Condition<NodePtr>>::from_clvm(allocator, standard.delegated_puzzle.1).ok()?;
        edit(&mut conditions);
        StandardSolution::from_conditions(conditions)
            .to_clvm(allocator)
            .ok()
    }

    let (Ok(puzzle), Ok(solution)) = (
        spend.puzzle_reveal.to_clvm(allocator),
        spend.solution.to_clvm(allocator),
    ) else {
        return false;
    };
    let puzzle = Puzzle::parse(allocator, puzzle);
    let rebuilt = if matches!(StandardLayer::parse_puzzle(allocator, puzzle), Ok(Some(_))) {
        edited(allocator, solution, edit)
    } else if matches!(
        CatLayer::<StandardLayer>::parse_puzzle(allocator, puzzle),
        Ok(Some(_))
    ) {
        let Ok(mut cat) = CatSolution::<NodePtr>::from_clvm(allocator, solution) else {
            return false;
        };
        let Some(inner) = edited(allocator, cat.inner_puzzle_solution, edit) else {
            return false;
        };
        cat.inner_puzzle_solution = inner;
        cat.to_clvm(allocator).ok()
    } else if let Ok(Some((_, p2, _))) =
        chia_wallet_sdk::driver::OptionContract::parse(allocator, spend.coin, puzzle, solution)
        && matches!(StandardLayer::parse_puzzle(allocator, p2), Ok(Some(_)))
    {
        use chia_puzzle_types::singleton::SingletonSolution;
        use chia_wallet_sdk::types::puzzles::OptionContractSolution;
        let Ok(mut option) =
            SingletonSolution::<OptionContractSolution<NodePtr>>::from_clvm(allocator, solution)
        else {
            return false;
        };
        let Some(inner) = edited(allocator, option.inner_solution.inner_solution, edit) else {
            return false;
        };
        option.inner_solution.inner_solution = inner;
        option.to_clvm(allocator).ok()
    } else {
        None
    };
    let Some(rebuilt) = rebuilt.and_then(|ptr| Program::from_clvm(allocator, ptr).ok()) else {
        return false;
    };
    spend.solution = rebuilt;
    true
}

/// The take, built from the wallet's own coins with Sage's offer code and **no key**: the
/// user's side comes back unsigned, joined with the maker's spends, and with the maker's
/// signature alone. The wallet's key is used once, by wallet-kit, after the approval, as for
/// a send. (Sage's `take_offer` endpoint signs the user's side as it builds; not used here.)
async fn build_take(
    wallet: &sage_wallet::Wallet,
    offer: SpendBundle,
    fee: u64,
) -> Result<(Vec<CoinSpend>, Signature), sage_wallet::WalletError> {
    let taken = wallet.take_offer(offer, fee).await?;
    // `Offer::take` puts the maker's spends first and adds the signatures; the user's side
    // carries none yet, so the result's signature is the maker's.
    let joined = taken.offer.take(SpendBundle::new(
        taken.spend_bundle.coin_spends,
        Signature::default(),
    ));
    Ok((joined.coin_spends, joined.aggregated_signature))
}

/// `chia_takeOffer { offer, fee? }`: Sage takes the offer from the wallet's own coins with
/// the same code as `WalletConnect` (`auto_submit: false`, so nothing leaves the wallet
/// here), Klimper checks, asks once, signs once and broadcasts. The website never builds
/// the user's side and never sees which coins pay for it.
///
/// The offer may be several makers' offers aggregated into one (TASK-139): their spends
/// together and their signatures summed, as Sage's `aggregate_offers` makes it. It is
/// checked, built and taken the same way, all or nothing; `offers_taken` is told how many
/// offers it holds, for the approval screen, once the offer passed its checks, and `on_loan`
/// the loan the user lends, if the offer is a loan request (TASK-140).
#[allow(clippy::too_many_arguments)]
pub fn take_offer(
    state: &AppState,
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    params_json: &str,
    offers_taken: &dyn Fn(usize),
    on_loan: &dyn Fn(&options::LoanView, bool),
) -> Result<String, RpcError> {
    // As for a send: a take that cannot be built tells the website the wallet cannot pay
    // for it, so a run of them is a search for the balance. Checked first, so a website
    // that keeps sending offers the wallet refuses gets nothing more done.
    if failed_builds::too_many(ctx.dapp, ctx.now) {
        return Err(error(
            codes::SPENDABLE_BALANCE_EXCEEDED,
            "the wallet could not take this offer",
            Some("build_failed"),
        ));
    }
    let params: Value = serde_json::from_str(params_json).map_err(|_| invalid("json"))?;
    let fee = wallet_fee(&params)?;
    let offer = params
        .get("offer")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|offer| !offer.is_empty())
        .ok_or_else(|| invalid("offer"))?;
    // Everything about the offer itself is settled before Sage, its asset lookups or the
    // wallet lock are touched, and every refusal here counts like a failed build.
    let offer = decode_offer_bundle(offer)
        .and_then(|offer| check_offer(&offer, ctx).map(|()| offer))
        .inspect_err(|_| failed_builds::record(ctx.dapp, ctx.now))?;
    offers_taken(crate::trade::offers_in(&offer));
    let maker_coins = maker_coins(&offer);

    let (spends, maker_signature) = on_sage(state, async move |sage| {
        Ok(build_take(&*sage.wallet()?, offer, fee).await?)
    })
    .map_err(|e| {
        // Sage's own words stay here, as for a send: they name balances and coins.
        tracing::info!("xchonnect: chia_takeOffer could not be built: {e}");
        failed_builds::record(ctx.dapp, ctx.now);
        error(
            codes::SPENDABLE_BALANCE_EXCEEDED,
            "the wallet could not take this offer",
            Some("build_failed"),
        )
    })?;
    // Sage's own reading of the whole bundle, from the wallet database, for the
    // cross-check: the same summary its take_offer endpoint returns. A failure here is
    // not about the balance, and is answered as `check_spends` answers it.
    let request = ViewCoinSpends {
        coin_spends: spends.iter().map(to_json).collect(),
    };
    let summary = on_sage(state, async move |sage| {
        Ok(sage.view_coin_spends(request).await?.summary)
    })
    .map_err(|e| {
        tracing::warn!("xchonnect: the wallet could not summarise a take: {e}");
        error(
            codes::UNSUPPORTED_CONTENT,
            "the wallet could not read this transaction",
            Some("crosscheck_unavailable"),
        )
    })?;

    submit_take(
        ctx,
        signer,
        approver,
        broadcaster,
        &summary,
        spends,
        &maker_coins,
        maker_signature,
        fee,
        on_loan,
    )
}

/// The coins of every spend in the offer: the maker's, and the placeholders for what the
/// maker asks. In the built take, every other spend is one Sage built from the user's coins.
fn maker_coins(offer: &SpendBundle) -> HashSet<Bytes32> {
    offer
        .coin_spends
        .iter()
        .map(|spend| spend.coin.coin_id())
        .collect()
}

/// What a take claims: only that it is one (`offer.take`), or for a loan the user lends,
/// `loan.take`, which wallet-kit verifies: the user is the option's creator, the collateral
/// and the option are the borrower's (TASK-140). What the user pays and receives, the fee
/// included, is shown from the simulation. No fee is claimed: wallet-kit checks a fee claim
/// against the whole bundle's fee, and an offer's maker may pay a fee of their own in the
/// same bundle, so the user's fee alone would never match ("unauthorized",
/// `intent_mismatch`; TASK-137). The fee Sage built with is Klimper's own ([`wallet_fee`]),
/// and the cross-check holds both engines to the same total.
fn take_intent(loan: bool) -> Intent {
    Intent {
        kind: Some(if loan { "loan.take" } else { "offer.take" }.to_owned()),
        recipients: None,
        fee: None,
        net: None,
    }
}

/// Everything after Sage built the take: the cross-check, then wallet-kit's submit with the
/// maker's signature (binding, intent, limits, one approval, sign, broadcast). Separate so a
/// real take can be run through it in tests without a running wallet.
///
/// `maker_coins` are the coins of the offer's own spends ([`maker_coins`]).
#[allow(clippy::too_many_arguments)]
fn submit_take(
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    sage_summary: &TransactionSummary,
    spends: Vec<CoinSpend>,
    maker_coins: &HashSet<Bytes32>,
    maker_signature: Signature,
    fee: u64,
    on_loan: &dyn Fn(&options::LoanView, bool),
) -> Result<String, RpcError> {
    // Sage spends the wallet's coins with quoted conditions; held to it all the same. Only
    // the spends Sage built for the user: the offer's own spends are the website's, and a
    // refusal of one of them would say which keys the wallet holds. Nothing in them is
    // signed for the user's keys without passing here: any signature they need must already
    // be in the maker's, which `check_maker_signature` verified before the build.
    let user_spends: Vec<CoinSpend> = spends
        .iter()
        .filter(|spend| !maker_coins.contains(&spend.coin.coin_id()))
        .cloned()
        .collect();
    crate::sign_guard::check_spends(&user_spends, ctx.keys, &ctx.network)?;

    let kit_summary = kit::simulate(&spends, ctx.ownership, kit::simulate::DEFAULT_MAX_COST)
        .map_err(RpcError::from)?;
    let loan = options::take_loan(&kit_summary, &spends, ctx.ownership)?;
    if let Some(loan) = &loan {
        on_loan(loan, false);
    }
    let intent = take_intent(loan.is_some());
    // Whatever the website's "unknown contracts" setting: a take moves assets between two
    // parties, and the approval must be able to say which.
    if !kit_summary.unknown_puzzles.is_empty() {
        return Err(error(
            codes::UNSUPPORTED_CONTENT,
            "unknown contract",
            Some("unknown_contract"),
        ));
    }
    cross_check(sage_summary, &kit_summary)?;

    let submitted = kit::submit(
        SubmitRequest {
            method: CHIA_TAKE_OFFER,
            spends,
            other_signature: Some(maker_signature),
            intent: Some(&intent),
            wallet_built: true,
            // The fee Klimper built the take with ([`wallet_fee`]): the user's spends
            // reserve it (a maker's own fee is reserved by the maker's spends).
            wallet_fee: fee,
            // Sealed, as for a send: only the transaction id goes back to the website.
            signatures_released: false,
        },
        ctx,
        signer,
        approver,
        broadcaster,
    )?;
    Ok(submitted.to_json())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod aggregate_tests;
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod option_tests;
#[cfg(test)]
mod sage_sim;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod take_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sage_api::{TransactionInput, TransactionOutput};
    use xchonnect_wallet_kit::simulate::{OutputInfo, SpendInfo};
    use xchonnect_wallet_kit::{AssetDelta, SpendKind};

    const COIN: &str = "11";

    fn output(amount: u64, puzzle_hash: &str, to_user: bool) -> OutputInfo {
        let mut output = OutputInfo::default();
        output.asset = AssetId::Xch;
        output.amount = amount;
        output.puzzle_hash = puzzle_hash.to_owned();
        output.recipient = Some(puzzle_hash.to_owned());
        output.to_user = to_user;
        output
    }

    fn kit_summary(sent: u128, paid: u64, change: u64, fee: u64) -> Summary {
        let mut summary = Summary::default();
        summary.assets = vec![AssetDelta {
            asset: AssetId::Xch,
            sent,
            received: u128::from(change),
            conditional_received: 0,
            net: i128::from(change) - i128::try_from(sent).unwrap(),
        }];
        summary.implied_fee = Some(fee);
        summary.reserve_fee = fee;
        summary.owned_reserve_fee = fee;
        summary.spends = vec![SpendInfo {
            coin_id: COIN.repeat(32),
            puzzle_hash: "22".repeat(32),
            amount: u64::try_from(sent).unwrap(),
            kind: SpendKind::Standard,
            owned: true,
            agg_sigs: vec![],
            cost: 0,
        }];
        summary.outputs = vec![
            output(paid, &"33".repeat(32), false),
            output(change, &"22".repeat(32), true),
        ];
        summary
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

    fn sage_summary(input: u64, paid: u64, change: u64, fee: u64) -> TransactionSummary {
        let output = |amount: u64, receiving: bool| TransactionOutput {
            coin_id: "0x00".into(),
            amount: Amount::u64(amount),
            address: "xch1…".into(),
            receiving,
            burning: false,
        };
        TransactionSummary {
            fee: Amount::u64(fee),
            inputs: vec![TransactionInput {
                coin_id: format!("0x{}", COIN.repeat(32)),
                amount: Amount::u64(input),
                address: "xch1…".into(),
                asset: Some(xch()),
                outputs: vec![output(paid, false), output(change, true)],
            }],
        }
    }

    #[test]
    fn take_offer_is_built_by_the_wallet_like_a_send() {
        assert!(is_wallet_built(CHIA_SEND));
        assert!(is_wallet_built(CHIA_TAKE_OFFER));
        assert!(!is_wallet_built("chip0002_signCoinSpends"));
        assert!(!is_wallet_built(kit::SUBMIT_COIN_SPENDS));
    }

    /// The maker's signature is the offer's own aggregated signature, untouched: it is what
    /// completes the bundle after the user signs their side.
    #[test]
    fn the_offer_decodes_with_the_makers_signature() {
        let maker = chia_bls::SecretKey::from_seed(&[7; 32]);
        let signature = chia_bls::sign(&maker, b"maker");
        let offer =
            chia_wallet_sdk::driver::encode_offer(&SpendBundle::new(vec![], signature.clone()))
                .unwrap();
        assert_eq!(
            decode_offer_bundle(&offer).unwrap().aggregated_signature,
            signature
        );
    }

    #[test]
    fn a_take_without_a_readable_offer_is_refused_before_the_wallet_is_asked() {
        for offer in ["", "offer1notanoffer", "hello"] {
            let error = decode_offer_bundle(offer).unwrap_err();
            assert_eq!(error.code, codes::INVALID_PARAMS, "{offer:?}");
        }
    }

    /// The fee of a sealed request is Klimper's own, the user's default fee: a website's
    /// `fee` is read (a malformed one is refused) and ignored, whatever it is (xchonnect
    /// spec 11.1.1, 11.2). The default fee is capped at `MAX_FEE`.
    #[test]
    fn a_websites_fee_is_ignored_for_klimpers_own() {
        let fee = |params: Value| fee_of(&params, 2_000_000).unwrap();
        assert_eq!(fee(serde_json::json!({})), 2_000_000);
        assert_eq!(fee(serde_json::json!({ "fee": 0 })), 2_000_000);
        assert_eq!(fee(serde_json::json!({ "fee": 50_000_000 })), 2_000_000);
        assert_eq!(fee(serde_json::json!({ "fee": "1000000000" })), 2_000_000);
        assert!(fee_of(&serde_json::json!({ "fee": -1 }), 0).is_err());
        assert_eq!(
            fee_of(&serde_json::json!({ "fee": 50_000_000 }), 0).unwrap(),
            0
        );
        // The default fee as the webview hands it over: capped.
        assert_eq!(capped_fee(10_000_000_000_000), MAX_FEE);
        assert_eq!(capped_fee(2_000_000), 2_000_000);
    }

    /// The shape of a real offer take (found by taking an offer Sage built between two
    /// simulator wallets, TASK-137): the user's 1000-CAT coin creates a 0-amount coin and
    /// the CAT ring moves the value on to the maker's settlement payment. Sage leaves the
    /// zero out of what is paid, the simulation lists it; both mean nothing was paid
    /// directly, so the two engines agree.
    /// A maker's own fee is part of the bundle's fee, so a take must not claim the user's fee
    /// as the whole: wallet-kit would refuse it as "unauthorized" (`intent_mismatch`), which
    /// is what every offer with a maker fee got.
    #[test]
    fn a_take_claims_no_fee_of_its_own() {
        let intent = take_intent(false);
        assert_eq!(intent.kind.as_deref(), Some("offer.take"));
        assert_eq!(intent.fee, None);
        assert!(intent.recipients.is_none() && intent.net.is_none());
    }

    #[test]
    fn a_zero_amount_coin_of_an_offer_take_is_no_disagreement() {
        let cat = Bytes32::new([0x75; 32]);
        let mut kit = kit_summary(1_000, 0, 0, 0);
        kit.assets[0].asset = AssetId::Cat(cat);
        let mut zero = output(0, &"21".repeat(32), false);
        zero.asset = AssetId::Cat(cat);
        zero.recipient = None;
        kit.outputs = vec![zero];
        let mut sage = sage_summary(1_000, 0, 0, 0);
        sage.inputs[0].asset = Some(sage_api::Asset {
            asset_id: Some(hex::encode(cat)),
            ..xch()
        });
        sage.inputs[0].outputs = vec![TransactionOutput {
            coin_id: "84".repeat(32),
            amount: Amount::u64(0),
            address: String::new(),
            receiving: false,
            burning: false,
        }];
        assert!(cross_check(&sage, &kit).is_ok());
    }

    /// Ignoring zeros must not hide a real payment: an amount on one side only still refuses.
    #[test]
    fn a_payment_only_one_engine_sees_still_refuses() {
        let kit = kit_summary(1_000, 300, 700, 0);
        let sage = sage_summary(1_000, 0, 1_000, 0);
        assert!(cross_check(&sage, &kit).is_err());
    }

    #[test]
    fn the_two_engines_agreeing_lets_the_request_through() {
        assert!(
            cross_check(
                &sage_summary(1000, 700, 290, 10),
                &kit_summary(1000, 700, 290, 10)
            )
            .is_ok()
        );
    }

    #[test]
    fn any_disagreement_about_what_leaves_the_wallet_refuses() {
        let kit = kit_summary(1000, 700, 290, 10);
        for sage in [
            sage_summary(1000, 70, 920, 10),   // a different payment
            sage_summary(2000, 700, 1290, 10), // different coins spent
            sage_summary(1000, 700, 290, 1),   // a different fee
        ] {
            let refused = cross_check(&sage, &kit).unwrap_err();
            assert!(refused.data.unwrap().contains("crosscheck_mismatch"));
        }
    }

    #[test]
    fn coin_spends_survive_the_round_trip_through_sages_json() {
        let cs = CoinSpend {
            coin: Coin::new(
                Bytes32::new([1; 32]),
                Bytes32::new([2; 32]),
                1_500_000_000_000,
            ),
            puzzle_reveal: Program::from(vec![0xff, 0x01, 0x80]),
            solution: Program::from(vec![0x80]),
        };
        assert_eq!(from_json(&to_json(&cs)).unwrap(), cs);
    }
}

/// Recent sends a website asked for that could not be built, per verified domain, in memory.
/// After [`MAX`](failed_builds::MAX) within [`WINDOW_S`](failed_builds::WINDOW_S) its sends
/// are refused without building, so the answers stop telling it anything about the balance.
mod failed_builds {
    use std::collections::{HashMap, VecDeque};
    use std::sync::{LazyLock, Mutex};

    pub const MAX: usize = 3;
    pub const WINDOW_S: u64 = 600;

    static RECENT: LazyLock<Mutex<HashMap<String, VecDeque<u64>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    fn pruned(recent: &mut VecDeque<u64>, now: u64) {
        while recent
            .front()
            .is_some_and(|at| now.saturating_sub(*at) >= WINDOW_S)
        {
            recent.pop_front();
        }
    }

    pub fn record(dapp: &str, now: u64) {
        if let Ok(mut all) = RECENT.lock() {
            let recent = all.entry(dapp.to_owned()).or_default();
            pruned(recent, now);
            recent.push_back(now);
        }
    }

    pub fn too_many(dapp: &str, now: u64) -> bool {
        RECENT.lock().map_or(true, |mut all| {
            all.get_mut(dapp).is_some_and(|recent| {
                pruned(recent, now);
                recent.len() >= MAX
            })
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_run_of_failed_builds_closes_the_door_for_a_while() {
            let dapp = "probe.example";
            for at in 0..MAX as u64 {
                assert!(!too_many(dapp, 1_000 + at));
                record(dapp, 1_000 + at);
            }
            assert!(too_many(dapp, 1_010));
            assert!(!too_many("other.example", 1_010));
            assert!(!too_many(dapp, 1_000 + WINDOW_S + MAX as u64));
        }
    }
}
