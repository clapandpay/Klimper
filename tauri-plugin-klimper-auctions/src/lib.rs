// SPDX-License-Identifier: AGPL-3.0-or-later
//! NFT auctions for Klimper (the owner's "dutch offers", 2026-11-07): view one from its id,
//! start one for an NFT of the wallet, bid in XCH, end it, cancel the wallet's own before a
//! bid, list the ones the wallet started or bid on.
//!
//! The driver is chia-wallet-sdk's (`crates/chia-sdk-next` until Sage's SDK release has it);
//! this plugin puts it on Sage's wallet, with the checks a wallet must add (`wallet.rs`,
//! docs/auctions.md). Sage's crates are not changed. The screens reach it through
//! `src/klimper/auctions/api.ts` only.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use sage::Sage;
use sage_wallet::WalletError;
use serde::{Serialize, Serializer, ser::SerializeStruct};
use tauri::{
    AppHandle, Manager, Runtime, command,
    plugin::{Builder, TauriPlugin},
};
use tauri_plugin_klimper_partial_offers::wallet::WalletCtx;
use tokio::sync::Mutex;

pub mod records;
pub mod store;
pub mod wallet;

#[cfg(test)]
mod sage_sim;
#[cfg(test)]
mod tests;

use records::{AuctionView, MyAuction, Sent, StartRequest};
use store::{Store, StoredAuction};

