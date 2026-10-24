// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Klimper card (clapandpay milestone gnosis-card-mvp): a Visa card through Gnosis Pay,
//! on a self-custodied Safe on Gnosis Chain, owned by a key derived from the wallet's own
//! recovery words. Design and privacy rules: `docs/card.md`.
//!
//! Everything here runs on the device. There is no Klimper server in the card's path.

use std::collections::HashMap;
use std::sync::Arc;

use sage::Sage;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use tauri::{
    AppHandle, Manager, Runtime, State, command,
    plugin::{Builder, TauriPlugin},
};
use tokio::sync::Mutex;

pub mod account;
pub mod evm;
#[cfg(any(test, feature = "fake"))]
pub mod fake;
pub mod gnosis_pay;
pub mod onboarding;
pub mod siwe;

use evm::{Address, EvmKey};
use gnosis_pay::{ApiError, GnosisPay, Settings};
use onboarding::{Step, Term};

/// What `src-tauri` manages (`src-tauri/src/app_state.rs`).
pub type AppState = Arc<Mutex<Sage>>;

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("no wallet is open")]
    NoWallet,
    #[error("this wallet has no recovery words, so it has no card key")]
    NoRecoveryWords,
    #[error("the wallet's keys could not be read")]
    Keychain,
    #[error("this build of Klimper has no card")]
    NotConfigured,
    #[error("{0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Key(#[from] evm::KeyError),
    #[error(transparent)]
    Api(#[from] ApiError),
}

impl Error {
    /// A stable name for the webview to branch on.
    fn kind(&self) -> &'static str {
        match self {
            Self::NoWallet => "noWallet",
            Self::NoRecoveryWords => "noRecoveryWords",
            Self::Keychain | Self::Key(_) => "key",
            Self::NotConfigured => "notConfigured",
            Self::Invalid(_) => "invalid",
            Self::Api(ApiError::SignedOut) => "signedOut",
            Self::Api(ApiError::Siwe(_)) => "refusedMessage",
            Self::Api(ApiError::Network) => "network",
            Self::Api(ApiError::Status { .. } | ApiError::Unreadable) => "api",
        }
    }
}

/// `{ kind, message, code }`: what the webview's promise rejects with.
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("CardError", 3)?;
        s.serialize_field("kind", self.kind())?;
        s.serialize_field("message", &self.to_string())?;
        s.serialize_field(
            "code",
            &match self {
                Self::Api(api) => api.code(),
                _ => None,
            },
        )?;
        s.end()
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// The card's own state. The address per wallet is cached (it is public); the key is not.
#[derive(Debug)]
pub struct Card {
    addresses: Mutex<HashMap<u32, Address>>,
    api: Option<GnosisPay>,
}

impl Card {
    pub fn new(settings: Option<Settings>) -> Self {
        Self {
            addresses: Mutex::new(HashMap::new()),
            api: settings.map(GnosisPay::new),
        }
    }

    pub fn api(&self) -> Result<&GnosisPay> {
        self.api.as_ref().ok_or(Error::NotConfigured)
    }
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

/// The open wallet's session; `SignedOut` when it has none.
pub async fn session<'a>(card: &'a Card, sage: &AppState) -> Result<&'a GnosisPay> {
    let api = card.api()?;
    let (fingerprint, address) = address(card, sage).await?;
    if api.signed_in(fingerprint).await != Some(address) {
        return Err(ApiError::SignedOut.into());
    }
    Ok(api)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardStatus {
    /// Whether this build was made with a card registration (docs/card.md).
    pub configured: bool,
    /// `sandbox`, `production` or `custom`.
    pub environment: Option<&'static str>,
    /// Whether this wallet can have a card at all.
    pub available: bool,
    /// Why not, when it cannot: `noWallet` or `noRecoveryWords`.
    pub unavailable: Option<&'static str>,
    /// The card address (EIP-55), when available.
    pub address: Option<String>,
    pub signed_in: bool,
}

