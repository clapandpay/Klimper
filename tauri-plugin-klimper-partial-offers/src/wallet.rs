// SPDX-License-Identifier: AGPL-3.0-or-later
//! Partial offers on a Sage wallet: following one on the chain, taking an amount of it, making
//! one, and cancelling it. The puzzles are chia-wallet-sdk's (through `crate::partial`);
//! everything else goes through `sage-wallet`'s public API (coin selection, spends, signing,
//! sending), so these functions are shaped to become `sage-wallet/src/wallet/offer/partial.rs`
//! in a pull request to Sage without rewriting.

use std::sync::Arc;

use chia_sdk_next::PartialOfferAsset;
use chia_wallet_sdk::{
    chia::{
        bls::{SecretKey, Signature},
        protocol::{Bytes32, SpendBundle},
        puzzle_types::standard::StandardArgs,
    },
    driver::{Action, Id, SpendContext, SpendWithConditions, Spends, StandardLayer},
    signer::AggSigConstants,
    types::Conditions,
};
use sage_wallet::{SyncCommand, Transaction, Wallet, WalletPeer, insert_transaction};
use tokio::sync::mpsc;

use crate::{
    Error, PartialOffer, PartialSpend, Terms,
    records::{
        Fill, MakeRequest, PartialOfferView, Status, TakeResult, hex0x, parse_amount, parse_hash,
    },
};

/// What the partial offer functions need of a logged-in wallet.
#[derive(Debug, Clone)]
pub struct WalletCtx {
    pub wallet: Arc<Wallet>,
    pub agg_sig: AggSigConstants,
    /// Only for what signs (take, make, cancel).
    pub master_sk: Option<SecretKey>,
    /// Only for what reads or sends to the chain.
    pub peer: Option<WalletPeer>,
    /// Sage's sync, told about the coins a transaction creates.
    pub sync: Option<mpsc::Sender<SyncCommand>>,
    /// The address prefix (`xch`, `txch`).
    pub prefix: String,
}

impl WalletCtx {
    fn master_sk(&self) -> Result<SecretKey, Error> {
        self.master_sk.clone().ok_or(Error::NoSigningKey)
    }

    pub fn peer(&self) -> Result<&WalletPeer, Error> {
        self.peer.as_ref().ok_or(Error::NoPeer)
    }

    pub async fn sign(&self, spend_bundle: SpendBundle) -> Result<SpendBundle, Error> {
        Ok(self
            .wallet
            .sign_transaction(spend_bundle, &self.agg_sig, self.master_sk()?, false)
            .await?)
    }

    /// Sends a signed transaction, as Sage's own `submit` does: recorded as pending in the
    /// wallet, and the coins it creates watched.
    pub async fn submit(&self, spend_bundle: SpendBundle) -> Result<Bytes32, Error> {
        let name = spend_bundle.name();
        let subscriptions = insert_transaction(
            &self.wallet.db,
            self.peer()?,
            self.wallet.genesis_challenge,
            name,
            Transaction::from_coin_spends(spend_bundle.coin_spends)?,
            spend_bundle.aggregated_signature,
        )
        .await?;
        if let Some(sync) = &self.sync {
            let _ = sync
                .send(SyncCommand::SubscribeCoins {
                    coin_ids: subscriptions,
                })
                .await;
        }
        Ok(name)
    }

    /// Whether this wallet holds the key of `puzzle_hash` (a standard puzzle of its own).
    pub async fn owns(&self, puzzle_hash: Bytes32) -> Result<bool, Error> {
        Ok(self.wallet.db.public_key(puzzle_hash).await?.is_some())
    }
}

/// A partial offer followed to its newest coin.
#[derive(Debug, Clone)]
pub struct Followed {
    pub offer: PartialOffer,
    pub status: Status,
    pub fills: Vec<Fill>,
}

fn fill(offer: &PartialOffer, paid: u64, bought: u64) -> Fill {
    Fill {
        coin_id: hex0x(offer.coin().coin_id()),
        bought: bought.to_string(),
        paid: paid.to_string(),
    }
}

