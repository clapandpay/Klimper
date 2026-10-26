// SPDX-License-Identifier: AGPL-3.0-or-later
//! The partial offers this wallet made: kept in the store, followed on the chain when listed.

use chia_partial_offer::PartialOffer;
use tracing::warn;

use crate::{
    Error,
    records::{MyPartialOffer, PartialOfferView, Status, hex0x},
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
    })?;
    Ok(id)
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
                    let changed = !followed.fills.is_empty()
                        || matches!(followed.status, Status::Filled | Status::Cancelled);
                    stored.fills.extend(followed.fills);
                    stored.latest = followed.offer.to_text()?;
                    if matches!(followed.status, Status::Filled | Status::Cancelled) {
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
        mine.push(MyPartialOffer {
            id: stored.id.clone(),
            created_at: stored.created_at,
            fills: stored.fills.clone(),
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
