// SPDX-License-Identifier: AGPL-3.0-or-later
//! Partial offers (CHIP-0052) for Klimper (TASK-164): view one, take an amount of it, make one,
//! list and cancel the wallet's own.
//!
//! One of three replaceable parts (docs/partial-offers.md): the driver is
//! `crates/chia-partial-offer`, this plugin puts it on Sage's wallet, and the screens in
//! `src/klimper/partial/` reach it only through `api.ts`. Sage's own crates are not changed:
//! the plugin takes Sage's state as the Xchonnect plugin does and uses its public API. When
//! Sage ships partial offers itself, this plugin goes and `api.ts` points at Sage's commands.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use chia_partial_offer::PartialOfferError;
use chia_wallet_sdk::{driver::encode_offer, signer::AggSigConstants};
use sage::Sage;
use sage_api::ImportOffer;
use sage_wallet::WalletError;
use serde::{Serialize, Serializer, ser::SerializeStruct};
use tauri::{
    AppHandle, Manager, Runtime, command,
    plugin::{Builder, TauriPlugin},
};
use tokio::sync::Mutex;

pub mod mine;
pub mod records;
pub mod store;
pub mod wallet;

#[cfg(test)]
mod sage_sim;
#[cfg(test)]
mod tests;

use records::{
    CancelResult, MakeRequest, MakeResult, MyPartialOffer, PartialOfferView, Status, TakeResult,
    hex0x, parse_amount,
};
use store::Store;
use wallet::WalletCtx;

/// Sage's state, as `src-tauri` manages it.
pub type AppState = Arc<Mutex<Sage>>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Partial(#[from] PartialOfferError),
    #[error("{0}")]
    Wallet(#[from] WalletError),
    #[error("{0}")]
    Driver(#[from] chia_wallet_sdk::driver::DriverError),
    #[error("{0}")]
    Invalid(String),
    #[error("someone took from this offer first: look at it again")]
    OfferChanged,
    #[error("this offer is {0:?}")]
    Closed(Status),
    #[error("no wallet is open")]
    NoWallet,
    #[error("no connection to the network")]
    NoPeer,
    #[error("this wallet cannot sign")]
    NoSigningKey,
    #[error("this offer was not made by this wallet")]
    NotMine,
    #[error("partial offers file: {0}")]
    Store(String),
    #[error("{0}")]
    Sage(String),
}

impl Error {
    /// A fixed word the screens can act on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Partial(PartialOfferError::NotPartial(_)) => "not_partial",
            Self::Partial(_) | Self::Driver(_) | Self::Invalid(_) => "invalid",
            Self::Wallet(_) | Self::Sage(_) | Self::Store(_) => "wallet",
            Self::OfferChanged => "offer_changed",
            Self::Closed(_) => "closed",
            Self::NoWallet => "no_wallet",
            Self::NoPeer => "no_peer",
            Self::NoSigningKey => "no_signing_key",
            Self::NotMine => "not_mine",
        }
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut error = serializer.serialize_struct("Error", 2)?;
        error.serialize_field("code", self.code())?;
        error.serialize_field("message", &self.to_string())?;
        error.end()
    }
}

impl From<sage_database::DatabaseError> for Error {
    fn from(error: sage_database::DatabaseError) -> Self {
        Self::Wallet(error.into())
    }
}

impl From<sage::Error> for Error {
    fn from(error: sage::Error) -> Self {
        Self::Sage(error.to_string())
    }
}

