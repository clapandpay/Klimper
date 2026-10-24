// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Gnosis Pay user API, called from the device with the user's own session.
//!
//! * Every call is made here, in Rust: no token is ever in the webview, and the API's
//!   per-origin CORS list does not matter.
//! * The session (a 15-minute access token and a 7-day refresh token that rotates on every
//!   use) lives in memory only. Nothing about the card is written to disk.
//! * A refresh is serialised behind the session lock and skipped when another call already
//!   rotated the token: Gnosis Pay logs the whole session out when a refresh token is used
//!   twice.
//! * Error bodies are never logged or passed on whole (they can echo personal data); only
//!   the HTTP status and Gnosis Pay's short error code are.
//!
//! API reference: <https://docs.gnosispay.com> (the `OpenAPI` document at `/user-api/openapi.json`).

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::evm::Address;
use crate::siwe::{Expected, SiweError, SiweMessage};

/// Gnosis Pay's sandbox. A build points elsewhere only through `KLIMPER_CARD_API`.
pub const SANDBOX_API: &str = "https://core.sandbox.gnosispay.in/user-api";
pub const PRODUCTION_API: &str = "https://core.prod.gnosispay.com/user-api";
/// Gnosis Chain.
pub const GNOSIS_CHAIN_ID: u64 = 100;

const TIMEOUT: Duration = Duration::from_secs(20);

/// Where the card signs in. None of this is secret: the app name and the domain are in
/// every sign-in message. They come from the environment at build time, so the public
/// repository carries no partner registration (see docs/card.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub api: String,
    pub app_name: String,
    /// The SIWE domain Gnosis Pay whitelisted for `app_name`.
    pub domain: String,
    /// The SIWE URI; `https://{domain}` unless set.
    pub uri: String,
    pub chain_id: u64,
}

impl Settings {
    /// This build's settings, or `None` when the build was made without a card registration.
    pub fn from_build() -> Option<Self> {
        Self::from_values(
            option_env!("KLIMPER_CARD_API"),
            option_env!("KLIMPER_CARD_APP_NAME"),
            option_env!("KLIMPER_CARD_DOMAIN"),
            option_env!("KLIMPER_CARD_URI"),
            option_env!("KLIMPER_CARD_CHAIN_ID"),
        )
    }

    pub fn from_values(
        api: Option<&str>,
        app_name: Option<&str>,
        domain: Option<&str>,
        uri: Option<&str>,
        chain_id: Option<&str>,
    ) -> Option<Self> {
        let present = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        let app_name = present(app_name)?;
        let domain = present(domain)?;
        let api = present(api).unwrap_or_else(|| SANDBOX_API.to_string());
        // A release build talks to Gnosis Pay over https only.
        if !cfg!(debug_assertions) && !api.starts_with("https://") {
            return None;
        }
        let uri = present(uri).unwrap_or_else(|| format!("https://{domain}"));
        let chain_id = present(chain_id)
            .and_then(|id| id.parse().ok())
            .unwrap_or(GNOSIS_CHAIN_ID);
        Some(Self {
            api: api.trim_end_matches('/').to_string(),
            app_name,
            domain,
            uri,
            chain_id,
        })
    }

    /// `sandbox`, `production` or `custom`, for the screen.
    pub fn environment(&self) -> &'static str {
        match self.api.as_str() {
            SANDBOX_API => "sandbox",
            PRODUCTION_API => "production",
            _ => "custom",
        }
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ApiError {
    #[error("not signed in to the card")]
    SignedOut,
    #[error(transparent)]
    Siwe(#[from] SiweError),
    #[error("Gnosis Pay could not be reached")]
    Network,
    #[error("Gnosis Pay answered {status}{}", code.as_ref().map(|c| format!(" ({c})")).unwrap_or_default())]
    Status { status: u16, code: Option<String> },
    #[error("Gnosis Pay's answer was not understood")]
    Unreadable,
}

impl ApiError {
    /// Gnosis Pay's error code, e.g. `USER_ALREADY_EXISTS`.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Status { code, .. } => code.as_deref(),
            _ => None,
        }
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;

/// A bearer credential. Never printed, scrubbed when dropped.
#[derive(Clone)]
pub struct Token(Zeroizing<String>);

impl Token {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
    fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(…)")
    }
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        self.expose() == other.expose()
    }
}

