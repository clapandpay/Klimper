// SPDX-License-Identifier: AGPL-3.0-or-later
//! Option contracts over Xchonnect (TASK-140): Klimper writes, uses, claims back and sends
//! options, and requests and takes loans, from the wallet's own coins with Sage's option
//! code, the way it sends (`chia_send`) and takes offers (`chia_takeOffer`).
//!
//! An option (the Chia Wallet SDK's option primitives) is a singleton that commits to one
//! locked **collateral** coin: before the expiry whoever holds the option may pay the
//! **strike** to its **creator** and receive the collateral; from the expiry on the creator
//! takes the collateral back. wallet-kit recognises these spends and states them as facts it
//! computed (xchonnect spec 11.1.1), and verifies the intent kinds used here.
//!
//! * `chia_mintOption` locks the collateral and writes an option the wallet holds, with the
//!   wallet as creator (Sage's `mint_option`). With `creatorAddress` set to someone else and
//!   a `loan` principal it is a **loan request**: the creator is the lender, the option (the
//!   right to repay) stays with the borrower, and nothing is sent: Klimper signs an offer
//!   that asks the lender for the principal, which the website hands to the lender.
//! * `chia_exerciseOption` pays the strike and receives the collateral (Sage's
//!   `exercise_options`), only before the expiry.
//! * `chia_claimOption` takes the collateral back after the expiry; Sage has no such call,
//!   so it is built here from the SDK's clawback spend.
//! * `chia_transferOption` sends the option to an address (Sage's `transfer_options`).
//!
//! Every one is built key-free, cross-checked against Sage's own summary, submitted through
//! wallet-kit with a verified intent kind, and asks the user once.

use chia_wallet_sdk::{
    chia::puzzle_types::offer::{NotarizedPayment, Payment},
    driver::{
        Action, AssetInfo, Cat, CatAssetInfo, CatSpend, Id, Offer, OptionType, RequestedPayments,
        SpendContext, SpendKind, SpendWithConditions, Spends, StandardLayer,
    },
    prelude::{Conditions, Memos},
    puzzles::SETTLEMENT_PAYMENT_HASH,
};
use sage_database::{CoinKind, P2Puzzle};
use sage_wallet::{Wallet, WalletError, WalletPeer};
use serde::Serialize;
use xchonnect_wallet_kit::option::OptionEffect;

use super::*;

pub const CHIA_MINT_OPTION: &str = "chia_mintOption";
pub const CHIA_EXERCISE_OPTION: &str = "chia_exerciseOption";
pub const CHIA_CLAIM_OPTION: &str = "chia_claimOption";
pub const CHIA_TRANSFER_OPTION: &str = "chia_transferOption";

/// The option methods. Not granted to a connection unless the user turns them on for it
/// (on the pairing screen or in the website's settings): they lock funds until a date, so a
/// connection paired before they existed does not get them by itself either.
pub const OPTION_METHODS: [&str; 4] = [
    CHIA_MINT_OPTION,
    CHIA_EXERCISE_OPTION,
    CHIA_CLAIM_OPTION,
    CHIA_TRANSFER_OPTION,
];

/// What a loan request offers whoever takes it, in mojos. wallet-kit binds a taker's spends
/// only to a settlement payment the taker receives (spec 11.2), and a loan gives the lender
/// nothing else at the start: this mojo is what ties the lender's principal to the
/// borrower's spends, the collateral lock among them.
pub const LOAN_TIE_MOJOS: u64 = 1;

/// The shortest and the longest time an option may run, in seconds.
const MIN_TERM_S: u64 = 60;
const MAX_TERM_S: u64 = 10 * 366 * 24 * 3600;

/// One asset and an amount of it, as a request names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    pub asset: AssetId,
    pub amount: u64,
}

impl Part {
    fn option_type(self) -> Option<OptionType> {
        match self.asset {
            AssetId::Xch => Some(OptionType::Xch {
                amount: self.amount,
            }),
            AssetId::Cat(asset_id) => Some(OptionType::Cat {
                asset_id,
                amount: self.amount,
            }),
            _ => None,
        }
    }

    /// The Wallet SDK's id of the asset; `None` for an asset this version does not know.
    fn id(self) -> Option<Id> {
        match self.asset {
            AssetId::Cat(asset_id) | AssetId::Option(asset_id) => Some(Id::Existing(asset_id)),
            AssetId::Xch => Some(Id::Xch),
            _ => None,
        }
    }

    fn leg(self) -> crate::trade::Leg {
        crate::trade::Leg {
            asset: self.asset,
            amount: u128::from(self.amount),
        }
    }
}

/// Klimper's own key `loan` on a loan's approval, next to `trade` (computed by the wallet,
/// nothing from the website): who lends what to whom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LoanView {
    /// `borrower` (a loan request) or `lender` (taking one).
    pub role: &'static str,
    /// What the borrower receives and the lender pays.
    pub principal: crate::trade::Leg,
    /// The other side: the lender's puzzle hash for a borrower, the borrower's for a lender.
    #[serde(serialize_with = "hex_bytes")]
    pub counterparty: Bytes32,
    /// What the request offers the lender to tie the loan together ([`LOAN_TIE_MOJOS`]).
    pub tie: Option<crate::trade::Leg>,
}

fn hex_bytes<S: serde::Serializer>(value: &Bytes32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&hex::encode(value))
}

/// Why an option request is refused before anything is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The wallet holds no option with this id (or knows none to claim).
    UnknownOption,
    /// Too late to use the option.
    Expired,
    /// Too early to claim the collateral back.
    NotExpired,
    /// The collateral belongs to another creator.
    NotCreator,
    /// The fee is more than the collateral it would come out of.
    Fee,
}