/// Sage's state, as `src-tauri` manages it.
pub type AppState = Arc<Mutex<Sage>>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Driver(#[from] chia_sdk_next::DriverError),
    #[error("{0}")]
    Wallet(#[from] WalletError),
    #[error("{0}")]
    Context(#[from] tauri_plugin_klimper_partial_offers::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("not an auction Klimper can read: {0}")]
    NotAuction(String),
    #[error("{0}")]
    Closed(String),
    #[error("no wallet is open")]
    NoWallet,
    #[error("no connection to the network")]
    NoPeer,
    #[error("this auction was not started by this wallet")]
    NotMine,
    #[error("auctions file: {0}")]
    Store(String),
    #[error("{0}")]
    Sage(String),
}

impl Error {
    /// A fixed word the screens can act on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Driver(_) | Self::Invalid(_) => "invalid",
            Self::NotAuction(_) => "not_auction",
            Self::Closed(_) => "closed",
            Self::Wallet(_) | Self::Sage(_) | Self::Store(_) | Self::Context(_) => "wallet",
            Self::NoWallet => "no_wallet",
            Self::NoPeer => "no_peer",
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

impl From<chia_wallet_sdk::driver::DriverError> for Error {
    fn from(error: chia_wallet_sdk::driver::DriverError) -> Self {
        Self::Driver(error.into())
    }
}

impl From<chia_wallet_sdk::clvm_traits::ToClvmError> for Error {
    fn from(error: chia_wallet_sdk::clvm_traits::ToClvmError) -> Self {
        Self::Driver(error.into())
    }
}

impl From<chia_wallet_sdk::clvm_traits::FromClvmError> for Error {
    fn from(error: chia_wallet_sdk::clvm_traits::FromClvmError) -> Self {
        Self::Driver(error.into())
    }
}

impl From<chia_wallet_sdk::clvmr::error::EvalErr> for Error {
    fn from(error: chia_wallet_sdk::clvmr::error::EvalErr) -> Self {
        Self::Driver(error.into())
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

pub type Result<T> = std::result::Result<T, Error>;

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// The wallet context (shared with partial offers) and this wallet's auctions file.
pub async fn sage_ctx(state: &AppState, signing: bool) -> Result<(WalletCtx, Store)> {
    let (ctx, _) = tauri_plugin_klimper_partial_offers::sage_ctx(state, signing)
        .await
        .map_err(|error| match error {
            tauri_plugin_klimper_partial_offers::Error::NoWallet => Error::NoWallet,
            error => Error::Context(error),
        })?;
    let sage = state.lock().await;
    let store = Store::new(
        sage.wallet_db_path(ctx.wallet.fingerprint)?
            .with_file_name(format!("{}-klimper-auctions.json", sage.network_id())),
    );
    Ok((ctx, store))
}

fn hex0x(bytes: chia_wallet_sdk::chia::protocol::Bytes32) -> String {
    format!("0x{}", hex::encode(bytes))
}

/// What each command does, on Sage's state.
pub mod ops {
    use super::{
        AppState, AuctionView, MyAuction, Result, Sent, StartRequest, StoredAuction, hex0x, now,
        sage_ctx, wallet,
    };

    pub async fn view(state: &AppState, id: &str) -> Result<AuctionView> {
        let (ctx, _) = sage_ctx(state, false).await?;
        let followed = wallet::follow(&ctx, wallet::parse_hash(id)?).await?;
        wallet::view(&ctx, &followed, now()).await
    }

    pub async fn start(state: &AppState, request: &StartRequest) -> Result<Sent> {
        let (ctx, store) = sage_ctx(state, true).await?;
        let (id, bundle) = wallet::start(&ctx, request, now()).await?;
        let transaction_id = hex0x(ctx.submit(bundle).await?);
        store.put(StoredAuction {
            id: hex0x(id),
            role: "started".to_string(),
            created_at: now(),
        })?;
        Ok(Sent {
            id: hex0x(id),
            transaction_id,
        })
    }

    pub async fn bid(state: &AppState, id: &str, amount: &str, fee: &str) -> Result<Sent> {
        let (ctx, store) = sage_ctx(state, true).await?;
        let launcher_id = wallet::parse_hash(id)?;
        let bundle = wallet::bid(
            &ctx,
            launcher_id,
            wallet::parse_amount(amount)?,
            wallet::parse_amount(fee)?,
            now(),
        )
        .await?;
        let transaction_id = hex0x(ctx.submit(bundle).await?);
        store.put(StoredAuction {
            id: hex0x(launcher_id),
            role: "bid".to_string(),
            created_at: now(),
        })?;
        Ok(Sent {
            id: hex0x(launcher_id),
            transaction_id,
        })
    }

    pub async fn end(state: &AppState, id: &str, fee: &str) -> Result<Sent> {
        let (ctx, _) = sage_ctx(state, true).await?;
        let launcher_id = wallet::parse_hash(id)?;
        let bundle = wallet::end(&ctx, launcher_id, wallet::parse_amount(fee)?, now()).await?;
        Ok(Sent {
            id: hex0x(launcher_id),
            transaction_id: hex0x(ctx.submit(bundle).await?),
        })
    }

    pub async fn cancel(state: &AppState, id: &str, fee: &str) -> Result<Sent> {
        let (ctx, _) = sage_ctx(state, true).await?;
        let launcher_id = wallet::parse_hash(id)?;
        let bundle = wallet::cancel(&ctx, launcher_id, wallet::parse_amount(fee)?).await?;
        Ok(Sent {
            id: hex0x(launcher_id),
            transaction_id: hex0x(ctx.submit(bundle).await?),
        })
    }

    /// The auctions the wallet started or bid on, newest first; one that cannot be read shows
    /// its error and does not hide the others.
    pub async fn list(state: &AppState) -> Result<Vec<MyAuction>> {
        let (ctx, store) = sage_ctx(state, false).await?;
        let mut mine = Vec::new();
        for stored in store.list()? {
            let read = async {
                let followed = wallet::follow(&ctx, wallet::parse_hash(&stored.id)?).await?;
                wallet::view(&ctx, &followed, now()).await
            };
            let (view, error) = match read.await {
                Ok(view) => (Some(view), None),
                Err(error) => (None, Some(error.to_string())),
            };
            mine.push(MyAuction {
                id: stored.id,
                role: stored.role,
                created_at: stored.created_at,
                view,
                error,
            });
        }
        Ok(mine)
    }

    pub async fn forget(state: &AppState, id: &str) -> Result<bool> {
        let (_, store) = sage_ctx(state, false).await?;
        store.delete(id)
    }
}

fn state<R: Runtime>(app: &AppHandle<R>) -> AppState {
    app.state::<AppState>().inner().clone()
}

#[command]
async fn view_auction<R: Runtime>(app: AppHandle<R>, id: String) -> Result<AuctionView> {
    ops::view(&state(&app), &id).await
}

#[command]
async fn start_auction<R: Runtime>(app: AppHandle<R>, request: StartRequest) -> Result<Sent> {
    ops::start(&state(&app), &request).await
}

#[command]
async fn bid_auction<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    amount: String,
    fee: String,
) -> Result<Sent> {
    ops::bid(&state(&app), &id, &amount, &fee).await
}

#[command]
async fn end_auction<R: Runtime>(app: AppHandle<R>, id: String, fee: String) -> Result<Sent> {
    ops::end(&state(&app), &id, &fee).await
}

#[command]
async fn cancel_auction<R: Runtime>(app: AppHandle<R>, id: String, fee: String) -> Result<Sent> {
    ops::cancel(&state(&app), &id, &fee).await
}

#[command]
async fn get_auctions<R: Runtime>(app: AppHandle<R>) -> Result<Vec<MyAuction>> {
    ops::list(&state(&app)).await
}

#[command]
async fn delete_auction<R: Runtime>(app: AppHandle<R>, id: String) -> Result<bool> {
    ops::forget(&state(&app), &id).await
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("klimper-auctions")
        .invoke_handler(tauri::generate_handler![
            view_auction,
            start_auction,
            bid_auction,
            end_auction,
            cancel_auction,
            get_auctions,
            delete_auction,
        ])
        .build()
}
