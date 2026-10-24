// SPDX-License-Identifier: AGPL-3.0-or-later
//! The card's signer: one Ethereum key, derived from the wallet's own recovery words.
//!
//! A Gnosis Pay account is a Safe on Gnosis Chain whose owner is an Ethereum account, and
//! signing in is Sign-In with Ethereum (EIP-4361). So the card needs a secp256k1 key. It is
//! derived from the same 24 words as the Chia keys, on the standard Ethereum path
//! `m/44'/60'/0'/0/0` (BIP-32, BIP-44), so:
//!
//! * the recovery words the user already wrote down also recover the card account, and any
//!   Ethereum wallet given those words finds the same address (no lock-in to Klimper);
//! * nothing new is stored: the key is derived when it is needed and dropped right after.
//!
//! The Chia keys (BLS12-381) and this key (secp256k1) are different curves on different
//! chains: nothing on either chain links the Chia addresses to the card address. Klimper
//! never sends a Chia address to Gnosis Pay.
//!
//! A wallet imported from a bare secret key has no recovery words and therefore no card.

use bip39::Mnemonic;
use hmac::{Hmac, Mac};
use k256::ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::{NonZeroScalar, Scalar, SecretKey};
use sha2::Sha512;
use sha3::{Digest, Keccak256};
use zeroize::{Zeroize, Zeroizing};

/// The standard first Ethereum account: m/44'/60'/0'/0/0.
pub const PATH: [u32; 5] = [hardened(44), hardened(60), hardened(0), 0, 0];
const HARDENED: u32 = 0x8000_0000;

const fn hardened(index: u32) -> u32 {
    index | HARDENED
}

#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum KeyError {
    /// A derivation step hit the 1-in-2^127 invalid case. BIP-32 says to skip to the next
    /// index; on the fixed path a wallet never meets it, so we refuse instead.
    #[error("this wallet's words give no valid key on the Ethereum path")]
    InvalidDerivation,
    #[error("the signature could not be made")]
    Signing,
}

/// An Ethereum address, 20 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Address(pub [u8; 20]);

impl Address {
    /// EIP-55 mixed-case checksum form, `0x` prefixed.
    pub fn checksummed(&self) -> String {
        let lower = hex::encode(self.0);
        let hash = Keccak256::digest(lower.as_bytes());
        let mut out = String::with_capacity(42);
        out.push_str("0x");
        for (i, c) in lower.chars().enumerate() {
            let nibble = (hash[i / 2] >> (if i % 2 == 0 { 4 } else { 0 })) & 0x0f;
            if c.is_ascii_alphabetic() && nibble >= 8 {
                out.push(c.to_ascii_uppercase());
            } else {
                out.push(c);
            }
        }
        out
    }

    pub fn of(key: &VerifyingKey) -> Self {
        let point = key.to_encoded_point(false);
        let hash = Keccak256::digest(&point.as_bytes()[1..]);
        let mut address = [0u8; 20];
        address.copy_from_slice(&hash[12..]);
        Self(address)
    }

    /// Parses `0x` + 40 hex digits. A mixed-case input must carry a valid EIP-55 checksum.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix("0x")?;
        if digits.len() != 40 {
            return None;
        }
        let bytes: [u8; 20] = hex::decode(digits).ok()?.try_into().ok()?;
        let address = Self(bytes);
        let mixed = digits.chars().any(|c| c.is_ascii_uppercase())
            && digits.chars().any(|c| c.is_ascii_lowercase());
        if mixed && address.checksummed() != text {
            return None;
        }
        Some(address)
    }
}

/// The card signer. Holds the secret only for as long as the value lives.
pub struct EvmKey {
    signing: SigningKey,
}

impl std::fmt::Debug for EvmKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the secret; the address is enough to tell keys apart in a log.
        f.debug_struct("EvmKey")
            .field("address", &self.address().checksummed())
            .finish()
    }
}

impl EvmKey {
    /// Derives the key at [`PATH`] from the wallet's recovery words (empty BIP-39 passphrase,
    /// as Sage uses).
    pub fn from_mnemonic(mnemonic: &Mnemonic) -> Result<Self, KeyError> {
        let seed = Zeroizing::new(mnemonic.to_seed(""));
        Self::from_seed(&seed[..])
    }

    pub fn from_seed(seed: &[u8]) -> Result<Self, KeyError> {
        let (mut key, mut chain) = master(seed)?;
        for index in PATH {
            let (next_key, next_chain) = child(&key, &chain, index)?;
            key = next_key;
            chain.zeroize();
            chain = next_chain;
        }
        chain.zeroize();
        Ok(Self {
            signing: SigningKey::from(key),
        })
    }

    pub fn address(&self) -> Address {
        Address::of(self.signing.verifying_key())
    }

    /// EIP-191 `personal_sign`: what a wallet signs for a SIWE message. 65 bytes, r ‖ s ‖ v
    /// with v = 27 or 28.
    pub fn personal_sign(&self, message: &[u8]) -> Result<[u8; 65], KeyError> {
        self.sign_digest(&personal_digest(message))
    }

    /// Signs a 32-byte digest (an EIP-712 hash, for example) with a low-s, recoverable
    /// signature, r ‖ s ‖ v with v = 27 or 28.
    pub fn sign_digest(&self, digest: &[u8; 32]) -> Result<[u8; 65], KeyError> {
        let (signature, recovery) = self
            .signing
            .sign_prehash_recoverable(digest)
            .map_err(|_| KeyError::Signing)?;
        let mut out = [0u8; 65];
        out[..64].copy_from_slice(&signature.to_bytes());
        out[64] = 27 + recovery.to_byte();
        // Checked before it leaves: a signature that does not recover to our address is
        // never handed out.
        if recover(digest, &out) != Some(self.address()) {
            return Err(KeyError::Signing);
        }
        Ok(out)
    }
}