impl Refusal {
    fn rpc(self) -> RpcError {
        match self {
            Refusal::UnknownOption => invalid("unknown_option"),
            Refusal::Fee => invalid("fee"),
            Refusal::Expired => error(
                codes::UNAUTHORIZED,
                "this option has expired",
                Some("option_expired"),
            ),
            Refusal::NotExpired => error(
                codes::UNAUTHORIZED,
                "this option has not expired yet",
                Some("option_not_expired"),
            ),
            Refusal::NotCreator => error(
                codes::UNAUTHORIZED,
                "this wallet did not write this option",
                Some("not_creator"),
            ),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------------------

fn object<'a>(params: &'a Value, key: &str) -> Result<&'a Value, RpcError> {
    params
        .get(key)
        .filter(|v| v.is_object())
        .ok_or_else(|| invalid(key))
}

fn asset_param(value: Option<&Value>, key: &str) -> Result<AssetId, RpcError> {
    match value {
        None | Some(Value::Null) => Ok(AssetId::Xch),
        Some(Value::String(s)) if s.is_empty() || s.eq_ignore_ascii_case("xch") => Ok(AssetId::Xch),
        Some(Value::String(s)) => unhex(s)
            .and_then(|bytes| Bytes32::try_from(bytes.as_slice()).ok())
            .map(AssetId::Cat)
            .ok_or_else(|| invalid(key)),
        Some(_) => Err(invalid(key)),
    }
}

/// `{ assetId?, amount }`: XCH when `assetId` is absent or `"xch"`, else a CAT.
fn part_param(params: &Value, key: &str) -> Result<Part, RpcError> {
    let value = object(params, key)?;
    let amount = amount_param(value, "amount")
        .map_err(|_| invalid(key))?
        .filter(|amount| *amount > 0)
        .ok_or_else(|| invalid(key))?;
    Ok(Part {
        asset: asset_param(value.get("assetId"), key)?,
        amount,
    })
}

fn launcher_param(params: &Value) -> Result<Bytes32, RpcError> {
    params
        .get("launcherId")
        .and_then(Value::as_str)
        .and_then(unhex)
        .and_then(|bytes| Bytes32::try_from(bytes.as_slice()).ok())
        .ok_or_else(|| invalid("launcherId"))
}

/// What `chia_mintOption` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintParams {
    pub collateral: Part,
    pub strike: Part,
    pub expires_at: u64,
    pub creator_address: Option<String>,
    pub loan: Option<Part>,
    pub fee: u64,
}

pub fn mint_params(params_json: &str, now: u64) -> Result<MintParams, RpcError> {
    let params: Value = serde_json::from_str(params_json).map_err(|_| invalid("json"))?;
    let collateral = part_param(&params, "underlying")?;
    let strike = part_param(&params, "strike")?;
    // An option on itself pays nothing: a slip, not a contract anyone means.
    if collateral.asset == strike.asset {
        return Err(invalid("strike"));
    }
    let expires_at = amount_param(&params, "expiresAt")?
        .filter(|at| *at >= now.saturating_add(MIN_TERM_S))
        .filter(|at| *at <= now.saturating_add(MAX_TERM_S))
        .ok_or_else(|| invalid("expiresAt"))?;
    let creator_address = match params.get("creatorAddress") {
        None | Some(Value::Null) => None,
        Some(Value::String(address)) => Some(address.clone()),
        Some(_) => return Err(invalid("creatorAddress")),
    };
    let loan = match params.get("loan") {
        None | Some(Value::Null) => None,
        Some(_) => Some(part_param(&params, "loan")?),
    };
    Ok(MintParams {
        collateral,
        strike,
        expires_at,
        creator_address,
        loan,
        fee: amount_param(&params, "fee")?.unwrap_or(0),
    })
}

// ---------------------------------------------------------------------------------------
// Shared steps
// ---------------------------------------------------------------------------------------

fn build_failed(what: &str, ctx: &RequestContext<'_>, e: &str) -> RpcError {
    // Sage's own words stay here, as for a send: they name balances and coins.
    tracing::info!("xchonnect: {what} could not be built: {e}");
    failed_builds::record(ctx.dapp, ctx.now);
    error(
        codes::SPENDABLE_BALANCE_EXCEEDED,
        "the wallet could not build this request",
        Some("build_failed"),
    )
}

fn too_many_failed(ctx: &RequestContext<'_>) -> Result<(), RpcError> {
    if failed_builds::too_many(ctx.dapp, ctx.now) {
        return Err(error(
            codes::SPENDABLE_BALANCE_EXCEEDED,
            "the wallet could not build this request",
            Some("build_failed"),
        ));
    }
    Ok(())
}

/// Sage's own summary of `spends`, for the cross-check.
fn sage_view(state: &AppState, spends: &[CoinSpend]) -> Result<TransactionSummary, RpcError> {
    let request = ViewCoinSpends {
        coin_spends: spends.iter().map(to_json).collect(),
    };
    on_sage(state, async move |sage| {
        Ok(sage.view_coin_spends(request).await?.summary)
    })
    .map_err(|e| {
        tracing::warn!("xchonnect: the wallet could not summarise an option request: {e}");
        error(
            codes::UNSUPPORTED_CONTENT,
            "the wallet could not read this transaction",
            Some("crosscheck_unavailable"),
        )
    })
}

/// The answer to a website: wallet-kit's result with the option's id.
fn with_launcher(answer: &str, launcher_id: Bytes32) -> String {
    let mut value: Value =
        serde_json::from_str(answer).unwrap_or_else(|_| Value::Object(serde_json::Map::default()));
    if let Some(fields) = value.as_object_mut() {
        fields.insert("launcherId".to_owned(), Value::String(hex0x(launcher_id)));
    }
    value.to_string()
}

