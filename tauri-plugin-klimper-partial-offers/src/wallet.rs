// SPDX-License-Identifier: AGPL-3.0-or-later
//! Partial offers on a Sage wallet: following one on the chain, taking an amount of it, making
//! one, and cancelling it. Everything here goes through `sage-wallet`'s public API (coin
//! selection, offers, signing, sending), so these functions are shaped to become
//! `sage-wallet/src/wallet/offer/partial.rs` in a pull request to Sage without rewriting.

use std::sync::Arc;

use chia_partial_offer::{
    PartialAsset, PartialOffer, PartialOfferTerms, PartialSpend, make_partial_offer,
};
use chia_wallet_sdk::clvm_traits::clvm_quote;
use chia_wallet_sdk::clvmr::NodePtr;
use chia_wallet_sdk::{
    chia::{
        bls::{SecretKey, Signature},
        protocol::{Bytes32, SpendBundle},
        puzzle_types::standard::StandardArgs,
    },
    driver::{Action, Offer, Spend, SpendContext, StandardLayer},
    signer::AggSigConstants,
};
use indexmap::IndexMap;
use sage_wallet::{
    Offered, Requested, RequestedCat, SyncCommand, Transaction, Wallet, WalletPeer,
    insert_transaction,
};
use tokio::sync::mpsc;