/// keccak256("\x19Ethereum Signed Message:\n" ‖ len ‖ message).
pub fn personal_digest(message: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(format!("\x19Ethereum Signed Message:\n{}", message.len()).as_bytes());
    hasher.update(message);
    hasher.finalize().into()
}

/// The address a 65-byte signature over `digest` came from.
pub fn recover(digest: &[u8; 32], signature: &[u8; 65]) -> Option<Address> {
    let parsed = Signature::from_slice(&signature[..64]).ok()?;
    let v = signature[64];
    let recovery = RecoveryId::from_byte(if v >= 27 { v - 27 } else { v })?;
    let key = VerifyingKey::recover_from_prehash(digest, &parsed, recovery).ok()?;
    Some(Address::of(&key))
}

type ChainCode = [u8; 32];

fn hmac512(key: &[u8], parts: &[&[u8]]) -> Zeroizing<[u8; 64]> {
    let mut mac = Hmac::<Sha512>::new_from_slice(key).expect("HMAC takes any key length");
    for part in parts {
        mac.update(part);
    }
    Zeroizing::new(mac.finalize().into_bytes().into())
}

fn split(i: &[u8; 64]) -> ([u8; 32], ChainCode) {
    let mut left = [0u8; 32];
    let mut right = [0u8; 32];
    left.copy_from_slice(&i[..32]);
    right.copy_from_slice(&i[32..]);
    (left, right)
}

fn master(seed: &[u8]) -> Result<(SecretKey, ChainCode), KeyError> {
    let i = hmac512(b"Bitcoin seed", &[seed]);
    let (mut left, chain) = split(&i);
    let key = SecretKey::from_slice(&left).map_err(|_| KeyError::InvalidDerivation);
    left.zeroize();
    Ok((key?, chain))
}

fn child(
    parent: &SecretKey,
    chain: &ChainCode,
    index: u32,
) -> Result<(SecretKey, ChainCode), KeyError> {
    let index_bytes = index.to_be_bytes();
    let i = if index & HARDENED != 0 {
        let secret = Zeroizing::new(parent.to_bytes());
        hmac512(chain, &[&[0u8], secret.as_slice(), &index_bytes])
    } else {
        let public = parent.public_key().to_encoded_point(true);
        hmac512(chain, &[public.as_bytes(), &index_bytes])
    };
    let (mut left, next_chain) = split(&i);
    let tweak: Option<NonZeroScalar> = NonZeroScalar::try_from(left.as_slice()).ok();
    left.zeroize();
    // IL ≥ n is rejected by the conversion above (and IL = 0 has the same probability).
    let tweak = tweak.ok_or(KeyError::InvalidDerivation)?;
    let sum: Scalar = *tweak + *parent.to_nonzero_scalar();
    let sum = Option::<NonZeroScalar>::from(NonZeroScalar::new(sum))
        .ok_or(KeyError::InvalidDerivation)?;
    Ok((SecretKey::from(sum), next_chain))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn key(words: &str) -> EvmKey {
        EvmKey::from_mnemonic(&Mnemonic::from_str(words).unwrap()).unwrap()
    }

    // Public test vectors: the first account of well-known development mnemonics, as every
    // Ethereum wallet derives it on m/44'/60'/0'/0/0.
    #[test]
    fn the_first_account_of_the_development_mnemonics() {
        assert_eq!(
            key("test test test test test test test test test test test junk")
                .address()
                .checksummed(),
            "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
        );
        assert_eq!(
            key("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about")
                .address()
                .checksummed(),
            "0x9858EfFD232B4033E47d90003D41EC34EcaEda94"
        );
    }

    #[test]
    fn eip55_checksums() {
        // From EIP-55 itself.
        for text in [
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
            "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
            "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB",
            "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb",
        ] {
            let address = Address::parse(text).unwrap();
            assert_eq!(address.checksummed(), text);
        }
        assert!(Address::parse("0x5aaeb6053F3E94C9b9A09f33669435E7Ef1BeAed").is_none());
        assert!(Address::parse("0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaed").is_some());
        assert!(Address::parse("5aaeb6053f3e94c9b9a09f33669435e7ef1beaed").is_none());
        assert!(Address::parse("0x5aaeb6").is_none());
    }

    #[test]
    fn a_personal_signature_recovers_to_the_signer() {
        let signer = key("test test test test test test test test test test test junk");
        let message = b"hello world";
        let signature = signer.personal_sign(message).unwrap();
        assert!(signature[64] == 27 || signature[64] == 28);
        assert_eq!(
            recover(&personal_digest(message), &signature),
            Some(signer.address())
        );
        // Deterministic (RFC 6979): the same message gives the same signature.
        assert_eq!(signer.personal_sign(message).unwrap(), signature);
        // Another message does not verify against this signature.
        assert_ne!(
            recover(&personal_digest(b"hello world!"), &signature),
            Some(signer.address())
        );
    }

    #[test]
    fn the_personal_message_prefix() {
        // keccak256("\x19Ethereum Signed Message:\n11hello world"), as ethers' hashMessage.
        assert_eq!(
            hex::encode(personal_digest(b"hello world")),
            "d9eba16ed0ecae432b71fe008c98cc872bb4cc214d3220a36f365326cf807d68"
        );
    }

    #[test]
    fn debug_never_prints_the_secret() {
        let signer = key("test test test test test test test test test test test junk");
        let text = format!("{signer:?}");
        assert!(text.contains("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"));
        // Hardhat account 0's well-known private key.
        assert!(!text.contains("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"));
    }
}