/// Follows `offer` along the chain: through every fill to the newest coin, or to the fill that
/// took everything, or to the maker's claw back. Without a peer the offer comes back as it
/// is, with the status unknown.
pub async fn follow(ctx: &WalletCtx, mut offer: PartialOffer, now: u64) -> Result<Followed, Error> {
    let Some(peer) = &ctx.peer else {
        return Ok(Followed {
            offer,
            status: Status::Unknown,
            fills: Vec::new(),
        });
    };
    let genesis = ctx.wallet.genesis_challenge;
    let mut fills = Vec::new();
    loop {
        let coin_id = offer.coin().coin_id();
        let Some(state) = peer.fetch_optional_coin(coin_id, genesis).await? else {
            // Not on the chain. Before the first fill that is expected, unless the maker has
            // spent a coin it was made from (a cancel before any fill).
            let mut status = Status::Unknown;
            if offer.is_first() {
                status = Status::Open;
                let mut spent = false;
                for input in offer.input_coin_ids() {
                    if let Some(input) = peer.fetch_optional_coin(input, genesis).await?
                        && input.spent_height.is_some()
                    {
                        spent = true;
                    }
                }
                if spent {
                    // The first fill may have confirmed between the two lookups: then the
                    // partial coin exists now, and the offer goes on from there.
                    if peer.fetch_optional_coin(coin_id, genesis).await?.is_some() {
                        continue;
                    }
                    status = Status::Cancelled;
                }
            }
            return Ok(Followed {
                status: expired(&offer, status, now),
                offer,
                fills,
            });
        };
        // On the chain: the maker's spends are done, and a taker must not repeat them.
        offer = offer.on_chain();
        if state.spent_height.is_none() {
            let status = if offer.coin().amount < offer.initial_amounts().0 {
                Status::PartlyFilled
            } else {
                Status::Open
            };
            return Ok(Followed {
                status: expired(&offer, status, now),
                offer,
                fills,
            });
        }
        let spend = peer.fetch_coin_spend(coin_id, genesis).await?;
        match offer.follow(&spend)? {
            PartialSpend::Filled { paid, bought, next } => {
                fills.push(fill(&offer, paid, bought));
                match next {
                    Some(next) => offer = *next,
                    None => {
                        return Ok(Followed {
                            offer,
                            status: Status::Filled,
                            fills,
                        });
                    }
                }
            }
            PartialSpend::ClawedBack => {
                return Ok(Followed {
                    offer,
                    status: Status::Cancelled,
                    fills,
                });
            }
        }
    }
}

fn expired(offer: &PartialOffer, status: Status, now: u64) -> Status {
    match offer.info().expiration {
        Some(expiration) if status.is_live() && now >= expiration => Status::Expired,
        _ => status,
    }
}

/// Reads a `partial1…` text and follows it to its newest coin.
pub async fn view(ctx: &WalletCtx, text: &str, now: u64) -> Result<PartialOfferView, Error> {
    let offer = PartialOffer::from_text(text)?;
    let followed = follow(ctx, offer, now).await?;
    let is_mine = ctx.owns(followed.offer.info().maker_puzzle_hash).await?;
    PartialOfferView::new(
        &followed.offer,
        followed.status,
        followed.fills,
        is_mine,
        &ctx.prefix,
    )
}

