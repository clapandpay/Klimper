// SPDX-License-Identifier: AGPL-3.0-or-later
//! The card's number, expiry and CVV, shown on the device only (TASK-153), so the virtual
//! card can be added to Apple Pay or Google Pay and used online (TASK-157).
//!
//! Gnosis Pay's direct PCI route, which needs no backend:
//!
//! 1. `GET /pci/cards/{id}/challenge?action=view-details`: EIP-712 typed data, checked by
//!    [`crate::eip712::check_card_challenge`] and signed by the card key after the
//!    biometric prompt.
//! 2. `GET /pci/public-key`: Gnosis Pay's RSA key. Klimper makes a fresh AES-256 key and
//!    wraps it with RSA-OAEP-SHA256, so only this device can read the answer.
//! 3. `GET /pci/cards/{id}/details?encryptedKey=…&keyId=…` with `x-eip712-signature` and
//!    `x-eip712-nonce`: number and CVV encrypted with that AES key (AES-GCM).
//!
//! The details are decrypted here, handed to the webview once to be shown, and never
//! logged or stored. The AES key is scrubbed when dropped.
//!
//! Whether every partner may use this route, and the exact AES-GCM layout (tag appended to
//! the ciphertext is assumed), are open questions for Gnosis Pay (TASK-147). Their
//! alternative, the secure elements (PSE), needs a partner backend with an mTLS
//! certificate.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Oaep, RsaPublicKey};
use serde::Serialize;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::account::valid_card_id;
use crate::gnosis_pay::{ApiError, ApiResult, GnosisPay};

/// What the webview shows. `Debug` prints nothing secret.
#[derive(Clone)]
pub struct CardDetails {
    pub number: Zeroizing<String>,
    pub cvv: Zeroizing<String>,
    /// `MM/YY`.
    pub expires: String,
}

impl Serialize for CardDetails {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("CardDetails", 3)?;
        s.serialize_field("number", self.number.as_str())?;
        s.serialize_field("cvv", self.cvv.as_str())?;
        s.serialize_field("expires", &self.expires)?;
        s.end()
    }
}

impl std::fmt::Debug for CardDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CardDetails(…)")
    }
}

/// Gnosis Pay's key for wrapping ours: `(keyId, key)`.
fn wrapping_key(body: &Value) -> ApiResult<(Option<String>, RsaPublicKey)> {
    let pick = body["encryptionKeys"]
        .as_array()
        .and_then(|keys| {
            keys.iter().find(|k| {
                k["algorithm"]
                    .as_str()
                    .is_none_or(|a| a.to_ascii_uppercase().contains("RSA"))
            })
        })
        .map(|k| {
            (
                k["id"].as_str().map(str::to_string),
                k["publicKey"].as_str(),
            )
        });
    let (id, der) = match pick {
        Some((id, Some(der))) => (id, der),
        _ => (
            None,
            body["publicKey"].as_str().ok_or(ApiError::Unreadable)?,
        ),
    };
    let der = STANDARD
        .decode(der.trim())
        .map_err(|_| ApiError::Unreadable)?;
    let key = RsaPublicKey::from_public_key_der(&der).map_err(|_| ApiError::Unreadable)?;
    Ok((id, key))
}

fn decrypt(key: &[u8; 32], iv: &[u8], text: &Value) -> ApiResult<Zeroizing<String>> {
    if iv.len() != 12 {
        return Err(ApiError::Unreadable);
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| ApiError::Unreadable)?;
    let sealed = STANDARD
        .decode(text.as_str().ok_or(ApiError::Unreadable)?)
        .map_err(|_| ApiError::Unreadable)?;
    let plain = Zeroizing::new(
        cipher
            .decrypt(Nonce::from_slice(iv), sealed.as_slice())
            .map_err(|_| ApiError::Unreadable)?,
    );
    let text = std::str::from_utf8(&plain).map_err(|_| ApiError::Unreadable)?;
    Ok(Zeroizing::new(text.to_string()))
}

