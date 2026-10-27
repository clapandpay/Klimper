// SPDX-License-Identifier: AGPL-3.0-or-later
//! The partial offers this wallet made: kept in the store, followed on the chain when listed.

use chia_partial_offer::PartialOffer;
use tracing::warn;

use crate::{
    Error,
    records::{MyPartialOffer, PartialOfferView, PendingCancel, Status, hex0x},
    store::{Store, StoredOffer},
    wallet::{WalletCtx, follow},
};

/// Records a partial offer the wallet just made.
pub fn remember(store: &Store, offer: &PartialOffer, now: u64) -> Result<String, Error> {
    let id = hex0x(offer.coin.coin_id());
    store.put(StoredOffer {
        id: id.clone(),
        created_at: now,
        latest: offer.to_text()?,
        fills: Vec::new(),
        ended: None,
        cancel: None,
    })?;
    Ok(id)
}

/// Notes that a cancel of offer `id` was sent, so the list says "cancelling" until the chain
/// shows it cancelled.
pub fn cancel_sent(
    store: &Store,
    id: &str,
    transaction_id: String,
    coin_id: String,
    now: u64,
) -> Result<(), Error> {
    let mut stored = latest(store, id)?;
    stored.cancel = Some(PendingCancel {
        transaction_id,
        coin_id,
        sent_at: now,
    });
    store.update(stored)?;
    Ok(())
}

/// A view of one of the wallet's own offers says "cancelling" while a cancel it sent is
/// unconfirmed, as the list does. `text` is the text that was viewed.
pub fn with_pending_cancel(
    store: &Store,
    text: &str,
    mut view: PartialOfferView,
) -> Result<PartialOfferView, Error> {
    if view.is_mine {
        let text = text.trim();
        let pending = store.list()?.into_iter().find(|stored| {
            stored.cancel.is_some() && (stored.latest == text || stored.latest == view.offer)
        });
        if let Some(stored) = pending {
            view.status = shown(view.status, stored.cancel.as_ref());
        }
    }
    Ok(view)
}

/// What the list shows: a cancel that was sent reads as "cancelling" while the offer is still
/// live on the chain.
fn shown(status: Status, cancel: Option<&PendingCancel>) -> Status {
    match (status, cancel) {
        (Status::Open | Status::PartlyFilled | Status::Expired | Status::Unknown, Some(_)) => {
            Status::Cancelling
        }
        (status, _) => status,
    }
}

/// Every partial offer the wallet made, newest first, each still live followed to its newest
/// coin first. One offer that cannot be read or followed does not hide the others: it is
/// skipped (unreadable) or shown as it was last seen, with the status unknown.
pub async fn list(ctx: &WalletCtx, store: &Store, now: u64) -> Result<Vec<MyPartialOffer>, Error> {
    let mut mine = Vec::new();
    for mut stored in store.list()? {
        let offer = match PartialOffer::from_text(&stored.latest) {
            Ok(offer) => offer,
            Err(error) => {
                warn!("partial offer {} cannot be read: {error}", stored.id);
                continue;
            }
        };
        let (offer, status) = match stored.ended {
            Some(status) => (offer, status),
            None => match follow(ctx, offer.clone(), now).await {
                Ok(followed) => {
                    let ended = matches!(followed.status, Status::Filled | Status::Cancelled);
                    let was_pending = stored.cancel.is_some();
                    let changed = !followed.fills.is_empty() || ended;
                    // Past the coin the cancel spends, a fill confirmed first and the cancel
                    // failed: the offer is live again.
                    let coin_id = hex0x(followed.offer.coin.coin_id());
                    if ended
                        || stored
                            .cancel
                            .as_ref()
                            .is_some_and(|cancel| cancel.coin_id != coin_id)
                    {
                        stored.cancel = None;
                    }
                    let changed = changed || stored.cancel.is_some() != was_pending;
                    stored.fills.extend(followed.fills);
                    stored.latest = followed.offer.to_text()?;
                    if ended {
                        stored.ended = Some(followed.status);
                    }
                    if changed {
                        store.update(stored.clone())?;
                    }
                    (followed.offer, followed.status)
                }
                Err(error) => {
                    warn!("partial offer {} cannot be followed: {error}", stored.id);
                    (offer, Status::Unknown)
                }
            },
        };
        let status = shown(status, stored.cancel.as_ref());
        mine.push(MyPartialOffer {
            id: stored.id.clone(),
            created_at: stored.created_at,
            fills: stored.fills.clone(),
            cancel: stored.cancel.clone(),
            view: PartialOfferView::new(&offer, status, Vec::new(), true, &ctx.prefix)?,
        });
    }
    mine.sort_by_key(|offer| std::cmp::Reverse(offer.created_at));
    Ok(mine)
}

/// The newest text of one of the wallet's offers.
pub fn latest(store: &Store, id: &str) -> Result<StoredOffer, Error> {
    store
        .get(id)?
        .ok_or_else(|| Error::Invalid(format!("no partial offer {id}")))
}