/// Takes `paid` of the requested asset's worth of `text`, plus the required fee and `fee` as
/// the network fee. The offer is checked against the chain right before signing: if someone
/// filled it first, the answer is [`Error::OfferChanged`] and nothing is signed.
///
/// Built as Sage's `take_offer` builds a take: the fill (the SDK's, an offer of what the coin
/// releases for the payment to the maker) is taken with the wallet's coins, never the coins the
/// fill itself spends, and every spend of the wallet asserts the others. The fill's required fee
/// is left to the partial coin to reserve (the SDK's unreserved fee), so it is not counted
/// twice; `fee` is reserved by the wallet as usual.
pub async fn take(
    ctx: &WalletCtx,
    text: &str,
    paid: u64,
    fee: u64,
    now: u64,
) -> Result<(TakeResult, SpendBundle), Error> {
    let read = PartialOffer::from_text(text)?;
    let followed = follow(ctx, read.clone(), now).await?;
    if followed.offer.coin() != read.coin() {
        return Err(Error::OfferChanged);
    }
    match followed.status {
        Status::Open | Status::PartlyFilled => {}
        Status::Unknown => return Err(Error::NoPeer),
        status => return Err(Error::Closed(status)),
    }
    let offer = followed.offer;

    let paid = offer.trim_payment(paid);
    let max = offer.max_fill_requested();
    if paid < offer.min_fill_requested() {
        return Err(Error::Invalid(format!(
            "the least one fill pays is {}",
            offer.min_fill_requested()
        )));
    }
    if paid > max {
        return Err(Error::Invalid(format!("the most one fill pays is {max}")));
    }

    let mut spend_ctx = SpendContext::new();
    let filled = offer.bundle().fill(&mut spend_ctx, paid)?;
    let mut actions = filled.actions();
    if fee > 0 {
        actions.push(Action::fee(fee));
    }
    let mut spends = Spends::new(ctx.wallet.change_p2_puzzle_hash().await?);
    spends.add(filled.offer.offered_coins().clone());
    let excluded: Vec<Bytes32> = filled
        .offer
        .spend_bundle()
        .coin_spends
        .iter()
        .map(|spend| spend.coin.coin_id())
        .collect();
    ctx.wallet
        .select_spends_excluding(&mut spend_ctx, &mut spends, &actions, &excluded)
        .await?;
    let deltas = spends.apply(&mut spend_ctx, &actions)?;
    ctx.wallet
        .complete_spends(&mut spend_ctx, &deltas, spends)
        .await?;
    let signed = ctx
        .sign(SpendBundle::new(spend_ctx.take(), Signature::default()))
        .await?;
    let next = filled
        .child
        .map(|child| PartialOffer::from_coin(child)?.to_text())
        .transpose()?;
    let bundle = filled.offer.take(signed);

    Ok((
        TakeResult {
            transaction_id: hex0x(bundle.name()),
            bought: filled.offered_amount.to_string(),
            paid: filled.requested_amount.to_string(),
            next_offer: next,
        },
        bundle,
    ))
}

/// The maker's terms from a request, with this wallet's receive puzzle as the maker.
pub async fn terms(ctx: &WalletCtx, req: &MakeRequest) -> Result<Terms, Error> {
    let asset = |id: &Option<String>| -> Result<Option<Bytes32>, Error> {
        match id.as_deref() {
            None | Some("xch") => Ok(None),
            Some("") => Err(Error::Invalid("pick the token first".to_string())),
            Some(id) => parse_hash(id).map(Some),
        }
    };
    let side = async |id: Option<Bytes32>| -> Result<PartialOfferAsset, Error> {
        Ok(match id {
            None => PartialOfferAsset::Xch,
            Some(asset_id) => PartialOfferAsset::Cat {
                asset_id,
                hidden_puzzle_hash: ctx
                    .wallet
                    .fetch_offer_cat_hidden_puzzle_hash(asset_id)
                    .await?,
            },
        })
    };
    let offered = side(asset(&req.offered_asset_id)?).await?;
    let requested = side(asset(&req.requested_asset_id)?).await?;

    // The maker puzzle must be a standard one of this wallet: its key opens the claw back.
    let maker_puzzle_hash = ctx.wallet.change_p2_puzzle_hash().await?;
    let Some(public_key) = ctx.wallet.db.public_key(maker_puzzle_hash).await? else {
        return Err(Error::Invalid(
            "the wallet has no key for its own address".to_string(),
        ));
    };
    if Bytes32::from(StandardArgs::curry_tree_hash(public_key)) != maker_puzzle_hash {
        return Err(Error::Invalid(
            "the wallet's address is not a standard puzzle".to_string(),
        ));
    }

    let terms = Terms {
        offered,
        offered_amount: parse_amount(&req.offered_amount)?,
        requested,
        requested_amount: parse_amount(&req.requested_amount)?,
        min_fill_requested: parse_amount(&req.min_fill_requested)?,
        expiration: req.expires_at,
        required_fee: req
            .required_fee
            .as_deref()
            .map(parse_amount)
            .transpose()?
            .filter(|fee| *fee > 0),
        maker_puzzle_hash,
    };
    terms.check()?;
    Ok(terms)
}