impl GnosisPay {
    /// Step 1: the typed data to sign before the details are released.
    pub async fn card_challenge(&self, card_id: &str) -> ApiResult<Value> {
        if !valid_card_id(card_id) {
            return Err(ApiError::Unreadable);
        }
        self.get(&format!(
            "/pci/cards/{card_id}/challenge?action=view-details"
        ))
        .await
    }

    /// Steps 2 and 3, with the signed challenge.
    pub async fn card_details(
        &self,
        card_id: &str,
        signature: &[u8; 65],
        nonce: &str,
    ) -> ApiResult<CardDetails> {
        if !valid_card_id(card_id) || !nonce.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(ApiError::Unreadable);
        }
        let (key_id, public) = wrapping_key(&self.get::<Value>("/pci/public-key").await?)?;
        let aes = Zeroizing::new(rand::random::<[u8; 32]>());
        let wrapped = public
            .encrypt(
                &mut rand::rngs::OsRng,
                Oaep::new::<sha2::Sha256>(),
                aes.as_slice(),
            )
            .map_err(|_| ApiError::Unreadable)?;
        let mut query = vec![("encryptedKey", STANDARD.encode(wrapped))];
        if let Some(id) = key_id {
            query.push(("keyId", id));
        }
        let query = query
            .iter()
            .map(|(k, v)| format!("{k}={}", url_escape(v)))
            .collect::<Vec<_>>()
            .join("&");
        let signature = format!("0x{}", hex::encode(signature));
        let body: Value = self
            .call_with_headers(
                reqwest::Method::GET,
                &format!("/pci/cards/{card_id}/details?{query}"),
                None,
                &[
                    ("x-eip712-signature", &signature),
                    ("x-eip712-nonce", nonce),
                ],
            )
            .await?;
        let iv = STANDARD
            .decode(body["iv"].as_str().ok_or(ApiError::Unreadable)?)
            .map_err(|_| ApiError::Unreadable)?;
        Ok(CardDetails {
            number: decrypt(&aes, &iv, &body["cardNumber"])?,
            cvv: decrypt(&aes, &iv, &body["cvv"])?,
            expires: body["expirationDate"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(7)
                .collect(),
        })
    }
}

/// Percent-encoding for base64 in a query (`+`, `/`, `=`).
fn url_escape(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::eip712::check_card_challenge;
    use crate::fake::FakeGnosisPay;
    use crate::gnosis_pay::tests::{key, signed_in};

    #[test]
    fn details_never_print() {
        let details = CardDetails {
            number: Zeroizing::new("4000000000004242".into()),
            cvv: Zeroizing::new("123".into()),
            expires: "10/29".into(),
        };
        assert_eq!(format!("{details:?}"), "CardDetails(…)");
    }

    #[test]
    fn query_escaping() {
        assert_eq!(url_escape("a+b/c="), "a%2Bb%2Fc%3D");
    }

    #[tokio::test]
    async fn the_card_number_arrives_encrypted_to_this_device_only() {
        let fake = FakeGnosisPay::start().await;
        fake.state().await.onboarded(key().address());
        let api = signed_in(&fake).await;

        let challenge =
            check_card_challenge(&api.card_challenge("card-1").await.unwrap(), 100).unwrap();
        let signature = key().sign_digest(&challenge.digest).unwrap();
        let details = api
            .card_details("card-1", &signature, &challenge.nonce)
            .await
            .unwrap();
        assert_eq!(details.number.as_str(), "4000000000004242");
        assert_eq!(details.cvv.as_str(), "737");
        assert_eq!(details.expires, "10/29");
        // The fake saw only the wrapped key and the ciphertext it made for it.
        assert_eq!(fake.state().await.details_served, 1);

        // A challenge is single use, and a signature by another key is refused.
        assert!(
            api.card_details("card-1", &signature, &challenge.nonce)
                .await
                .is_err()
        );
        let challenge =
            check_card_challenge(&api.card_challenge("card-1").await.unwrap(), 100).unwrap();
        let other = crate::evm::EvmKey::from_secret(&[7; 32])
            .unwrap()
            .sign_digest(&challenge.digest)
            .unwrap();
        assert!(
            api.card_details("card-1", &other, &challenge.nonce)
                .await
                .is_err()
        );
    }
}