#[command]
async fn card_status<R: Runtime>(app: AppHandle<R>, card: State<'_, Card>) -> Result<CardStatus> {
    let sage = app.state::<AppState>().inner().clone();
    let configured = card.api.is_some();
    let environment = card.api.as_ref().map(|api| api.settings().environment());
    let (fingerprint, address) = match address(&card, &sage).await {
        Ok(found) => found,
        Err(error @ (Error::NoWallet | Error::NoRecoveryWords)) => {
            return Ok(CardStatus {
                configured,
                environment,
                available: false,
                unavailable: Some(error.kind()),
                address: None,
                signed_in: false,
            });
        }
        Err(error) => return Err(error),
    };
    let signed_in = match &card.api {
        Some(api) => api.signed_in(fingerprint).await == Some(address),
        None => false,
    };
    Ok(CardStatus {
        configured,
        environment,
        available: true,
        unavailable: None,
        address: Some(address.checksummed()),
        signed_in,
    })
}

/// Signs in to Gnosis Pay with the card key. The webview asks for Face ID or Touch ID first
/// (Sage's "Require biometrics"), as for any signature.
#[command]
async fn card_sign_in<R: Runtime>(app: AppHandle<R>, card: State<'_, Card>) -> Result<()> {
    let api = card.api()?;
    let sage = app.state::<AppState>().inner().clone();
    let (fingerprint, address) = address(&card, &sage).await?;
    let message = api.challenge(address, now()).await?;
    let (_, signature) = with_key(&sage, |key| {
        // The wallet may have been switched between the two steps.
        if key.address() != address {
            return Err(Error::NoWallet);
        }
        Ok(key.personal_sign(message.as_bytes())?)
    })
    .await?;
    api.verify(fingerprint, address, &message, &signature)
        .await?;
    Ok(())
}

#[command]
async fn card_sign_out(card: State<'_, Card>) -> Result<()> {
    card.api()?.sign_out().await;
    Ok(())
}

// Onboarding (TASK-150). Each command needs the open wallet's session; the e-mail address and
// the phone number pass through to Gnosis Pay and are not kept or logged.

#[command]
async fn card_onboarding<R: Runtime>(app: AppHandle<R>, card: State<'_, Card>) -> Result<Step> {
    let sage = app.state::<AppState>().inner().clone();
    Ok(session(&card, &sage).await?.onboarding().await?)
}

/// `true` when a code was sent, `false` when this card programme needs none.
#[command]
async fn card_send_email_code<R: Runtime>(
    app: AppHandle<R>,
    card: State<'_, Card>,
    email: String,
) -> Result<bool> {
    if !onboarding::valid_email(email.trim()) {
        return Err(Error::Invalid("that does not look like an e-mail address"));
    }
    let sage = app.state::<AppState>().inner().clone();
    Ok(session(&card, &sage)
        .await?
        .send_email_code(email.trim())
        .await?)
}

#[command]
async fn card_register<R: Runtime>(
    app: AppHandle<R>,
    card: State<'_, Card>,
    email: String,
    code: Option<String>,
) -> Result<()> {
    if !onboarding::valid_email(email.trim()) {
        return Err(Error::Invalid("that does not look like an e-mail address"));
    }
    if code.as_deref().is_some_and(|c| !onboarding::valid_code(c)) {
        return Err(Error::Invalid("the code has six digits"));
    }
    let sage = app.state::<AppState>().inner().clone();
    Ok(session(&card, &sage)
        .await?
        .register(email.trim(), code.as_deref())
        .await?)
}

#[command]
async fn card_accept_terms<R: Runtime>(
    app: AppHandle<R>,
    card: State<'_, Card>,
    terms: Vec<Term>,
) -> Result<()> {
    let sage = app.state::<AppState>().inner().clone();
    Ok(session(&card, &sage).await?.accept_terms(&terms).await?)
}

