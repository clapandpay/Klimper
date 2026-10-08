use std::collections::HashMap;

use bip39::Mnemonic;
use chia_wallet_sdk::prelude::*;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    KeychainError,
    encrypt::{decrypt, encrypt},
    key_data::{KeyData, SecretKeyData},
};

#[derive(Debug)]
pub struct Keychain {
    rng: ChaCha20Rng,
    keys: HashMap<u32, KeyData>,
    password: Password,
}

/// The password the secrets are encrypted with, held in memory while the keychain is unlocked.
/// A new keychain is unlocked with the empty password, which is what Sage has always used.
#[derive(Clone)]
struct Password(Option<Zeroizing<Vec<u8>>>);

impl Default for Password {
    fn default() -> Self {
        Self(Some(Zeroizing::new(Vec::new())))
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "Unlocked"
        } else {
            "Locked"
        })
    }
}

impl Default for Keychain {
    fn default() -> Self {
        Self {
            rng: ChaCha20Rng::from_entropy(),
            keys: HashMap::default(),
            password: Password::default(),
        }
    }
}

impl Keychain {
    /// Loads a keychain. It opens with Sage's empty password only if that password actually
    /// decrypts it; secrets sealed under another password (a passkey) stay locked from the
    /// first moment, so nothing that runs before the lock screen can read or add a key.
    pub fn from_bytes(data: &[u8]) -> Result<Self, KeychainError> {
        let keys = bincode::deserialize(data)?;
        let mut keychain = Self {
            rng: ChaCha20Rng::from_entropy(),
            keys,
            password: Password(None),
        };
        match keychain.unlock(b"") {
            Ok(()) | Err(KeychainError::Decrypt) => Ok(keychain),
            Err(error) => Err(error),
        }
    }

    /// The password to pass to the methods that read or add secrets.
    pub fn password(&self) -> Result<Zeroizing<Vec<u8>>, KeychainError> {
        self.password.0.clone().ok_or(KeychainError::Locked)
    }

    pub fn is_locked(&self) -> bool {
        self.password.0.is_none()
    }

    /// Forgets the password; secrets can't be read or added until [`Self::unlock`].
    pub fn lock(&mut self) {
        self.password.0 = None;
    }

    /// Checks the password against the stored secrets and keeps it in memory.
    pub fn unlock(&mut self, password: &[u8]) -> Result<(), KeychainError> {
        if let Some(KeyData::Secret { encrypted, .. }) = self
            .keys
            .values()
            .find(|key_data| matches!(key_data, KeyData::Secret { .. }))
        {
            decrypt::<SecretKeyData>(encrypted, password)?;
        }

        self.password.0 = Some(Zeroizing::new(password.to_vec()));
        Ok(())
    }

    /// Re-encrypts every secret with a new password. Requires the keychain to be unlocked;
    /// nothing changes if any secret fails to re-encrypt.
    pub fn change_password(&mut self, new_password: &[u8]) -> Result<(), KeychainError> {
        let old_password = self.password()?;
        let mut keys = HashMap::with_capacity(self.keys.len());

        for (fingerprint, key_data) in &self.keys {
            let key_data = match key_data {
                KeyData::Public { .. } => key_data.clone(),
                KeyData::Secret {
                    master_pk,
                    entropy,
                    encrypted,
                } => {
                    let mut data = decrypt::<SecretKeyData>(encrypted, &old_password)?;
                    let encrypted = encrypt(new_password, &mut self.rng, &data);
                    data.0.zeroize();
                    KeyData::Secret {
                        master_pk: *master_pk,
                        entropy: *entropy,
                        encrypted: encrypted?,
                    }
                }
            };
            keys.insert(*fingerprint, key_data);
        }

        self.keys = keys;
        self.password.0 = Some(Zeroizing::new(new_password.to_vec()));
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, KeychainError> {
        Ok(bincode::serialize(&self.keys)?)
    }

    pub fn contains(&self, fingerprint: u32) -> bool {
        self.keys.contains_key(&fingerprint)
    }

    pub fn remove(&mut self, fingerprint: u32) -> bool {
        self.keys.remove(&fingerprint).is_some()
    }

    pub fn fingerprints(&self) -> impl Iterator<Item = u32> + '_ {
        self.keys.keys().copied()
    }

    pub fn extract_public_key(&self, fingerprint: u32) -> Result<Option<PublicKey>, KeychainError> {
        match self.keys.get(&fingerprint) {
            Some(KeyData::Public { master_pk } | KeyData::Secret { master_pk, .. }) => {
                Ok(Some(PublicKey::from_bytes(master_pk)?))
            }
            None => Ok(None),
        }
    }

    pub fn extract_secrets(
        &self,
        fingerprint: u32,
        password: &[u8],
    ) -> Result<(Option<Mnemonic>, Option<SecretKey>), KeychainError> {
        match self.keys.get(&fingerprint) {
            Some(KeyData::Public { .. }) | None => Ok((None, None)),
            Some(KeyData::Secret {
                entropy, encrypted, ..
            }) => {
                let data = decrypt::<SecretKeyData>(encrypted, password)?;

                let mnemonic = if *entropy {
                    Some(Mnemonic::from_entropy(&data.0)?)
                } else {
                    None
                };

                let secret_key = if let Some(mnemonic) = mnemonic.as_ref() {
                    SecretKey::from_seed(&mnemonic.to_seed(""))
                } else {
                    SecretKey::from_bytes(&data.0.try_into().expect("invalid length"))?
                };

                Ok((mnemonic, Some(secret_key)))
            }
        }
    }

