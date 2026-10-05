// SPDX-License-Identifier: AGPL-3.0-or-later
//! Push registration (clapandpay TASK-087, xchonnect `docs/wallet-integration.md` section 4).
//!
//! A phone is never connected: the relay asks Klimper's own push gateway for a content-free
//! wake-up, and the session loop answers it by reading its mailbox. This module builds what
//! the relay stores: the APNs device token sealed to the gateway's public key, with a fresh
//! HPKE ephemeral key per call, so two sessions of one phone cannot be linked by the relay.
//!
//! Nothing here reads a notification. A wake-up only ever wakes the session loop; what it
//! carried is not parsed (the app never acts on push content).

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use xchonnect_core::b64;
use xchonnect_core::crypto::{Entropy, hmac_sha256, random_array};
use xchonnect_core::push::{MAX_LIFETIME_S, Platform, PushToken};

use crate::error::{Error, Result};

/// Event for the screen: what the user answered when asked for notification permission
/// (`{ "state": "granted" | "denied" | "undetermined" }`).
pub const EVENT_PUSH: &str = "klimper-xchonnect://push";

/// Re-register once less than this is left on the token (it lives at most 90 days).
pub const RENEW_BEFORE_S: u64 = 30 * 24 * 3600;

/// Where wake-ups go and who may open the sealed token. Unset means push is off and the
/// session loop keeps polling in the foreground.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// The gateway's wake endpoint, as the relay will call it.
    pub url: String,
    /// The gateway's X25519 public key (`GET /v1/keys`), rotated yearly.
    pub public_key: [u8; 32],
}

impl GatewayConfig {
    /// Read the build-time settings `KLIMPER_PUSH_GATEWAY_URL` and
    /// `KLIMPER_PUSH_GATEWAY_KEY` (base64url). Either one missing or malformed turns push
    /// off rather than failing the app.
    pub fn from_build() -> Option<Self> {
        Self::parse(
            option_env!("KLIMPER_PUSH_GATEWAY_URL")?,
            option_env!("KLIMPER_PUSH_GATEWAY_KEY")?,
        )
    }

    fn parse(url: &str, key: &str) -> Option<Self> {
        if !url.starts_with("https://") {
            return None;
        }
        let public_key = b64::decode(key).ok()?.try_into().ok()?;
        Some(GatewayConfig {
            url: url.to_owned(),
            public_key,
        })
    }
}

/// What the app keeps per session: enough to know when to register again, and the hint key
/// for encrypted previews (TASK-088). It never leaves the session's own record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registration {
    pub expires_at: u64,
    pub hint_key_hex: String,
    /// The mailbox the token was registered on. A key rotation moves the session to a new
    /// mailbox, which has no push token yet.
    #[serde(default)]
    pub mailbox: String,
    /// [`token_tag`] of the device token that was sealed, so a new one is noticed without
    /// keeping the token itself.
    #[serde(default)]
    pub token_tag: String,
}

impl Registration {
    /// True when the token should be sealed again: it is close to expiry, the session moved
    /// to another mailbox, or APNs gave the phone a new device token since.
    pub fn due(&self, mailbox: &str, token_tag: &str, now: u64) -> bool {
        self.expires_at.saturating_sub(now) < RENEW_BEFORE_S
            || self.mailbox != mailbox
            || self.token_tag != token_tag
    }
}

/// A short, non-reversible label for a device token: 8 bytes of an HMAC under a fixed key.
pub fn token_tag(device_token: &str) -> String {
    hmac_sha256(b"klimper push token tag", &[device_token.as_bytes()])
        .map(|mac| hex::encode(&mac[..8]))
        .unwrap_or_default()
}

/// What the loops share: where wake-ups go, and the phone's current APNs device token. The
/// token arrives from the native side (it can change at any launch) and is held in memory
/// only; each session stores just its [`token_tag`].
#[derive(Debug, Default)]
pub struct PushHandle {
    gateway: Option<GatewayConfig>,
    device_token: Mutex<Option<String>>,
}

impl PushHandle {
    pub fn new(gateway: Option<GatewayConfig>) -> Self {
        PushHandle {
            gateway,
            device_token: Mutex::new(None),
        }
    }

    pub fn gateway(&self) -> Option<&GatewayConfig> {
        self.gateway.as_ref()
    }

    pub fn device_token(&self) -> Option<String> {
        self.device_token.lock().ok()?.clone()
    }

    /// Remember the device token; true when it is new, so the loops should register it.
    pub fn set_device_token(&self, token: &str) -> bool {
        let Ok(mut held) = self.device_token.lock() else {
            return false;
        };
        if held.as_deref() == Some(token) {
            return false;
        }
        *held = Some(token.to_owned());
        true
    }
}

/// The body of `PUT /v1/mailboxes/{id}/push`, and the registration to store beside it.
#[derive(Debug)]
pub struct Sealed {
    pub body: serde_json::Value,
    pub registration: Registration,
}

