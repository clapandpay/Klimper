// SPDX-License-Identifier: AGPL-3.0-or-later
//! The envelope: the wallet key that encrypts the Sage keychain, wrapped with a key derived
//! from the passkey's PRF output. Stored next to `keys.bin` as `passkey.json`; it holds no
//! secret on its own, only the passkey (behind Face ID) can open it.

use std::{fs, io, path::Path};

use aes_gcm::{
    Aes256Gcm, Key, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use hkdf::Hkdf;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

pub const FILE_NAME: &str = "passkey.json";
const VERSION: u8 = 1;
const HKDF_INFO: &[u8] = b"klimper/keychain-envelope/v1";

/// A 32-byte secret: the wallet key or a PRF output. Zeroized on drop.
pub type Secret = Zeroizing<[u8; 32]>;

#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    #[error("the passkey did not open the envelope")]
    Unwrap,
    #[error("unsupported envelope version {0}")]
    Version(u8),
    #[error("malformed envelope: {0}")]
    Malformed(&'static str),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u8,
    pub rp_id: String,
    /// base64url, as the platform returns it.
    pub credential_id: String,
    /// base64url; the PRF input sent with every assertion.
    pub prf_salt: String,
    /// base64url AES-256-GCM nonce and ciphertext of the wallet key.
    pub nonce: String,
    pub wrapped_key: String,
}

pub fn random_secret() -> Secret {
    let mut secret = Zeroizing::new([0; 32]);
    OsRng.fill_bytes(secret.as_mut());
    secret
}

fn wrapping_key(prf_output: &[u8; 32], prf_salt: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut key = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(Some(prf_salt), prf_output)
        .expand(HKDF_INFO, key.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    key
}

/// Everything but the ciphertext is authenticated, so no field can be swapped.
fn associated_data(version: u8, rp_id: &str, credential_id: &str, prf_salt: &str) -> Vec<u8> {
    format!("klimper-envelope\0{version}\0{rp_id}\0{credential_id}\0{prf_salt}").into_bytes()
}

impl Envelope {
    /// The PRF salt to evaluate before calling [`Envelope::seal`].
    pub fn new_prf_salt() -> String {
        B64.encode(random_secret().as_ref())
    }

    pub fn seal(
        rp_id: &str,
        credential_id: &str,
        prf_salt: &str,
        prf_output: &[u8; 32],
        wallet_key: &[u8; 32],
    ) -> Result<Self, EnvelopeError> {
        let salt = B64
            .decode(prf_salt)
            .map_err(|_| EnvelopeError::Malformed("prf_salt"))?;
        let key = wrapping_key(prf_output, &salt);
        let mut nonce = [0; 12];
        OsRng.fill_bytes(&mut nonce);

        let aad = associated_data(VERSION, rp_id, credential_id, prf_salt);
        let wrapped = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_ref()))
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: wallet_key,
                    aad: &aad,
                },
            )
            .map_err(|_| EnvelopeError::Unwrap)?;

        Ok(Self {
            version: VERSION,
            rp_id: rp_id.to_string(),
            credential_id: credential_id.to_string(),
            prf_salt: prf_salt.to_string(),
            nonce: B64.encode(nonce),
            wrapped_key: B64.encode(wrapped),
        })
    }

    pub fn open(&self, prf_output: &[u8; 32]) -> Result<Secret, EnvelopeError> {
        if self.version != VERSION {
            return Err(EnvelopeError::Version(self.version));
        }
        let salt = B64
            .decode(&self.prf_salt)
            .map_err(|_| EnvelopeError::Malformed("prf_salt"))?;
        let nonce = B64
            .decode(&self.nonce)
            .ok()
            .filter(|nonce| nonce.len() == 12)
            .ok_or(EnvelopeError::Malformed("nonce"))?;
        let wrapped = B64
            .decode(&self.wrapped_key)
            .map_err(|_| EnvelopeError::Malformed("wrapped_key"))?;

        let key = wrapping_key(prf_output, &salt);
        let aad = associated_data(
            self.version,
            &self.rp_id,
            &self.credential_id,
            &self.prf_salt,
        );
        let plain = Zeroizing::new(
            Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key.as_ref()))
                .decrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &wrapped,
                        aad: &aad,
                    },
                )
                .map_err(|_| EnvelopeError::Unwrap)?,
        );

        let mut wallet_key = Zeroizing::new([0; 32]);
        if plain.len() != 32 {
            return Err(EnvelopeError::Malformed("wallet key length"));
        }
        wallet_key.copy_from_slice(&plain);
        Ok(wallet_key)
    }

    pub fn load(dir: &Path) -> Result<Option<Self>, EnvelopeError> {
        match fs::read(dir.join(FILE_NAME)) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Write then rename, so a crash leaves either the old file or the new one.
    pub fn save(&self, dir: &Path) -> Result<(), EnvelopeError> {
        let temp_path = dir.join(format!("{FILE_NAME}.tmp"));
        fs::write(&temp_path, serde_json::to_vec_pretty(self)?)?;
        fs::rename(temp_path, dir.join(FILE_NAME))?;
        Ok(())
    }

    pub fn delete(dir: &Path) -> Result<(), EnvelopeError> {
        match fs::remove_file(dir.join(FILE_NAME)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }
}

/// Decodes a base64url PRF output from the platform into a 32-byte secret.
pub fn decode_prf_output(value: &str) -> Result<Secret, EnvelopeError> {
    let bytes = Zeroizing::new(
        B64.decode(value.trim_end_matches('='))
            .map_err(|_| EnvelopeError::Malformed("prf output"))?,
    );
    if bytes.len() != 32 {
        return Err(EnvelopeError::Malformed("prf output length"));
    }
    let mut secret = Zeroizing::new([0; 32]);
    secret.copy_from_slice(&bytes);
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sealed() -> (Envelope, Secret, Secret) {
        let prf = random_secret();
        let wallet_key = random_secret();
        let envelope = Envelope::seal(
            "clapandpay.com",
            "Y3JlZA",
            &Envelope::new_prf_salt(),
            &prf,
            &wallet_key,
        )
        .unwrap();
        (envelope, prf, wallet_key)
    }

    #[test]
    fn the_right_prf_output_opens_it() {
        let (envelope, prf, wallet_key) = sealed();
        assert_eq!(*envelope.open(&prf).unwrap(), *wallet_key);
    }

    #[test]
    fn a_wrong_prf_output_does_not() {
        let (envelope, _, _) = sealed();
        assert!(matches!(
            envelope.open(&random_secret()),
            Err(EnvelopeError::Unwrap)
        ));
    }

    #[test]
    fn every_field_is_bound() {
        let (envelope, prf, _) = sealed();
        let tampered = [
            Envelope {
                rp_id: "evil.example".into(),
                ..envelope.clone()
            },
            Envelope {
                credential_id: "b3RoZXI".into(),
                ..envelope.clone()
            },
            Envelope {
                prf_salt: Envelope::new_prf_salt(),
                ..envelope.clone()
            },
        ];
        for envelope in tampered {
            assert!(envelope.open(&prf).is_err());
        }
        let wrong_version = Envelope {
            version: 2,
            ..envelope
        };
        assert!(matches!(
            wrong_version.open(&prf),
            Err(EnvelopeError::Version(2))
        ));
    }

    #[test]
    fn save_load_delete() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Envelope::load(dir.path()).unwrap().is_none());
        let (envelope, _, _) = sealed();
        envelope.save(dir.path()).unwrap();
        assert_eq!(Envelope::load(dir.path()).unwrap(), Some(envelope));
        Envelope::delete(dir.path()).unwrap();
        Envelope::delete(dir.path()).unwrap();
        assert!(Envelope::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn prf_output_must_be_32_bytes() {
        assert!(decode_prf_output(&B64.encode([1; 32])).is_ok());
        assert!(decode_prf_output(&B64.encode([1; 16])).is_err());
    }
}