    pub fn has_secret_key(&self, fingerprint: u32) -> bool {
        let Some(key_data) = self.keys.get(&fingerprint) else {
            return false;
        };

        match key_data {
            KeyData::Public { .. } => false,
            KeyData::Secret { .. } => true,
        }
    }

    pub fn add_public_key(&mut self, master_pk: &PublicKey) -> Result<u32, KeychainError> {
        let fingerprint = master_pk.get_fingerprint();

        if self.contains(fingerprint) {
            return Err(KeychainError::KeyExists);
        }

        self.keys.insert(
            fingerprint,
            KeyData::Public {
                master_pk: master_pk.to_bytes(),
            },
        );

        Ok(fingerprint)
    }

    pub fn add_secret_key(
        &mut self,
        master_sk: &SecretKey,
        password: &[u8],
    ) -> Result<u32, KeychainError> {
        let master_pk = master_sk.public_key();
        let fingerprint = master_pk.get_fingerprint();

        if self.contains(fingerprint) {
            return Err(KeychainError::KeyExists);
        }

        let encrypted = encrypt(
            password,
            &mut self.rng,
            &SecretKeyData(master_sk.to_bytes().to_vec()),
        )?;

        self.keys.insert(
            fingerprint,
            KeyData::Secret {
                master_pk: master_pk.to_bytes(),
                entropy: false,
                encrypted,
            },
        );

        Ok(fingerprint)
    }

    pub fn add_mnemonic(
        &mut self,
        mnemonic: &Mnemonic,
        password: &[u8],
    ) -> Result<u32, KeychainError> {
        let entropy = mnemonic.to_entropy();
        let seed = mnemonic.to_seed("");
        let master_sk = SecretKey::from_seed(&seed);
        let master_pk = master_sk.public_key();
        let fingerprint = master_pk.get_fingerprint();

        if self.contains(fingerprint) {
            return Err(KeychainError::KeyExists);
        }

        let encrypted = encrypt(password, &mut self.rng, &SecretKeyData(entropy))?;

        self.keys.insert(
            fingerprint,
            KeyData::Secret {
                master_pk: master_pk.to_bytes(),
                entropy: true,
                encrypted,
            },
        );

        Ok(fingerprint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mnemonic() -> Mnemonic {
        Mnemonic::from_entropy(&[7; 32]).unwrap()
    }

    #[test]
    fn starts_unlocked_with_the_empty_password() {
        let mut keychain = Keychain::default();
        let fingerprint = keychain
            .add_mnemonic(&mnemonic(), &keychain.password().unwrap())
            .unwrap();
        let (_, secret_key) = keychain.extract_secrets(fingerprint, b"").unwrap();
        assert!(secret_key.is_some());
    }

    #[test]
    fn change_password_reencrypts_and_survives_a_reload() {
        let mut keychain = Keychain::default();
        let fingerprint = keychain.add_mnemonic(&mnemonic(), b"").unwrap();
        keychain.change_password(b"wallet key").unwrap();

        assert!(matches!(
            keychain.extract_secrets(fingerprint, b""),
            Err(KeychainError::Decrypt)
        ));
        let (restored, _) = keychain
            .extract_secrets(fingerprint, b"wallet key")
            .unwrap();
        assert_eq!(restored.unwrap(), mnemonic());

        let mut reloaded = Keychain::from_bytes(&keychain.to_bytes().unwrap()).unwrap();
        assert!(matches!(reloaded.unlock(b""), Err(KeychainError::Decrypt)));
        reloaded.unlock(b"wallet key").unwrap();
        assert_eq!(&*reloaded.password().unwrap(), b"wallet key");
    }

    #[test]
    fn a_reloaded_keychain_is_open_only_under_the_empty_password() {
        let mut open = Keychain::default();
        open.add_mnemonic(&mnemonic(), b"").unwrap();
        assert!(
            !Keychain::from_bytes(&open.to_bytes().unwrap())
                .unwrap()
                .is_locked()
        );

        open.change_password(b"wallet key").unwrap();
        let sealed = Keychain::from_bytes(&open.to_bytes().unwrap()).unwrap();
        assert!(sealed.is_locked(), "a passkey keychain starts locked");
    }

    #[test]
    fn locked_keychain_has_no_password() {
        let mut keychain = Keychain::default();
        keychain.add_mnemonic(&mnemonic(), b"").unwrap();
        keychain.lock();

        assert!(keychain.is_locked());
        assert!(matches!(keychain.password(), Err(KeychainError::Locked)));
        assert!(matches!(
            keychain.change_password(b"x"),
            Err(KeychainError::Locked)
        ));
        assert!(!format!("{keychain:?}").contains("wallet key"));
    }
}