/// The intent of a wallet-built option request: the kind wallet-kit verifies, the fee, and
/// who is paid (`None`: not claimed).
fn intent(kind: &str, recipients: Option<Vec<RecipientClaim>>, fee: u64) -> Intent {
    Intent {
        kind: Some(kind.to_owned()),
        recipients,
        fee: Some(fee),
        net: None,
    }
}

/// Simulate, cross-check, submit through wallet-kit. `signatures_released`: the signed
/// bundle goes back to the website (a loan request's offer); otherwise Klimper broadcasts it
/// and answers only its id and status, and the request is sealed (xchonnect spec 11.2).
#[allow(clippy::too_many_arguments)]
fn checked_submit(
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    sage_summary: &TransactionSummary,
    method: &'static str,
    spends: Vec<CoinSpend>,
    intent: &Intent,
    signatures_released: bool,
) -> Result<kit::handlers::Submitted, RpcError> {
    refuse_in_flight(&spends, ctx)?;
    let kit_summary = kit::simulate(&spends, ctx.ownership, kit::simulate::DEFAULT_MAX_COST)
        .map_err(RpcError::from)?;
    cross_check(sage_summary, &kit_summary)?;
    kit::submit(
        SubmitRequest {
            method,
            spends,
            other_signature: None,
            intent: Some(intent),
            wallet_built: true,
            signatures_released,
        },
        ctx,
        signer,
        approver,
        broadcaster,
    )
}

// ---------------------------------------------------------------------------------------
// chia_mintOption
// ---------------------------------------------------------------------------------------

/// The option written: its spends and its id, and for a loan request what its offer needs.
#[derive(Debug)]
pub struct Minted {
    pub spends: Vec<CoinSpend>,
    pub launcher_id: Bytes32,
    /// The offer's nonce, from the borrower's coins (a loan request).
    pub nonce: Bytes32,
    /// The wallet's own address: the option's owner, where a loan's principal is paid.
    pub receive: Bytes32,
}

/// The coin one of `spends` creates at `puzzle_hash` with `amount`, if any.
fn created_coin(spends: &[CoinSpend], puzzle_hash: Bytes32, amount: u64) -> Option<Coin> {
    use chia_wallet_sdk::prelude::{Allocator, Condition, FromClvm, ToClvm};
    let mut allocator = Allocator::new();
    spends.iter().find_map(|spend| {
        let puzzle = spend.puzzle_reveal.to_clvm(&mut allocator).ok()?;
        let solution = spend.solution.to_clvm(&mut allocator).ok()?;
        let output = chia_wallet_sdk::types::run_puzzle(&mut allocator, puzzle, solution).ok()?;
        let conditions = Vec::<Condition>::from_clvm(&allocator, output).ok()?;
        conditions
            .into_iter()
            .find_map(|condition| match condition {
                Condition::CreateCoin(cc)
                    if cc.puzzle_hash == puzzle_hash && cc.amount == amount =>
                {
                    Some(Coin::new(spend.coin.coin_id(), puzzle_hash, amount))
                }
                _ => None,
            })
    })
}

/// Add `conditions` to every spend of the wallet's own coins in `spends`, before Sage's
/// `complete_spends` turns them into coin spends.
fn add_to_every_spend(spends: &mut Spends, conditions: &Conditions) {
    let xch = spends.xch.items.iter_mut().map(|item| &mut item.kind);
    let cats = spends
        .cats
        .values_mut()
        .flat_map(|cat| cat.items.iter_mut().map(|item| &mut item.kind));
    for kind in xch.chain(cats) {
        if let SpendKind::Conditions(spend) = kind {
            spend.add_conditions(conditions.clone());
        }
    }
}