use crate::{
    Error,
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

    fn peer(&self) -> Result<&WalletPeer, Error> {
        self.peer.as_ref().ok_or(Error::NoPeer)
    }

    async fn sign(&self, spend_bundle: SpendBundle) -> Result<SpendBundle, Error> {
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

fn fill(offer: &PartialOffer, bought: u64) -> Fill {
    Fill {
        coin_id: hex0x(offer.coin.coin_id()),
        bought: bought.to_string(),
        paid: offer.info.price_data.reverse_quote(bought).to_string(),
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
        let coin_id = offer.coin.coin_id();
        let Some(state) = peer.fetch_optional_coin(coin_id, genesis).await? else {
            // Not on the chain. Before the first fill that is expected, unless the maker has
            // spent the coins it was made from (a cancel before any fill).
            let mut status = Status::Unknown;
            if offer.is_first() {
                status = Status::Open;
                for input in offer.input_coin_ids() {
                    if let Some(input) = peer.fetch_optional_coin(input, genesis).await?
                        && input.spent_height.is_some()
                    {
                        status = Status::Cancelled;
                    }
                }
            }
            return Ok(Followed {
                status: expired(&offer, status, now),
                offer,
                fills,
            });
        };
        // On the chain: the maker's spends are done, and a taker must not repeat them.
        offer.spend_bundle = SpendBundle::new(Vec::new(), Signature::default());
        if state.spent_height.is_none() {
            let status = if offer.coin.amount < offer.info.initial_offered() {
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
            PartialSpend::Filled { bought, next } => {
                fills.push(fill(&offer, bought));
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
    match offer.info.expiration {
        Some(expiration) if status.is_live() && now >= expiration => Status::Expired,
        _ => status,
    }
}

/// Reads a `partial1…` text and follows it to its newest coin.
pub async fn view(ctx: &WalletCtx, text: &str, now: u64) -> Result<PartialOfferView, Error> {
    let offer = PartialOffer::from_text(text)?;
    let followed = follow(ctx, offer, now).await?;
    let is_mine = ctx.owns(followed.offer.info.maker_puzzle_hash).await?;
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
pub async fn take(
    ctx: &WalletCtx,
    text: &str,
    paid: u64,
    fee: u64,
    now: u64,
) -> Result<(TakeResult, SpendBundle), Error> {
    let read = PartialOffer::from_text(text)?;
    let followed = follow(ctx, read.clone(), now).await?;
    if followed.offer.coin != read.coin {
        return Err(Error::OfferChanged);
    }
    match followed.status {
        Status::Open | Status::PartlyFilled => {}
        Status::Unknown => return Err(Error::NoPeer),
        status => return Err(Error::Closed(status)),
    }
    let offer = followed.offer;
    let info = offer.info.clone();

    let paid = info.trim_payment(paid);
    let max = info.max_fill_requested(offer.coin.amount);
    if paid < info.min_fill_requested() {
        return Err(Error::Invalid(format!(
            "the least one fill pays is {}",
            info.min_fill_requested()
        )));
    }
    if paid > max {
        return Err(Error::Invalid(format!("the most one fill pays is {max}")));
    }
    let bought = info.price_data.quote(paid);
    let required_fee = info.required_fee.unwrap_or(0);

    // The taker's own offer: what this fill pays, for what it buys.
    let mut offered = Offered {
        fee,
        ..Default::default()
    };
    match info.requested_asset_info.asset_id {
        None => offered.xch = paid + required_fee,
        Some(asset_id) => {
            offered.cats = IndexMap::from([(asset_id, paid)]);
            offered.xch = required_fee;
        }
    }
    let mut requested = Requested::default();
    match info.offered_asset_info.asset_id {
        None => requested.xch = bought,
        Some(asset_id) => {
            requested.cats.insert(
                asset_id,
                RequestedCat {
                    amount: bought,
                    hidden_puzzle_hash: info.offered_asset_info.hidden_puzzle_hash,
                },
            );
        }
    }
    let unsigned = ctx.wallet.make_offer(offered, requested, None).await?;
    let signed = ctx.sign(unsigned).await?;
    let mut spend_ctx = SpendContext::new();
    let taker = Offer::from_spend_bundle(&mut spend_ctx, &signed)?;
    let next = (offer.coin.amount > bought).then(|| offer.child(offer.coin.amount - bought));
    let bundle = offer.accept_offer(&mut spend_ctx, taker)?;

    Ok((
        TakeResult {
            transaction_id: hex0x(bundle.name()),
            bought: bought.to_string(),
            paid: paid.to_string(),
            next_offer: next.map(|next| next.to_text()).transpose()?,
        },
        bundle,
    ))
}

/// The maker's terms from a request, with this wallet's receive puzzle as the maker.
pub async fn terms(ctx: &WalletCtx, req: &MakeRequest) -> Result<PartialOfferTerms, Error> {
    let asset = |id: &Option<String>| -> Result<Option<Bytes32>, Error> {
        id.as_deref()
            .filter(|id| !id.is_empty() && *id != "xch")
            .map(parse_hash)
            .transpose()
    };
    let side = async |id: Option<Bytes32>| -> Result<PartialAsset, Error> {
        Ok(match id {
            None => PartialAsset::xch(),
            Some(asset_id) => PartialAsset::cat(
                asset_id,
                ctx.wallet
                    .fetch_offer_cat_hidden_puzzle_hash(asset_id)
                    .await?,
            ),
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

    let terms = PartialOfferTerms {
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

/// Makes a partial offer: the wallet's one-sided offer of the offered asset, signed, turned
/// into the first partial coin. Nothing is sent: the first taker sends the maker's spends.
/// Returns the offer and the one-sided offer (for the wallet to lock its coins with).
pub async fn make(
    ctx: &WalletCtx,
    terms: &PartialOfferTerms,
) -> Result<(PartialOffer, SpendBundle), Error> {
    let (xch, cat) = terms.one_sided_offer();
    let offered = Offered {
        xch,
        cats: cat.into_iter().collect(),
        ..Default::default()
    };
    let unsigned = ctx
        .wallet
        .make_offer(offered, Requested::default(), None)
        .await?;
    let one_sided = ctx.sign(unsigned).await?;
    let partial = make_partial_offer(&one_sided, terms, &ctx.agg_sig)?;
    Ok((partial, one_sided))
}

/// Cancels a partial offer this wallet made: before the first fill by spending one of the
/// coins it was made from, after it by clawing the rest back. Costs `fee` as the network fee.
pub async fn cancel(ctx: &WalletCtx, text: &str, fee: u64, now: u64) -> Result<SpendBundle, Error> {
    let followed = follow(ctx, PartialOffer::from_text(text)?, now).await?;
    match followed.status {
        Status::Open | Status::PartlyFilled | Status::Expired => {}
        Status::Unknown => return Err(Error::NoPeer),
        status => return Err(Error::Closed(status)),
    }
    let offer = followed.offer;
    let maker = offer.info.maker_puzzle_hash;
    let Some(public_key) = ctx.wallet.db.public_key(maker).await? else {
        return Err(Error::NotMine);
    };

    let mut spend_ctx = SpendContext::new();
    if offer.is_first() {
        // The partial coin does not exist yet: spending any coin it is made from cancels it.
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
        let conditions = offer.claw_back_conditions(&mut spend_ctx)?;
        let delegated = Spend::new(spend_ctx.alloc(&clvm_quote!(conditions))?, NodePtr::NIL);
        let inner =
            StandardLayer::new(public_key).delegated_inner_spend(&mut spend_ctx, delegated)?;
        offer.claw_back(&mut spend_ctx, inner)?;
    }
    ctx.sign(SpendBundle::new(spend_ctx.take(), Signature::default()))
        .await
}