/// Seal `device_token` for one session. `sandbox` is true for builds signed for development
/// (debug), where APNs is its sandbox; `TestFlight` and App Store builds use production.
pub fn seal(
    rng: &mut dyn Entropy,
    gateway: &GatewayConfig,
    device_token: &str,
    sandbox: bool,
    now: u64,
) -> Result<Sealed> {
    let hint_key = random_array::<32>(rng);
    let expires_at = now + MAX_LIFETIME_S;
    let token = PushToken {
        platform: if sandbox {
            Platform::ApnsSandbox
        } else {
            Platform::Apns
        },
        device_token: device_token.to_owned(),
        hint_key,
        exp: expires_at,
    };
    let sealed = token
        .seal(rng, &gateway.public_key, now)
        .map_err(Error::protocol)?;
    Ok(Sealed {
        body: serde_json::json!({
            "push_reg": {
                "gateway_url": gateway.url,
                "sealed_token": b64::encode(&sealed),
            }
        }),
        registration: Registration {
            expires_at,
            hint_key_hex: hex::encode(hint_key),
            mailbox: String::new(),
            token_tag: token_tag(device_token),
        },
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testing::NOW;
    use xchonnect_core::crypto::{OsEntropy, X25519Secret};

    fn gateway(secret: &X25519Secret) -> GatewayConfig {
        GatewayConfig {
            url: "https://push.example/v1/wake".to_owned(),
            public_key: secret.public_key(),
        }
    }

    #[test]
    fn the_gateway_opens_what_the_wallet_sealed_and_nobody_else_can() {
        let sk = X25519Secret::from_bytes([7; 32]);
        let sealed = seal(&mut OsEntropy, &gateway(&sk), "abcd1234", false, NOW).unwrap();
        let raw = b64::decode(sealed.body["push_reg"]["sealed_token"].as_str().unwrap()).unwrap();

        let opened = PushToken::open(&sk, &raw, NOW + 1).unwrap();
        assert_eq!(opened.platform, Platform::Apns);
        assert_eq!(opened.device_token, "abcd1234");
        assert_eq!(opened.exp, sealed.registration.expires_at);
        assert_eq!(
            hex::encode(opened.hint_key),
            sealed.registration.hint_key_hex
        );

        let other = X25519Secret::from_bytes([8; 32]);
        assert!(PushToken::open(&other, &raw, NOW + 1).is_err());
    }

    #[test]
    fn debug_builds_register_with_the_apns_sandbox() {
        let sk = X25519Secret::from_bytes([7; 32]);
        let sealed = seal(&mut OsEntropy, &gateway(&sk), "abcd1234", true, NOW).unwrap();
        let raw = b64::decode(sealed.body["push_reg"]["sealed_token"].as_str().unwrap()).unwrap();
        assert_eq!(
            PushToken::open(&sk, &raw, NOW).unwrap().platform,
            Platform::ApnsSandbox
        );
    }

    /// The relay must not be able to link two sessions of one phone.
    #[test]
    fn every_registration_is_sealed_afresh() {
        let sk = X25519Secret::from_bytes([7; 32]);
        let a = seal(&mut OsEntropy, &gateway(&sk), "abcd1234", false, NOW).unwrap();
        let b = seal(&mut OsEntropy, &gateway(&sk), "abcd1234", false, NOW).unwrap();
        assert_ne!(a.body, b.body);
        assert_ne!(a.registration.hint_key_hex, b.registration.hint_key_hex);
    }

    fn registered(device_token: &str) -> Registration {
        let sk = X25519Secret::from_bytes([7; 32]);
        let mut reg = seal(&mut OsEntropy, &gateway(&sk), device_token, false, NOW)
            .unwrap()
            .registration;
        reg.mailbox = "mbx".to_owned();
        reg
    }

    #[test]
    fn a_registration_is_renewed_thirty_days_before_it_expires() {
        let reg = registered("abcd1234");
        let tag = token_tag("abcd1234");
        assert!(!reg.due("mbx", &tag, NOW + 59 * 24 * 3600));
        assert!(reg.due("mbx", &tag, NOW + 61 * 24 * 3600));
        assert!(reg.due("mbx", &tag, reg.expires_at + 1));
    }

    #[test]
    fn a_rotated_mailbox_or_a_new_device_token_registers_again() {
        let reg = registered("abcd1234");
        assert!(!reg.due("mbx", &token_tag("abcd1234"), NOW));
        assert!(reg.due("other mailbox", &token_tag("abcd1234"), NOW));
        assert!(reg.due("mbx", &token_tag("ffff0000"), NOW));
    }

    #[test]
    fn the_tag_neither_contains_nor_repeats_the_token() {
        assert!(!token_tag("abcd1234").contains("abcd1234"));
        assert_eq!(token_tag("abcd1234"), token_tag("abcd1234"));
        assert_ne!(token_tag("abcd1234"), token_tag("abcd1235"));
    }

    #[test]
    fn only_a_changed_device_token_wakes_the_loops() {
        let handle = PushHandle::new(None);
        assert_eq!(handle.device_token(), None);
        assert!(handle.set_device_token("aa"));
        assert!(!handle.set_device_token("aa"));
        assert!(handle.set_device_token("bb"));
        assert_eq!(handle.device_token().as_deref(), Some("bb"));
    }

    #[test]
    fn a_gateway_needs_https_and_a_32_byte_key() {
        let key = b64::encode(&[1u8; 32]);
        assert!(GatewayConfig::parse("https://push.example/v1/wake", &key).is_some());
        assert!(GatewayConfig::parse("http://push.example/v1/wake", &key).is_none());
        assert!(GatewayConfig::parse("https://push.example", &b64::encode(&[1u8; 31])).is_none());
        assert!(GatewayConfig::parse("https://push.example", "not base64!").is_none());
    }
}