/// Lock the collateral and write the option, key-free, from the wallet's own coins with
/// Sage's coin selection and spend code and the SDK's option launcher, for a **loan
/// request** (Sage's own `mint_option`, [`build_sage_mint`], cannot write an option whose
/// creator is the lender):
///
/// 1. the fee and the singleton launcher's coin (hinted with the creator), and for a loan
///    request the tie to the lender. XCH collateral is set aside here too, at another of the
///    wallet's addresses, and locked from there in step 2: the wallet may hold a single coin;
/// 2. the collateral, locked at the option's collateral puzzle (which commits to the
///    launcher, so only now can it be built): XCH from the coin step 1 set aside, a CAT from
///    the wallet's CAT coins;
/// 3. the launch: the eve option's inner puzzle is a quoted program that hands the option to
///    the wallet, so its outputs are fixed.
///
/// A loan request's signed spends go to the lender as an offer, so its signatures are
/// released and only direct assertions bind it (xchonnect spec 11.2, "Loan requests"):
/// **every** spend of the borrower asserts the settlement announcement of the principal paid
/// to the borrower, and every spend that locks the collateral asserts the launcher's
/// announcement, so the launch is guaranteed (spec 11.1.1 item 1). These are conditions of
/// Klimper's own build, given to each spend before Sage's `complete_spends` makes the coin
/// spends; nothing Sage built is changed afterwards. The launcher's announcement commits to
/// the collateral coin, which only Sage's build of step 2 names: step 2 is built once to
/// find it and once more, from the same coins, with the assertion.
///
/// `exclude`: coins not to spend (the coins of a sealed bundle still in flight).
pub async fn build_mint(
    wallet: &Wallet,
    creator: Option<Bytes32>,
    params: &MintParams,
    loan: Option<Part>,
    exclude: &[Bytes32],
) -> Result<Option<Minted>, WalletError> {
    use chia_wallet_sdk::driver::{OptionLauncher, OptionLauncherInfo};
    use chia_wallet_sdk::prelude::Condition;
    use chia_wallet_sdk::puzzles::SINGLETON_LAUNCHER_HASH;

    let (Some(strike_type), Some(_)) =
        (params.strike.option_type(), params.collateral.option_type())
    else {
        return Ok(None);
    };
    let receive = wallet.change_p2_puzzle_hash().await?;
    let creator = creator.unwrap_or(receive);
    let launcher_hash = Bytes32::from(SINGLETON_LAUNCHER_HASH);
    let mut ctx = SpendContext::new();

    // 1. The fee, the launcher's coin, for a loan the tie, and the XCH collateral set aside.
    let hint = ctx.hint(creator)?;
    let mut actions = vec![
        Action::fee(params.fee),
        Action::send(Id::Xch, launcher_hash, 1, hint),
    ];
    if loan.is_some() {
        actions.push(Action::send(
            Id::Xch,
            SETTLEMENT_PAYMENT_HASH.into(),
            LOAN_TIE_MOJOS,
            Memos::None,
        ));
    }
    let set_aside = if params.collateral.asset == AssetId::Xch {
        let Some(aside) = wallet
            .db
            .custody_p2_puzzle_hashes()
            .await?
            .into_iter()
            .find(|ph| *ph != receive)
        else {
            return Ok(None);
        };
        actions.push(Action::send(
            Id::Xch,
            aside,
            params.collateral.amount,
            Memos::None,
        ));
        Some(aside)
    } else {
        None
    };
    let mut first = wallet.prepare_spends_for_selection(&mut ctx, &[]).await?;
    wallet
        .select_spends_excluding(&mut ctx, &mut first, &actions, exclude)
        .await?;
    let first_ids = first.non_settlement_coin_ids();
    let nonce = Offer::nonce(first_ids.clone());
    // What every spend of the borrower asserts: the principal paid to the borrower, before
    // the option expires.
    let mut asserted = Conditions::new();
    if let Some(principal) = loan {
        let (requested, asset_info) = loan_requested(&mut ctx, nonce, receive, principal)?;
        asserted = asserted
            .extend(requested.assertions(&mut ctx, &asset_info)?)
            .assert_before_seconds_absolute(params.expires_at);
    }
    let deltas = first.apply(&mut ctx, &actions)?;
    add_to_every_spend(&mut first, &asserted);
    wallet.complete_spends(&mut ctx, &deltas, first).await?;
    let mut spends = ctx.take();
    let Some(launcher_coin) = created_coin(&spends, launcher_hash, 1) else {
        return Ok(None);
    };
    let (_, launcher) = OptionLauncher::create_early(
        &mut ctx,
        launcher_coin.parent_coin_info,
        1,
        OptionLauncherInfo::new(
            creator,
            receive,
            params.expires_at,
            params.collateral.amount,
            strike_type,
        ),
        1,
    )?;
    if launcher.launcher().coin() != launcher_coin {
        return Ok(None);
    }

    // 2. The collateral, locked at the option's collateral puzzle, from other coins.
    let locked_at = launcher.p2_puzzle_hash();
    let Some(collateral_id) = params.collateral.id() else {
        return Ok(None);
    };
    let actions = [Action::send(
        collateral_id,
        locked_at,
        params.collateral.amount,
        Memos::None,
    )];
    let mut second = wallet.prepare_spends_for_selection(&mut ctx, &[]).await?;
    if let Some(aside) = set_aside {
        let Some(coin) = created_coin(&spends, aside, params.collateral.amount) else {
            return Ok(None);
        };
        second.add(coin);
    } else {
        let excluded: Vec<Bytes32> = first_ids.iter().chain(exclude).copied().collect();
        wallet
            .select_spends_excluding(&mut ctx, &mut second, &actions, &excluded)
            .await?;
    }
    let collateral_puzzle_hash = match params.collateral.asset {
        AssetId::Cat(asset_id) => Bytes32::from(
            chia_wallet_sdk::driver::CatInfo::new(asset_id, None, locked_at).puzzle_hash(),
        ),
        _ => locked_at,
    };
    // Which coin the collateral is: from a first build of step 2, set aside.
    let mut trial = second.clone();
    let deltas = trial.apply(&mut ctx, &actions)?;
    wallet.complete_spends(&mut ctx, &deltas, trial).await?;
    let Some(collateral) = created_coin(
        &ctx.take(),
        collateral_puzzle_hash,
        params.collateral.amount,
    ) else {
        return Ok(None);
    };

    // 3. The launch, with the eve's outputs fixed; the announcement every locking spend
    //    asserts.
    let launcher_id = launcher.launcher().coin().coin_id();
    let (launch, _option) = launcher
        .with_underlying(collateral.coin_id())
        .mint(&mut ctx)?;
    let launch: Vec<Bytes32> = launch
        .into_iter()
        .filter_map(|condition| match condition {
            Condition::AssertCoinAnnouncement(assert) => Some(assert.announcement_id),
            _ => None,
        })
        .collect();
    let [launch] = launch.as_slice() else {
        return Ok(None);
    };
    let deltas = second.apply(&mut ctx, &actions)?;
    add_to_every_spend(
        &mut second,
        &asserted.clone().assert_coin_announcement(*launch),
    );
    wallet.complete_spends(&mut ctx, &deltas, second).await?;
    let rest = ctx.take();
    // The same coins, the same collateral coin.
    if created_coin(&rest, collateral_puzzle_hash, params.collateral.amount) != Some(collateral) {
        return Ok(None);
    }
    spends.extend(rest);
    Ok(Some(Minted {
        spends,
        launcher_id,
        nonce,
        receive,
    }))
}