/// The maker's spends for `terms`, unsigned: the offered amount sent to the partial puzzle with
/// the creation memos CHIP-0052 asks for (so the coin can be found on the chain), from the
/// wallet's own coins, change back to the wallet, and `fee` as the network fee (paid when the
/// first taker sends these spends). What chia-wallet-sdk's maker does.
pub async fn maker_spends(ctx: &WalletCtx, terms: &Terms, fee: u64) -> Result<SpendBundle, Error> {
    let info = terms.info()?;
    let mut spend_ctx = SpendContext::new();
    let memos = info.creation_memos(&mut spend_ctx)?;
    let id = match terms.offered {
        PartialOfferAsset::Xch => Id::Xch,
        PartialOfferAsset::Cat { asset_id, .. } => Id::Existing(asset_id),
    };
    let mut actions = vec![Action::send(
        id,
        info.p2_puzzle_hash(),
        terms.offered_amount,
        memos,
    )];
    if fee > 0 {
        actions.push(Action::fee(fee));
    }
    ctx.wallet
        .spend(&mut spend_ctx, Vec::new(), &actions)
        .await?;
    Ok(SpendBundle::new(spend_ctx.take(), Signature::default()))
}

/// Makes a partial offer: the maker's spends, signed, and the offer they make. Nothing is
/// sent: the first taker sends the maker's spends with the first fill.
pub async fn make(ctx: &WalletCtx, terms: &Terms) -> Result<PartialOffer, Error> {
    let signed = ctx.sign(maker_spends(ctx, terms, 0).await?).await?;
    Ok(PartialOffer::from_maker_spend_bundle(
        signed,
        terms.info()?,
    )?)
}

/// Cancels a partial offer this wallet made: before the first fill by spending one of the
/// wallet's own coins it was made from, after it by clawing the rest back. Costs `fee` as the
/// network fee. Answers the signed cancel and the partial coin it spends (`0x` hex).
pub async fn cancel(
    ctx: &WalletCtx,
    text: &str,
    fee: u64,
    now: u64,
) -> Result<(SpendBundle, String), Error> {
    let followed = follow(ctx, PartialOffer::from_text(text)?, now).await?;
    match followed.status {
        Status::Open | Status::PartlyFilled | Status::Expired => {}
        Status::Unknown => return Err(Error::NoPeer),
        status => return Err(Error::Closed(status)),
    }
    let offer = followed.offer;
    let coin_id = hex0x(offer.coin().coin_id());
    let maker = offer.info().maker_puzzle_hash;
    let Some(public_key) = ctx.wallet.db.public_key(maker).await? else {
        return Err(Error::NotMine);
    };

    let mut spend_ctx = SpendContext::new();
    if offer.is_first() {
        // The partial coin does not exist yet: spending any coin it is made from cancels it.
        // Only one the wallet holds on the chain will do (an offer made by Klimper 1.1 or 1.2
        // also spends settlement and security coins that never reach the chain).
        let mut own = None;
        for coin_id in offer.input_coin_ids() {
            if ctx.wallet.db.coin_kind(coin_id).await?.is_some() {
                own = Some(coin_id);
                break;
            }
        }
        let coin_id = own.ok_or(Error::NotMine)?;
        ctx.wallet
            .spend(&mut spend_ctx, vec![coin_id], &[Action::fee(fee)])
            .await?;
    } else {
        if fee > 0 {
            ctx.wallet
                .spend(&mut spend_ctx, Vec::new(), &[Action::fee(fee)])
                .await?;
        }
        let hint = spend_ctx.hint(maker)?;
        let inner = StandardLayer::new(public_key).spend_with_conditions(
            &mut spend_ctx,
            Conditions::new().create_coin(maker, offer.coin().amount, hint),
        )?;
        offer.offer().claw_back(&mut spend_ctx, inner)?;
    }
    let bundle = ctx
        .sign(SpendBundle::new(spend_ctx.take(), Signature::default()))
        .await?;
    Ok((bundle, coin_id))
}