#[derive(Debug)]
struct Session {
    /// The wallet the session belongs to; another wallet never uses it.
    fingerprint: u32,
    address: Address,
    access: Token,
    refresh: Token,
}

#[derive(Debug)]
pub struct GnosisPay {
    settings: Settings,
    http: reqwest::Client,
    session: Mutex<Option<Session>>,
}

#[derive(Deserialize)]
struct SiweChallenge {
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tokens {
    access_token: String,
    refresh_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessOnly {
    access_token: String,
}

impl GnosisPay {
    pub fn new(settings: Settings) -> Self {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("Klimper/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("an HTTP client with a timeout");
        Self {
            settings,
            http,
            session: Mutex::new(None),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.settings.api)
    }

    /// Step 1 of the sign-in: Gnosis Pay's message for `address`, checked (siwe.rs) so that
    /// the only thing the caller can sign is a sign-in to this build's domain.
    pub async fn challenge(&self, address: Address, now: u64) -> ApiResult<String> {
        let response = self
            .http
            .get(self.url(&format!("/auth/siwe/{}", address.checksummed())))
            .query(&[
                ("domain", self.settings.domain.as_str()),
                ("uri", self.settings.uri.as_str()),
                ("appName", self.settings.app_name.as_str()),
            ])
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        let SiweChallenge { message } = read(response).await?;
        SiweMessage::check(
            &message,
            &Expected {
                domain: &self.settings.domain,
                uri: &self.settings.uri,
                address,
                chain_id: self.settings.chain_id,
                now,
            },
        )?;
        Ok(message)
    }

    /// Step 2: the signed message for a session, kept for `fingerprint` only.
    pub async fn verify(
        &self,
        fingerprint: u32,
        address: Address,
        message: &str,
        signature: &[u8; 65],
    ) -> ApiResult<()> {
        let response = self
            .http
            .post(self.url("/auth/siwe"))
            .json(&serde_json::json!({
                "address": address.checksummed(),
                "message": message,
                "signature": format!("0x{}", hex::encode(signature)),
            }))
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        let tokens: Tokens = read(response).await?;
        *self.session.lock().await = Some(Session {
            fingerprint,
            address,
            access: Token::new(tokens.access_token),
            refresh: Token::new(tokens.refresh_token),
        });
        Ok(())
    }

    /// The address signed in for `fingerprint`, if any. A session of another wallet is
    /// dropped (not revoked: its tokens are simply forgotten).
    pub async fn signed_in(&self, fingerprint: u32) -> Option<Address> {
        let mut session = self.session.lock().await;
        match session.as_ref() {
            Some(s) if s.fingerprint == fingerprint => Some(s.address),
            Some(_) => {
                *session = None;
                None
            }
            None => None,
        }
    }

    /// Revokes the session at Gnosis Pay (best effort) and forgets it here either way.
    pub async fn sign_out(&self) {
        let Some(session) = self.session.lock().await.take() else {
            return;
        };
        let result = self
            .http
            .post(self.url("/auth/logout"))
            .json(&serde_json::json!({ "refreshToken": session.refresh.expose() }))
            .send()
            .await;
        if !result.is_ok_and(|r| r.status().is_success()) {
            tracing::info!(
                "card: sign-out could not reach Gnosis Pay; the session is forgotten here"
            );
        }
    }

    /// Gnosis Pay's `POST /user` hands back a new access token that carries the user id;
    /// every later call must use it.
    pub async fn adopt_access_token(&self, access: String) {
        if let Some(session) = self.session.lock().await.as_mut() {
            session.access = Token::new(access);
        }
    }

    /// Rotates the session once, unless another call already did since `stale` was read.
    async fn refresh(&self, stale: &Token) -> ApiResult<()> {
        let mut guard = self.session.lock().await;
        let Some(session) = guard.as_mut() else {
            return Err(ApiError::SignedOut);
        };
        if &session.access != stale {
            return Ok(());
        }
        let response = self
            .http
            .post(self.url("/auth/refresh"))
            .json(&serde_json::json!({ "refreshToken": session.refresh.expose() }))
            .send()
            .await
            .map_err(|_| ApiError::Network)?;
        match read::<Tokens>(response).await {
            Ok(tokens) => {
                session.access = Token::new(tokens.access_token);
                session.refresh = Token::new(tokens.refresh_token);
                Ok(())
            }
            Err(ApiError::Network) => Err(ApiError::Network),
            Err(_) => {
                *guard = None;
                Err(ApiError::SignedOut)
            }
        }
    }

    /// An authenticated call. A 401 refreshes once and retries; a second 401 signs out.
    pub async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> ApiResult<T> {
        self.call_with_headers(method, path, body, &[]).await
    }

    /// [`Self::call`] with extra headers (the EIP-712 signature of a card challenge).
    pub async fn call_with_headers<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        headers: &[(&str, &str)],
    ) -> ApiResult<T> {
        for attempt in 0..2 {
            let access = match self.session.lock().await.as_ref() {
                Some(session) => session.access.clone(),
                None => return Err(ApiError::SignedOut),
            };
            let mut request = self
                .http
                .request(method.clone(), self.url(path))
                .bearer_auth(access.expose());
            if let Some(body) = body {
                request = request.json(body);
            }
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            let response = request.send().await.map_err(|_| ApiError::Network)?;
            if response.status() == StatusCode::UNAUTHORIZED {
                if attempt == 0 {
                    self.refresh(&access).await?;
                    continue;
                }
                *self.session.lock().await = None;
                return Err(ApiError::SignedOut);
            }
            return read(response).await;
        }
        Err(ApiError::SignedOut)
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        self.call(Method::GET, path, None).await
    }

    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> ApiResult<T> {
        self.call(Method::POST, path, Some(body)).await
    }

    /// `POST /user`: registers the signed-in address with an e-mail (and its code) and
    /// switches the session to the access token that carries the new user id.
    pub async fn register(&self, email: &str, otp: Option<&str>) -> ApiResult<()> {
        let mut body = serde_json::json!({ "email": email });
        if let Some(otp) = otp {
            body["otp"] = Value::String(otp.to_string());
        }
        let AccessOnly { access_token } = self.post("/user", &body).await?;
        self.adopt_access_token(access_token).await;
        Ok(())
    }
}

/// Reads a JSON answer; a failure keeps only the status and Gnosis Pay's short code.
async fn read<T: DeserializeOwned>(response: reqwest::Response) -> ApiResult<T> {
    let status = response.status();
    let bytes = response.bytes().await.map_err(|_| ApiError::Network)?;
    if !status.is_success() {
        return Err(ApiError::Status {
            status: status.as_u16(),
            code: error_code(&bytes),
        });
    }
    // A 2xx with an empty body (202, 204) reads as `null`.
    let bytes: &[u8] = if bytes.is_empty() { b"null" } else { &bytes };
    serde_json::from_slice(bytes).map_err(|_| ApiError::Unreadable)
}

/// `{"error": "USER_ALREADY_EXISTS"}`, `{"code": …}` or `{"message": …}`: kept only when it
/// looks like a code (upper case and underscores, short), never free text.
fn error_code(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    ["code", "error", "message"]
        .iter()
        .filter_map(|key| value.get(key)?.as_str())
        .find(|text| {
            text.len() <= 64
                && !text.is_empty()
                && text
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
        })
        .map(str::to_string)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;
    use crate::evm::EvmKey;
    use crate::fake::FakeGnosisPay;
    use crate::siwe::rfc3339;

    pub(crate) fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    pub(crate) fn key() -> EvmKey {
        use std::str::FromStr;
        EvmKey::from_mnemonic(
            &bip39::Mnemonic::from_str(
                "test test test test test test test test test test test junk",
            )
            .unwrap(),
        )
        .unwrap()
    }

    pub(crate) async fn signed_in(fake: &FakeGnosisPay) -> GnosisPay {
        let api = GnosisPay::new(fake.settings());
        let key = key();
        let message = api.challenge(key.address(), now()).await.unwrap();
        let signature = key.personal_sign(message.as_bytes()).unwrap();
        api.verify(7, key.address(), &message, &signature)
            .await
            .unwrap();
        api
    }

    #[test]
    fn settings_need_an_app_name_and_a_domain() {
        assert_eq!(Settings::from_values(None, None, None, None, None), None);
        assert_eq!(
            Settings::from_values(None, Some("klimper"), None, None, None),
            None
        );
        assert_eq!(
            Settings::from_values(None, Some(" "), Some("card.example"), None, None),
            None
        );
        let settings =
            Settings::from_values(None, Some("klimper"), Some("card.example"), None, None).unwrap();
        assert_eq!(settings.api, SANDBOX_API);
        assert_eq!(settings.uri, "https://card.example");
        assert_eq!(settings.chain_id, 100);
        assert_eq!(settings.environment(), "sandbox");
    }

    #[test]
    fn error_codes_are_kept_free_text_is_not() {
        assert_eq!(
            error_code(br#"{"error":"USER_ALREADY_EXISTS"}"#).as_deref(),
            Some("USER_ALREADY_EXISTS")
        );
        assert_eq!(
            error_code(br#"{"message":"Hello Jane Doe, your e-mail jane@example.com"}"#),
            None
        );
        assert_eq!(error_code(b"not json"), None);
    }

    #[test]
    fn tokens_never_print() {
        assert_eq!(
            format!("{:?}", Token::new("secret-access".into())),
            "Token(…)"
        );
    }

    #[tokio::test]
    async fn sign_in_signs_only_a_checked_message() {
        let fake = FakeGnosisPay::start().await;
        let api = signed_in(&fake).await;
        assert_eq!(api.signed_in(7).await, Some(key().address()));
        // The fake verified the signature against the address, as Gnosis Pay does.
        assert_eq!(fake.state().await.sign_ins, 1);
    }

    #[tokio::test]
    async fn a_message_for_another_site_is_never_signed() {
        let fake = FakeGnosisPay::start().await;
        fake.state().await.siwe_domain_override = Some("evil.example".into());
        let api = GnosisPay::new(fake.settings());
        assert_eq!(
            api.challenge(key().address(), now()).await,
            Err(ApiError::Siwe(SiweError::WrongDomain))
        );
        // An old message (a replay) is refused too.
        fake.state().await.siwe_domain_override = None;
        fake.state().await.siwe_issued_at_override = Some(rfc3339(now() - 3600));
        assert_eq!(
            api.challenge(key().address(), now()).await,
            Err(ApiError::Siwe(SiweError::WrongTime))
        );
    }

    #[tokio::test]
    async fn an_expired_access_token_is_refreshed_once_and_the_new_one_kept() {
        let fake = FakeGnosisPay::start().await;
        let api = signed_in(&fake).await;
        fake.state().await.expire_access();
        // Two calls at once: one refresh only, or Gnosis Pay would log the session out.
        let (a, b) = tokio::join!(
            api.get::<Value>("/user/balances/v2"),
            api.get::<Value>("/user/balances/v2")
        );
        a.unwrap();
        b.unwrap();
        let state = fake.state().await;
        assert_eq!(state.refreshes, 1);
        assert_eq!(state.reused_refresh_tokens, 0);
    }

    #[tokio::test]
    async fn a_refused_refresh_signs_out() {
        let fake = FakeGnosisPay::start().await;
        let api = signed_in(&fake).await;
        {
            let mut state = fake.state().await;
            state.expire_access();
            state.revoke_refresh();
        }
        assert_eq!(
            api.get::<Value>("/user/onboarding").await,
            Err(ApiError::SignedOut)
        );
        assert_eq!(api.signed_in(7).await, None);
    }

    #[tokio::test]
    async fn sign_out_revokes_at_gnosis_pay() {
        let fake = FakeGnosisPay::start().await;
        let api = signed_in(&fake).await;
        api.sign_out().await;
        assert_eq!(api.signed_in(7).await, None);
        assert_eq!(fake.state().await.logouts, 1);
        assert_eq!(
            api.get::<Value>("/user/onboarding").await,
            Err(ApiError::SignedOut)
        );
    }

    #[tokio::test]
    async fn another_wallet_does_not_see_the_session() {
        let fake = FakeGnosisPay::start().await;
        let api = signed_in(&fake).await;
        assert_eq!(api.signed_in(8).await, None);
        assert_eq!(api.signed_in(7).await, None);
    }
}