/// A plain option, written with Sage's own `mint_option`, unchanged: the wallet is the
/// creator (its change address) and holds the option. Sage is the source of truth for the
/// spends it builds; Klimper adds nothing to them.
pub async fn build_sage_mint(
    wallet: &Wallet,
    params: &MintParams,
) -> Result<Option<Minted>, WalletError> {
    let (Some(strike_type), Some(underlying_type)) =
        (params.strike.option_type(), params.collateral.option_type())
    else {
        return Ok(None);
    };
    let receive = wallet.change_p2_puzzle_hash().await?;
    let (spends, option) = wallet
        .mint_option(
            sage_wallet::WalletOptionMint {
                expiration_seconds: params.expires_at,
                underlying_type,
                strike_type,
            },
            params.fee,
        )
        .await?;
    Ok(Some(Minted {
        spends,
        launcher_id: option.info.launcher_id,
        nonce: Bytes32::default(),
        receive,
    }))
}

/// `chia_mintOption`: a plain option (the wallet is the creator and holds it), or with a
/// lender as creator and a `loan` principal, a loan request returned as an offer.
#[allow(clippy::too_many_arguments)]
pub fn mint_option(
    state: &AppState,
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    params_json: &str,
    on_loan: &dyn Fn(&LoanView, bool),
) -> Result<String, RpcError> {
    too_many_failed(ctx)?;
    let params = mint_params(params_json, ctx.now)?;
    let creator = match &params.creator_address {
        None => None,
        Some(address) => {
            let address = address.clone();
            Some(
                on_sage(state, async move |sage| sage.parse_address(address))
                    .map_err(|_| invalid("creatorAddress"))?,
            )
        }
    };
    let lender = creator.filter(|ph| !ctx.ownership.p2_puzzle_hashes.contains(ph));
    match (lender, params.loan) {
        (Some(lender), Some(principal)) => {
            return loan_request(
                state, ctx, signer, approver, &params, lender, principal, on_loan,
            );
        }
        // An option written for someone else is a loan, and a loan names its principal; a
        // principal without a lender is no loan at all.
        (Some(_), None) | (None, Some(_)) => return Err(invalid("loan")),
        (None, None) => {}
    }
    // The creator is the wallet itself: Sage writes with its change address as creator,
    // whichever of the wallet's own addresses the website named.
    let build_params = params.clone();
    let minted = on_sage(state, async move |sage| {
        Ok(build_sage_mint(&*sage.wallet()?, &build_params).await?)
    })
    .map_err(|e| build_failed(CHIA_MINT_OPTION, ctx, &e))?
    .ok_or_else(|| invalid("underlying"))?;
    let summary = sage_view(state, &minted.spends)?;
    submit_mint(
        ctx,
        signer,
        approver,
        broadcaster,
        &summary,
        minted,
        params.fee,
    )
}

/// After the build: what the user writes is an option they hold, nothing paid to anyone
/// (the collateral and the launcher are option outputs, not payments), this fee.
pub fn submit_mint(
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    sage_summary: &TransactionSummary,
    minted: Minted,
    fee: u64,
) -> Result<String, RpcError> {
    let intent = intent("option.mint", Some(Vec::new()), fee);
    let submitted = checked_submit(
        ctx,
        signer,
        approver,
        broadcaster,
        sage_summary,
        CHIA_MINT_OPTION,
        minted.spends,
        &intent,
        false,
    )?;
    Ok(with_launcher(&submitted.to_json(), minted.launcher_id))
}

// ---------------------------------------------------------------------------------------
// Loans
// ---------------------------------------------------------------------------------------

/// The payment a loan request asks of the lender: the principal to the borrower.
fn loan_requested(
    ctx: &mut SpendContext,
    nonce: Bytes32,
    receive: Bytes32,
    principal: Part,
) -> Result<(RequestedPayments, AssetInfo), chia_wallet_sdk::driver::DriverError> {
    let mut asset_info = AssetInfo::new();
    let mut requested = RequestedPayments::new();
    let hint = ctx.hint(receive)?;
    let payment = NotarizedPayment::new(nonce, vec![Payment::new(receive, principal.amount, hint)]);
    match principal.asset {
        AssetId::Xch => requested.xch.push(payment),
        AssetId::Cat(asset_id) => {
            asset_info.insert_cat(asset_id, CatAssetInfo::new(None))?;
            requested.cats.entry(asset_id).or_default().push(payment);
        }
        _ => return Err(chia_wallet_sdk::driver::DriverError::InvalidAssetId),
    }
    Ok((requested, asset_info))
}

/// Signs without sending: a loan request goes to the lender as an offer.
struct OfferOnly;