type Result<T> = std::result::Result<T, Error>;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// The logged-in wallet, its peer and (for what signs) its key, taken from Sage's state; the
/// lock is let go before any chain work.
async fn wallet_ctx<R: Runtime>(app: &AppHandle<R>, signing: bool) -> Result<(WalletCtx, Store)> {
    let state = app.state::<AppState>().inner().clone();
    let sage = state.lock().await;
    let wallet = sage.wallet().map_err(|_| Error::NoWallet)?;
    let master_sk = if signing {
        let (_mnemonic, master_sk) = sage
            .keychain
            .extract_secrets(wallet.fingerprint, b"")
            .map_err(|error| Error::Sage(error.to_string()))?;
        Some(master_sk.ok_or(Error::NoSigningKey)?)
    } else {
        None
    };
    let peer = sage.peer_state.lock().await.acquire_peer();
    let store = Store::new(
        sage.wallet_db_path(wallet.fingerprint)?
            .with_file_name(format!("{}-klimper-partial-offers.json", sage.network_id())),
    );
    let ctx = WalletCtx {
        wallet,
        agg_sig: AggSigConstants::new(sage.network().agg_sig_me()),
        master_sk,
        peer,
        sync: Some(sage.command_sender.clone()),
        prefix: sage.network().prefix(),
    };
    Ok((ctx, store))
}

/// Reads a `partial1…` text and follows it on the chain to its newest coin.
#[command]
async fn view_partial_offer<R: Runtime>(
    app: AppHandle<R>,
    offer: String,
) -> Result<PartialOfferView> {
    let (ctx, _) = wallet_ctx(&app, false).await?;
    wallet::view(&ctx, &offer, now()).await
}

/// Takes `amount` mojos of the requested asset's worth of the offer, with `fee` mojos as the
/// network fee (the offer's required fee comes on top), and sends it.
#[command]
async fn take_partial_offer<R: Runtime>(
    app: AppHandle<R>,
    offer: String,
    amount: String,
    fee: String,
) -> Result<TakeResult> {
    let (ctx, _) = wallet_ctx(&app, true).await?;
    let (result, bundle) = wallet::take(
        &ctx,
        &offer,
        parse_amount(&amount)?,
        parse_amount(&fee)?,
        now(),
    )
    .await?;
    ctx.submit(bundle).await?;
    Ok(result)
}

/// Makes a partial offer. Nothing is sent: the coins it is made from are locked in the wallet
/// (as an offer of its own, which Sage lists) until the first fill or a cancel spends them.
#[command]
async fn make_partial_offer<R: Runtime>(
    app: AppHandle<R>,
    request: MakeRequest,
) -> Result<MakeResult> {
    let (ctx, store) = wallet_ctx(&app, true).await?;
    let terms = wallet::terms(&ctx, &request).await?;
    let (offer, one_sided) = wallet::make(&ctx, &terms).await?;
    {
        let state = app.state::<AppState>().inner().clone();
        let sage = state.lock().await;
        sage.import_offer(ImportOffer {
            offer: encode_offer(&one_sided)?,
        })
        .await?;
    }
    let id = mine::remember(&store, &offer, now())?;
    Ok(MakeResult {
        id,
        offer: offer.to_text()?,
    })
}

/// Cancels one of the wallet's partial offers, with `fee` mojos as the network fee.
#[command]
async fn cancel_partial_offer<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    fee: String,
) -> Result<CancelResult> {
    let (ctx, store) = wallet_ctx(&app, true).await?;
    let stored = mine::latest(&store, &id)?;
    let bundle = wallet::cancel(&ctx, &stored.latest, parse_amount(&fee)?, now()).await?;
    let name = ctx.submit(bundle).await?;
    Ok(CancelResult {
        transaction_id: hex0x(name),
    })
}

/// The wallet's partial offers, newest first, each still live followed on the chain.
#[command]
async fn get_partial_offers<R: Runtime>(app: AppHandle<R>) -> Result<Vec<MyPartialOffer>> {
    let (ctx, store) = wallet_ctx(&app, false).await?;
    mine::list(&ctx, &store, now()).await
}

/// Forgets one of the wallet's partial offers (it stays on the chain as it is).
#[command]
async fn delete_partial_offer<R: Runtime>(app: AppHandle<R>, id: String) -> Result<bool> {
    let (_, store) = wallet_ctx(&app, false).await?;
    store.delete(&id)
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("klimper-partial-offers")
        .invoke_handler(tauri::generate_handler![
            view_partial_offer,
            take_partial_offer,
            make_partial_offer,
            cancel_partial_offer,
            get_partial_offers,
            delete_partial_offer,
        ])
        .build()
}
