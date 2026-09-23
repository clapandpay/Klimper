// SPDX-License-Identifier: AGPL-3.0-or-later
//! Klimper passkey login. The Sage keychain is encrypted with a random wallet key; the wallet
//! key is sealed in an [`envelope::Envelope`] that only the passkey's PRF output opens. The
//! PRF output comes from the Swift side straight into Rust and never reaches the webview.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use sage::Sage;
use serde::{Deserialize, Serialize, Serializer};
use tauri::{
    AppHandle, Manager, Runtime, State, command,
    plugin::{Builder, TauriPlugin},
};
use tokio::sync::Mutex;

pub mod envelope;
mod platform;

use envelope::{Envelope, EnvelopeError, decode_prf_output, random_secret};
use platform::Passkeys;

/// The relying party: passkeys are bound to this domain, which must list the app in its
/// `apple-app-site-association` file (docs/passkey.md).
pub const RP_ID: &str = "clapandpay.com";
const USER_NAME: &str = "Klimper wallet";

type AppState = Arc<Mutex<Sage>>;

/// Set once the passkey has unlocked the keychain in this run of the app.
#[derive(Default)]
struct Unlocked(AtomicBool);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("passkeys need iOS 18 or later on an iPhone or iPad")]
    Unsupported,
    #[error("a passkey already protects this wallet")]
    AlreadyEnrolled,
    #[error("no passkey protects this wallet")]
    NotEnrolled,
    #[error("the passkey did not return a PRF output; it can't protect the keys")]
    NoPrf,
    #[error("passkey: {0}")]
    Platform(String),
    #[error(transparent)]
    Envelope(#[from] EnvelopeError),
    #[error(transparent)]
    Sage(#[from] sage::Error),
    #[error(transparent)]
    Keychain(#[from] sage_keychain::KeychainError),
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub supported: bool,
    pub enrolled: bool,
    pub locked: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegisterRequest {
    pub rp_id: String,
    pub user_id: String,
    pub user_name: String,
    pub challenge: String,
    pub prf_salt: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegisterResponse {
    pub credential_id: String,
    pub prf_output: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssertRequest {
    pub rp_id: String,
    pub credential_id: String,
    pub challenge: String,
    pub prf_salt: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssertResponse {
    pub prf_output: Option<String>,
}

/// Nothing verifies the assertion signature (there is no server); the challenge only has to
/// be fresh. The security rests on the PRF output, which needs the passkey and Face ID.
fn challenge() -> String {
    B64.encode(random_secret().as_ref())
}

/// Runs a Face ID prompt off the async runtime, without holding the Sage lock.
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(job)
        .await
        .map_err(|error| Error::Platform(error.to_string()))?
}

async fn prf_from_assertion<R: Runtime>(
    app: &AppHandle<R>,
    envelope: &Envelope,
) -> Result<envelope::Secret> {
    let passkeys = app.state::<Passkeys<R>>().inner().clone();
    let request = AssertRequest {
        rp_id: envelope.rp_id.clone(),
        credential_id: envelope.credential_id.clone(),
        challenge: challenge(),
        prf_salt: envelope.prf_salt.clone(),
    };
    let response = blocking(move || passkeys.assert(request)).await?;
    Ok(decode_prf_output(
        &response.prf_output.ok_or(Error::NoPrf)?,
    )?)
}

/// Re-keys the keychain to the wallet key and stores the envelope next to it.
fn seal_keychain(sage: &mut Sage, envelope: &Envelope, wallet_key: &[u8; 32]) -> Result<()> {
    if Envelope::load(&sage.path)?.is_some() {
        return Err(Error::AlreadyEnrolled);
    }
    let old_password = sage.keychain.password()?;

    // Envelope first: if the app dies before keys.bin is re-keyed, unlock finishes the job.
    envelope.save(&sage.path)?;
    sage.keychain.change_password(wallet_key)?;
    if let Err(error) = sage.save_keychain() {
        sage.keychain.change_password(&old_password)?;
        Envelope::delete(&sage.path)?;
        return Err(error.into());
    }
    Ok(())
}

fn open_keychain(sage: &mut Sage, wallet_key: &[u8; 32]) -> Result<()> {
    match sage.keychain.unlock(wallet_key) {
        Ok(()) => Ok(()),
        // Enrolment was interrupted after the envelope was written: finish re-keying.
        Err(sage_keychain::KeychainError::Decrypt) => {
            sage.keychain.unlock(b"")?;
            sage.keychain.change_password(wallet_key)?;
            sage.save_keychain()?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// Back to Sage's empty password; keys.bin first, so a crash leaves a state unlock repairs.
fn unseal_keychain(sage: &mut Sage, wallet_key: &[u8; 32]) -> Result<()> {
    sage.keychain.unlock(wallet_key)?;
    sage.keychain.change_password(b"")?;
    sage.save_keychain()?;
    Envelope::delete(&sage.path)?;
    Ok(())
}

#[command]
async fn status<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    unlocked: State<'_, Unlocked>,
) -> Result<Status> {
    let mut sage = state.lock().await;
    let enrolled = Envelope::load(&sage.path)?.is_some();

    // The keychain starts unlocked with Sage's empty password; with a passkey it stays
    // locked until the passkey opens it.
    if enrolled && !unlocked.0.load(Ordering::SeqCst) {
        sage.keychain.lock();
    }

    Ok(Status {
        supported: app.state::<Passkeys<R>>().supported(),
        enrolled,
        locked: sage.keychain.is_locked(),
    })
}

#[command]
async fn enroll<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    unlocked: State<'_, Unlocked>,
) -> Result<()> {
    let passkeys = app.state::<Passkeys<R>>().inner().clone();
    if !passkeys.supported() {
        return Err(Error::Unsupported);
    }

    {
        let sage = state.lock().await;
        if Envelope::load(&sage.path)?.is_some() {
            return Err(Error::AlreadyEnrolled);
        }
        sage.keychain.password()?;
    }

    let prf_salt = Envelope::new_prf_salt();
    let request = RegisterRequest {
        rp_id: RP_ID.to_string(),
        user_id: B64.encode(random_secret().as_ref()),
        user_name: USER_NAME.to_string(),
        challenge: challenge(),
        prf_salt: prf_salt.clone(),
    };
    let registration = blocking({
        let passkeys = passkeys.clone();
        move || passkeys.register(request)
    })
    .await?;

    // Some authenticators evaluate the PRF only on assertion: ask once more if needed.
    let prf = if let Some(output) = registration.prf_output {
        decode_prf_output(&output)?
    } else {
        let request = AssertRequest {
            rp_id: RP_ID.to_string(),
            credential_id: registration.credential_id.clone(),
            challenge: challenge(),
            prf_salt: prf_salt.clone(),
        };
        let response = blocking(move || passkeys.assert(request)).await?;
        decode_prf_output(&response.prf_output.ok_or(Error::NoPrf)?)?
    };

    let wallet_key = random_secret();
    let envelope = Envelope::seal(
        RP_ID,
        &registration.credential_id,
        &prf_salt,
        &prf,
        &wallet_key,
    )?;

    seal_keychain(&mut *state.lock().await, &envelope, &wallet_key)?;
    unlocked.0.store(true, Ordering::SeqCst);
    Ok(())
}

#[command]
async fn unlock<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    unlocked: State<'_, Unlocked>,
) -> Result<()> {
    let envelope = Envelope::load(&state.lock().await.path)?.ok_or(Error::NotEnrolled)?;
    let prf = prf_from_assertion(&app, &envelope).await?;
    let wallet_key = envelope.open(&prf)?;

    open_keychain(&mut *state.lock().await, &wallet_key)?;
    unlocked.0.store(true, Ordering::SeqCst);
    Ok(())
}

#[command]
async fn lock(state: State<'_, AppState>, unlocked: State<'_, Unlocked>) -> Result<()> {
    let mut sage = state.lock().await;
    if Envelope::load(&sage.path)?.is_some() {
        unlocked.0.store(false, Ordering::SeqCst);
        sage.keychain.lock();
    }
    Ok(())
}

/// Back to Sage's empty password. Needs the passkey once more, so a stolen unlocked phone
/// can't quietly strip the protection.
#[command]
async fn remove<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    unlocked: State<'_, Unlocked>,
) -> Result<()> {
    let envelope = Envelope::load(&state.lock().await.path)?.ok_or(Error::NotEnrolled)?;
    let prf = prf_from_assertion(&app, &envelope).await?;
    let wallet_key = envelope.open(&prf)?;

    unseal_keychain(&mut *state.lock().await, &wallet_key)?;
    unlocked.0.store(false, Ordering::SeqCst);
    Ok(())
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("klimper-passkey")
        .invoke_handler(tauri::generate_handler![
            status, enroll, unlock, lock, remove
        ])
        .setup(|app, api| {
            app.manage(platform::init(app, api)?);
            app.manage(Unlocked::default());
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use bip39::Mnemonic;
    use sage_keychain::{Keychain, KeychainError};

    use super::*;

    fn sage_with_wallet(dir: &std::path::Path) -> (Sage, u32) {
        let mut sage = Sage::new(dir, true);
        let mnemonic = Mnemonic::from_entropy(&[9; 32]).unwrap();
        let fingerprint = sage.keychain.add_mnemonic(&mnemonic, b"").unwrap();
        sage.save_keychain().unwrap();
        (sage, fingerprint)
    }

    fn reload(sage: &Sage) -> Keychain {
        Keychain::from_bytes(&std::fs::read(sage.path.join("keys.bin")).unwrap()).unwrap()
    }

    fn sealed(prf: &[u8; 32], wallet_key: &[u8; 32]) -> Envelope {
        Envelope::seal(RP_ID, "Y3JlZA", &Envelope::new_prf_salt(), prf, wallet_key).unwrap()
    }

    #[test]
    fn enrol_lock_unlock_remove() {
        let dir = tempfile::tempdir().unwrap();
        let (mut sage, fingerprint) = sage_with_wallet(dir.path());
        let (prf, wallet_key) = (random_secret(), random_secret());

        seal_keychain(&mut sage, &sealed(&prf, &wallet_key), &wallet_key).unwrap();
        assert!(matches!(
            reload(&sage).extract_secrets(fingerprint, b""),
            Err(KeychainError::Decrypt)
        ));

        sage.keychain.lock();
        assert!(matches!(
            sage.keychain.password(),
            Err(KeychainError::Locked)
        ));

        let envelope = Envelope::load(&sage.path).unwrap().unwrap();
        open_keychain(&mut sage, &envelope.open(&prf).unwrap()).unwrap();
        let password = sage.keychain.password().unwrap();
        assert!(
            sage.keychain
                .extract_secrets(fingerprint, &password)
                .unwrap()
                .1
                .is_some()
        );

        unseal_keychain(&mut sage, &wallet_key).unwrap();
        assert!(Envelope::load(&sage.path).unwrap().is_none());
        assert!(
            reload(&sage)
                .extract_secrets(fingerprint, b"")
                .unwrap()
                .1
                .is_some()
        );
    }

    #[test]
    fn a_second_enrolment_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut sage, _) = sage_with_wallet(dir.path());
        let wallet_key = random_secret();
        seal_keychain(
            &mut sage,
            &sealed(&random_secret(), &wallet_key),
            &wallet_key,
        )
        .unwrap();
        let other = random_secret();
        assert!(matches!(
            seal_keychain(&mut sage, &sealed(&random_secret(), &other), &other),
            Err(Error::AlreadyEnrolled)
        ));
    }

    #[test]
    fn unlock_finishes_an_interrupted_enrolment() {
        let dir = tempfile::tempdir().unwrap();
        let (mut sage, fingerprint) = sage_with_wallet(dir.path());
        let (prf, wallet_key) = (random_secret(), random_secret());

        // The app died after writing the envelope, before re-keying keys.bin.
        sealed(&prf, &wallet_key).save(&sage.path).unwrap();
        sage.keychain.lock();

        let envelope = Envelope::load(&sage.path).unwrap().unwrap();
        open_keychain(&mut sage, &envelope.open(&prf).unwrap()).unwrap();
        assert!(matches!(
            reload(&sage).extract_secrets(fingerprint, b""),
            Err(KeychainError::Decrypt)
        ));
        assert!(
            reload(&sage)
                .extract_secrets(fingerprint, wallet_key.as_ref())
                .is_ok()
        );
    }

    #[test]
    fn a_wrong_wallet_key_does_not_unlock() {
        let dir = tempfile::tempdir().unwrap();
        let (mut sage, _) = sage_with_wallet(dir.path());
        let wallet_key = random_secret();
        seal_keychain(
            &mut sage,
            &sealed(&random_secret(), &wallet_key),
            &wallet_key,
        )
        .unwrap();
        sage.keychain.lock();
        assert!(open_keychain(&mut sage, &random_secret()).is_err());
        assert!(sage.keychain.is_locked());
    }
}
