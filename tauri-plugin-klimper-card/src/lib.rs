// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Klimper card (clapandpay milestone gnosis-card-mvp): a Visa card through Gnosis Pay,
//! on a self-custodied Safe on Gnosis Chain, owned by a key derived from the wallet's own
//! recovery words. Design and privacy rules: `docs/card.md`.
//!
//! Everything here runs on the device. There is no Klimper server in the card's path.

use std::collections::HashMap;
use std::sync::Arc;

use sage::Sage;
use serde::{Serialize, Serializer};
use tauri::{
    AppHandle, Manager, Runtime, State, command,
    plugin::{Builder, TauriPlugin},
};
use tokio::sync::Mutex;

pub mod evm;

use evm::{Address, EvmKey};

/// What `src-tauri` manages (`src-tauri/src/app_state.rs`).
pub type AppState = Arc<Mutex<Sage>>;

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum Error {
    #[error("no wallet is open")]
    NoWallet,
    #[error("this wallet has no recovery words, so it has no card key")]
    NoRecoveryWords,
    #[error("the wallet's keys could not be read")]
    Keychain,
    #[error(transparent)]
    Key(#[from] evm::KeyError),
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// The card's own state. The address per wallet is cached (it is public); the key is not.
#[derive(Debug, Default)]
pub struct Card {
    addresses: Mutex<HashMap<u32, Address>>,
}

/// Opens the active wallet's card key for the length of `f`, then drops it.
pub async fn with_key<T>(
    sage: &AppState,
    f: impl FnOnce(&EvmKey) -> Result<T>,
) -> Result<(u32, T)> {
    let sage = sage.lock().await;
    let fingerprint = sage.config.global.fingerprint.ok_or(Error::NoWallet)?;
    let (mnemonic, secret) = sage
        .keychain
        .extract_secrets(fingerprint, b"")
        .map_err(|error| {
            tracing::warn!("card: the keychain refused: {error}");
            Error::Keychain
        })?;
    drop(sage);
    let Some(mnemonic) = mnemonic else {
        return Err(if secret.is_some() {
            Error::NoRecoveryWords
        } else {
            Error::NoWallet
        });
    };
    // `bip39::Mnemonic` is not scrubbed on drop (no zeroize feature in this workspace's
    // bip39 build); it goes out of scope at the end of this function, as in Sage itself.
    let key = EvmKey::from_mnemonic(&mnemonic)?;
    Ok((fingerprint, f(&key)?))
}

/// The card address of the open wallet, derived once per wallet per run.
pub async fn address(card: &Card, sage: &AppState) -> Result<(u32, Address)> {
    let fingerprint = sage
        .lock()
        .await
        .config
        .global
        .fingerprint
        .ok_or(Error::NoWallet)?;
    if let Some(address) = card.addresses.lock().await.get(&fingerprint) {
        return Ok((fingerprint, *address));
    }
    let (fingerprint, address) = with_key(sage, |key| Ok(key.address())).await?;
    card.addresses.lock().await.insert(fingerprint, address);
    Ok((fingerprint, address))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardStatus {
    /// Whether this wallet can have a card at all.
    pub available: bool,
    /// Why not, when it cannot: `noWallet` or `noRecoveryWords`.
    pub unavailable: Option<&'static str>,
    /// The card address (EIP-55), when available.
    pub address: Option<String>,
}

#[command]
async fn card_status<R: Runtime>(app: AppHandle<R>, card: State<'_, Card>) -> Result<CardStatus> {
    let sage = app.state::<AppState>().inner().clone();
    Ok(match address(&card, &sage).await {
        Ok((_, address)) => CardStatus {
            available: true,
            unavailable: None,
            address: Some(address.checksummed()),
        },
        Err(Error::NoWallet) => CardStatus {
            available: false,
            unavailable: Some("noWallet"),
            address: None,
        },
        Err(Error::NoRecoveryWords) => CardStatus {
            available: false,
            unavailable: Some("noRecoveryWords"),
            address: None,
        },
        Err(error) => return Err(error),
    })
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("klimper-card")
        .invoke_handler(tauri::generate_handler![card_status])
        .setup(|app, _api| {
            app.manage(Card::default());
            Ok(())
        })
        .build()
}