impl Broadcaster for OfferOnly {
    fn submit(&self, bundle: &SpendBundle) -> Result<Broadcast, String> {
        Ok(Broadcast {
            transaction_id: hex0x(bundle.name()),
            status: "not_sent".to_owned(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn loan_request(
    state: &AppState,
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    params: &MintParams,
    lender: Bytes32,
    principal: Part,
    on_loan: &dyn Fn(&LoanView, bool),
) -> Result<String, RpcError> {
    let build_params = params.clone();
    let in_flight: Vec<Bytes32> = ctx.signed_coins.in_flight.iter().copied().collect();
    let built = on_sage(state, async move |sage| {
        Ok(build_mint(
            &*sage.wallet()?,
            Some(lender),
            &build_params,
            Some(principal),
            &in_flight,
        )
        .await?)
    })
    .map_err(|e| build_failed("a loan request", ctx, &e))?
    .ok_or_else(|| invalid("underlying"))?;
    let summary = sage_view(state, &built.spends)?;
    submit_loan_request(
        ctx, signer, approver, &summary, built, lender, principal, params.fee, on_loan,
    )
}

/// After the build: the borrower signs their side (the verified kind `loan.request`), and the
/// signed spends become the offer the website hands to the lender. Nothing is broadcast.
#[allow(clippy::too_many_arguments)]
pub fn submit_loan_request(
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    sage_summary: &TransactionSummary,
    built: Minted,
    lender: Bytes32,
    principal: Part,
    fee: u64,
    on_loan: &dyn Fn(&LoanView, bool),
) -> Result<String, RpcError> {
    on_loan(
        &LoanView {
            role: "borrower",
            principal: principal.leg(),
            counterparty: lender,
            tie: Some(crate::trade::Leg {
                asset: AssetId::Xch,
                amount: u128::from(LOAN_TIE_MOJOS),
            }),
        },
        true,
    );
    let intent = intent("loan.request", None, fee);
    let submitted = checked_submit(
        ctx,
        signer,
        approver,
        &OfferOnly,
        sage_summary,
        CHIA_MINT_OPTION,
        built.spends,
        &intent,
        // The signed spends go to the lender as an offer: released (spec 11.2).
        true,
    )?;
    let offer = finish_loan_offer(submitted.bundle, built.nonce, built.receive, principal)
        .ok_or_else(|| invalid("offer"))?;
    Ok(serde_json::json!({
        "offer": offer,
        "launcherId": hex0x(built.launcher_id),
    })
    .to_string())
}

/// The signed borrower's spends and the payment they ask for, as an offer string.
fn finish_loan_offer(
    signed: SpendBundle,
    nonce: Bytes32,
    receive: Bytes32,
    principal: Part,
) -> Option<String> {
    let mut ctx = SpendContext::new();
    let (requested, asset_info) = loan_requested(&mut ctx, nonce, receive, principal).ok()?;
    let offer = Offer::from_input_spend_bundle(&mut ctx, signed, requested, asset_info).ok()?;
    let bundle = offer.to_spend_bundle(&mut ctx).ok()?;
    chia_wallet_sdk::driver::encode_offer(&bundle).ok()
}

/// What a take does with options, from the simulation: the loan the user lends (a mint whose
/// creator is the user, with someone else's collateral and option), or a refusal for a mint
/// that is neither the user's to hold nor to claim back: paying for it buys nothing.
pub fn take_loan(
    summary: &Summary,
    spends: &[CoinSpend],
    ownership: &kit::Ownership,
) -> Result<Option<LoanView>, RpcError> {
    let mut loan = None;
    for effect in &summary.options {
        let OptionEffect::Mint {
            owner,
            owner_is_user,
            creator_is_user,
            funded_by_user,
            ..
        } = effect
        else {
            continue;
        };
        if !owner_is_user && !creator_is_user {
            return Err(error(
                codes::UNAUTHORIZED,
                "this offer writes an option for someone else",
                Some("option_not_yours"),
            ));
        }
        if *creator_is_user && !owner_is_user && !funded_by_user {
            loan = Some(*owner);
        }
    }
    let Some(borrower) = loan else {
        return Ok(None);
    };
    // The principal: what the bundle's settlement payments pay the borrower, one asset.
    let mut paid = paid_through_settlement(spends, ownership, borrower)?.into_iter();
    let (Some(principal), None) = (paid.next(), paid.next()) else {
        return Ok(None);
    };
    Ok(Some(LoanView {
        role: "lender",
        principal,
        counterparty: borrower,
        tie: None,
    }))
}

/// What the settlement payments of `spends` pay to `to`, per asset, read from the
/// settlement spends' own solutions (as wallet-kit reads them for binding).
fn paid_through_settlement(
    spends: &[CoinSpend],
    ownership: &kit::Ownership,
    to: Bytes32,
) -> Result<Vec<crate::trade::Leg>, RpcError> {
    use chia_puzzle_types::cat::CatSolution;
    use chia_puzzle_types::offer::SettlementPaymentsSolution;
    use chia_wallet_sdk::prelude::{Allocator, FromClvm, NodePtr, ToClvm};

    let mut allocator = Allocator::new();
    let (executed, _) = kit::simulate::execute(
        &mut allocator,
        spends,
        ownership,
        kit::simulate::DEFAULT_MAX_COST,
    )
    .map_err(RpcError::from)?;
    let settlement = Bytes32::from(SETTLEMENT_PAYMENT_HASH);
    let mut paid: BTreeMap<AssetId, u128> = BTreeMap::new();
    for (spend, executed) in spends.iter().zip(&executed) {
        let Ok(solution) = spend.solution.to_clvm(&mut allocator) else {
            continue;
        };
        let (asset, inner) = match &executed.kind {
            kit::SpendKind::Settlement => (AssetId::Xch, solution),
            kit::SpendKind::Cat {
                asset_id,
                p2_puzzle_hash,
                hidden_puzzle_hash: None,
            } if *p2_puzzle_hash == settlement => {
                let Ok(cat) = CatSolution::<NodePtr>::from_clvm(&allocator, solution) else {
                    continue;
                };
                (AssetId::Cat(*asset_id), cat.inner_puzzle_solution)
            }
            _ => continue,
        };
        let Ok(parsed) = SettlementPaymentsSolution::<NodePtr>::from_clvm(&allocator, inner) else {
            continue;
        };
        for payment in parsed
            .notarized_payments
            .iter()
            .flat_map(|np| &np.payments)
            .filter(|p| p.puzzle_hash == to)
        {
            *paid.entry(asset).or_default() += u128::from(payment.amount);
        }
    }
    Ok(paid
        .into_iter()
        .map(|(asset, amount)| crate::trade::Leg { asset, amount })
        .collect())
}

// ---------------------------------------------------------------------------------------
// chia_exerciseOption
// ---------------------------------------------------------------------------------------

/// Pay the strike and receive the collateral, key-free (Sage's `exercise_options`), only
/// while the option runs.
pub async fn build_exercise(
    wallet: &Wallet,
    launcher_id: Bytes32,
    fee: u64,
    now: u64,
) -> Result<Result<Vec<CoinSpend>, Refusal>, WalletError> {
    if wallet.db.spendable_option(launcher_id).await?.is_none() {
        return Ok(Err(Refusal::UnknownOption));
    }
    let Some(underlying) = wallet.db.option_underlying(launcher_id).await? else {
        return Ok(Err(Refusal::UnknownOption));
    };
    if now >= underlying.seconds {
        return Ok(Err(Refusal::Expired));
    }
    Ok(Ok(wallet.exercise_options(vec![launcher_id], fee).await?))
}

pub fn exercise_option(
    state: &AppState,
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    params_json: &str,
) -> Result<String, RpcError> {
    too_many_failed(ctx)?;
    let params: Value = serde_json::from_str(params_json).map_err(|_| invalid("json"))?;
    let launcher_id = launcher_param(&params)?;
    let fee = amount_param(&params, "fee")?.unwrap_or(0);
    let now = ctx.now;
    let spends = on_sage(state, async move |sage| {
        Ok(build_exercise(&*sage.wallet()?, launcher_id, fee, now).await?)
    })
    .map_err(|e| build_failed(CHIA_EXERCISE_OPTION, ctx, &e))?
    .map_err(Refusal::rpc)?;
    // Sage's spends as Sage builds them: nothing is added (Sage is the source of truth).
    let summary = sage_view(state, &spends)?;
    submit_exercise(ctx, signer, approver, broadcaster, &summary, spends, fee)
}

/// After the build: wallet-kit requires the user's spends to require one another and the
/// user's guaranteed outcome to cover the collateral less the strike and the fee (spec
/// 11.1.1, `BindingReport::exercises_covered`), and verifies the kind. Sage's
/// `exercise_options` links its spends by `ASSERT_CONCURRENT_SPEND`, which wallet-kit counts
/// in a sealed request (Klimper broadcasts it and keeps the signatures, spec 11.2), so Sage's
/// spends pass as Sage builds them. Klimper does not add links to Sage's spends.
pub fn submit_exercise(
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    sage_summary: &TransactionSummary,
    spends: Vec<CoinSpend>,
    fee: u64,
) -> Result<String, RpcError> {
    let intent = intent("option.exercise", None, fee);
    let submitted = checked_submit(
        ctx,
        signer,
        approver,
        broadcaster,
        sage_summary,
        CHIA_EXERCISE_OPTION,
        spends,
        &intent,
        false,
    )?;
    Ok(submitted.to_json())
}

// ---------------------------------------------------------------------------------------
// chia_claimOption
// ---------------------------------------------------------------------------------------

/// Take the collateral back after the expiry, key-free. The option's terms come from the
/// wallet's database, or for an option the wallet never held (a loan it lent) from the chain
/// through `peer`, as Sage reads an option it is offered. The fee comes out of XCH
/// collateral, or from the wallet's XCH for a CAT.
#[allow(clippy::implicit_hasher)]
pub async fn build_claim(
    wallet: &Wallet,
    peer: Option<&WalletPeer>,
    owned: &std::collections::HashSet<Bytes32>,
    launcher_id: Bytes32,
    fee: u64,
    now: u64,
) -> Result<Result<Vec<CoinSpend>, Refusal>, WalletError> {
    let mut underlying = wallet.db.option_underlying(launcher_id).await?;
    if underlying.is_none()
        && let Some(peer) = peer
    {
        wallet
            .fetch_offer_option_info(Some(peer), launcher_id)
            .await?;
        underlying = wallet.db.option_underlying(launcher_id).await?;
    }
    let Some(underlying) = underlying else {
        return Ok(Err(Refusal::UnknownOption));
    };
    if !owned.contains(&underlying.creator_puzzle_hash) {
        return Ok(Err(Refusal::NotCreator));
    }
    if now < underlying.seconds {
        return Ok(Err(Refusal::NotExpired));
    }
    let Some(info) = wallet.db.offer_option_info(launcher_id).await? else {
        return Ok(Err(Refusal::UnknownOption));
    };
    let coin_id = info.underlying_coin_hash;
    let P2Puzzle::PublicKey(key) = wallet.db.p2_puzzle(underlying.creator_puzzle_hash).await?
    else {
        return Ok(Err(Refusal::NotCreator));
    };
    let creator = underlying.creator_puzzle_hash;
    let p2 = StandardLayer::new(key);
    let mut ctx = SpendContext::new();
    match wallet.db.underlying_coin_kind(launcher_id).await? {
        Some(CoinKind::Xch) => {
            let Some(coin) = wallet.db.xch_coin(coin_id).await? else {
                return Ok(Err(Refusal::UnknownOption));
            };
            let Some(back) = coin.amount.checked_sub(fee) else {
                return Ok(Err(Refusal::Fee));
            };
            let mut conditions = Conditions::new().create_coin(creator, back, Memos::None);
            if fee > 0 {
                conditions = conditions.reserve_fee(fee);
            }
            let inner = p2.spend_with_conditions(&mut ctx, conditions)?;
            underlying.clawback_coin_spend(&mut ctx, coin, inner)?;
        }
        Some(CoinKind::Cat) => {
            let Some(cat) = wallet.db.cat_coin(coin_id).await? else {
                return Ok(Err(Refusal::UnknownOption));
            };
            let hint = ctx.hint(creator)?;
            let inner = p2.spend_with_conditions(
                &mut ctx,
                Conditions::new().create_coin(creator, cat.coin.amount, hint),
            )?;
            let spend = underlying.clawback_spend(&mut ctx, inner)?;
            Cat::spend_all(&mut ctx, &[CatSpend::new(cat, spend)])?;
            if fee > 0 {
                wallet.spend(&mut ctx, vec![], &[Action::fee(fee)]).await?;
            }
        }
        _ => return Ok(Err(Refusal::UnknownOption)),
    }
    Ok(Ok(ctx.take()))
}

pub fn claim_option(
    state: &AppState,
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    params_json: &str,
) -> Result<String, RpcError> {
    too_many_failed(ctx)?;
    let params: Value = serde_json::from_str(params_json).map_err(|_| invalid("json"))?;
    let launcher_id = launcher_param(&params)?;
    let fee = amount_param(&params, "fee")?.unwrap_or(0);
    let now = ctx.now;
    let owned = ctx.ownership.p2_puzzle_hashes.clone();
    let spends = on_sage(state, async move |sage| {
        let peer = sage.peer_state.lock().await.acquire_peer();
        Ok(build_claim(
            &*sage.wallet()?,
            peer.as_ref(),
            &owned,
            launcher_id,
            fee,
            now,
        )
        .await?)
    })
    .map_err(|e| build_failed(CHIA_CLAIM_OPTION, ctx, &e))?
    .map_err(Refusal::rpc)?;
    let summary = sage_view(state, &spends)?;
    submit_claim(ctx, signer, approver, broadcaster, &summary, spends, fee)
}

/// After the build: everything comes back to the user (nothing paid to anyone), this fee,
/// the verified kind `option.clawback`.
pub fn submit_claim(
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    sage_summary: &TransactionSummary,
    spends: Vec<CoinSpend>,
    fee: u64,
) -> Result<String, RpcError> {
    let intent = intent("option.clawback", Some(Vec::new()), fee);
    let submitted = checked_submit(
        ctx,
        signer,
        approver,
        broadcaster,
        sage_summary,
        CHIA_CLAIM_OPTION,
        spends,
        &intent,
        false,
    )?;
    Ok(submitted.to_json())
}

// ---------------------------------------------------------------------------------------
// chia_transferOption
// ---------------------------------------------------------------------------------------

/// Send the option to `to`, key-free (Sage's `transfer_options`). Returns the spends and the
/// option coin's amount.
pub async fn build_transfer(
    wallet: &Wallet,
    launcher_id: Bytes32,
    to: Bytes32,
    fee: u64,
) -> Result<Result<(Vec<CoinSpend>, u64), Refusal>, WalletError> {
    let Some(option) = wallet.db.spendable_option(launcher_id).await? else {
        return Ok(Err(Refusal::UnknownOption));
    };
    let spends = wallet
        .transfer_options(vec![launcher_id], to, fee, None)
        .await?;
    Ok(Ok((spends, option.coin.amount)))
}

pub fn transfer_option(
    state: &AppState,
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    params_json: &str,
) -> Result<String, RpcError> {
    too_many_failed(ctx)?;
    let params: Value = serde_json::from_str(params_json).map_err(|_| invalid("json"))?;
    let launcher_id = launcher_param(&params)?;
    let fee = amount_param(&params, "fee")?.unwrap_or(0);
    let address = params
        .get("address")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("address"))?
        .to_owned();
    let to = on_sage(state, async move |sage| sage.parse_address(address))
        .map_err(|_| invalid("address"))?;
    let (spends, amount) = on_sage(state, async move |sage| {
        Ok(build_transfer(&*sage.wallet()?, launcher_id, to, fee).await?)
    })
    .map_err(|e| build_failed(CHIA_TRANSFER_OPTION, ctx, &e))?
    .map_err(Refusal::rpc)?;
    let summary = sage_view(state, &spends)?;
    submit_transfer(
        ctx,
        signer,
        approver,
        broadcaster,
        &summary,
        spends,
        Part {
            asset: AssetId::Option(launcher_id),
            amount,
        },
        to,
        fee,
    )
}

/// After the build: exactly this option to exactly this address (nothing to anyone else) and
/// this fee, the verified kind `option.transfer`.
#[allow(clippy::too_many_arguments)]
pub fn submit_transfer(
    ctx: &RequestContext<'_>,
    signer: &dyn kit::Signer,
    approver: &dyn kit::Approver,
    broadcaster: &dyn Broadcaster,
    sage_summary: &TransactionSummary,
    spends: Vec<CoinSpend>,
    option: Part,
    to: Bytes32,
    fee: u64,
) -> Result<String, RpcError> {
    let recipients = if ctx.ownership.p2_puzzle_hashes.contains(&to) {
        Vec::new()
    } else {
        vec![RecipientClaim {
            asset: option.asset,
            puzzle_hash: to,
            amount: option.amount,
        }]
    };
    let intent = intent("option.transfer", Some(recipients), fee);
    let submitted = checked_submit(
        ctx,
        signer,
        approver,
        broadcaster,
        sage_summary,
        CHIA_TRANSFER_OPTION,
        spends,
        &intent,
        false,
    )?;
    Ok(submitted.to_json())
}