#[derive(Debug, serde::Deserialize)]
pub struct Answer {
    question: String,
    answer: String,
}

#[command]
async fn card_answer_source_of_funds<R: Runtime>(
    app: AppHandle<R>,
    card: State<'_, Card>,
    answers: Vec<Answer>,
) -> Result<()> {
    let answers: Vec<(String, String)> = answers
        .into_iter()
        .map(|a| (a.question, a.answer))
        .collect();
    let sage = app.state::<AppState>().inner().clone();
    Ok(session(&card, &sage)
        .await?
        .answer_source_of_funds(&answers)
        .await?)
}

#[command]
async fn card_create_account<R: Runtime>(app: AppHandle<R>, card: State<'_, Card>) -> Result<()> {
    let sage = app.state::<AppState>().inner().clone();
    Ok(session(&card, &sage).await?.create_account().await?)
}

#[command]
async fn card_send_phone_code<R: Runtime>(
    app: AppHandle<R>,
    card: State<'_, Card>,
    phone: String,
) -> Result<()> {
    let phone: String = phone.chars().filter(|c| !c.is_whitespace()).collect();
    if !onboarding::valid_phone(&phone) {
        return Err(Error::Invalid(
            "write the number with its country code, e.g. +43…",
        ));
    }
    let sage = app.state::<AppState>().inner().clone();
    Ok(session(&card, &sage).await?.send_phone_code(&phone).await?)
}

#[command]
async fn card_verify_phone<R: Runtime>(
    app: AppHandle<R>,
    card: State<'_, Card>,
    code: String,
) -> Result<()> {
    if !onboarding::valid_code(code.trim()) {
        return Err(Error::Invalid("the code has six digits"));
    }
    let sage = app.state::<AppState>().inner().clone();
    Ok(session(&card, &sage)
        .await?
        .verify_phone(code.trim())
        .await?)
}

#[command]
async fn card_order_card<R: Runtime>(
    app: AppHandle<R>,
    card: State<'_, Card>,
    name: String,
) -> Result<()> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 50 {
        return Err(Error::Invalid("a card name has 1 to 50 characters"));
    }
    let sage = app.state::<AppState>().inner().clone();
    Ok(session(&card, &sage)
        .await?
        .order_virtual_card(name)
        .await?)
}

// The account (TASK-151).

#[command]
async fn card_account<R: Runtime>(
    app: AppHandle<R>,
    card: State<'_, Card>,
) -> Result<account::Account> {
    let sage = app.state::<AppState>().inner().clone();
    Ok(session(&card, &sage).await?.account().await?)
}

#[command]
async fn card_transactions<R: Runtime>(
    app: AppHandle<R>,
    card: State<'_, Card>,
    cursor: Option<String>,
) -> Result<account::Statement> {
    let sage = app.state::<AppState>().inner().clone();
    Ok(session(&card, &sage)
        .await?
        .statement(cursor.as_deref())
        .await?)
}

#[command]
async fn card_set_frozen<R: Runtime>(
    app: AppHandle<R>,
    card: State<'_, Card>,
    card_id: String,
    frozen: bool,
) -> Result<()> {
    let sage = app.state::<AppState>().inner().clone();
    Ok(session(&card, &sage)
        .await?
        .set_frozen(&card_id, frozen)
        .await?)
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("klimper-card")
        .invoke_handler(tauri::generate_handler![
            card_status,
            card_sign_in,
            card_sign_out,
            card_onboarding,
            card_send_email_code,
            card_register,
            card_accept_terms,
            card_answer_source_of_funds,
            card_create_account,
            card_send_phone_code,
            card_verify_phone,
            card_order_card,
            card_account,
            card_transactions,
            card_set_frozen,
        ])
        .setup(|app, _api| {
            app.manage(Card::new(Settings::from_build()));
            Ok(())
        })
        .build()
}
