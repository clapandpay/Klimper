// SPDX-License-Identifier: AGPL-3.0-or-later
//! The bridge to Klimper's own key storage (clapandpay TASK-078) and to the facts
//! wallet-kit needs about the wallet (TASK-079).
//!
//! Ownership and the signable key set come from the **Sage database**, never from the
//! request: a dApp cannot make the wallet believe it owns a puzzle hash, and
//! `policy::plan` refuses to sign for a key that is not in [`WalletSnapshot::keys`]
//! (spec 11.1 item 2).
//!
//! ## Per-signature approval on desktop
//!
//! Spec 11.1 item 4 asks for biometrics on every signature. On iOS that is the passkey
//! assertion in `tauri-plugin-klimper-passkey`, whose Swift side only exists for iOS 18+
//! (`Passkeys::supported()` is `false` everywhere else), so on the desktop target the gate
//! is the two things desktop actually has:
//!
//! 1. **The keychain must be open.** `keys.bin` is opened by [`sage_keychain::Keychain`];
//!    with a passkey enrolled, `keychain.password()` fails with `Locked` until the passkey
//!    has unlocked it, and the signer then cannot produce anything.
//! 2. **One explicit confirmation per signature**, through [`crate::approval`], in
//!    addition to the approval of the request as a whole. Declining returns
//!    [`SignerError::Cancelled`] and nothing is signed.
//!
//! Secret key material is fetched inside the smallest possible scope — per signature, not
//! per request — and dropped immediately. The keychain password is a
//! `Zeroizing<Vec<u8>>`; `chia_bls::SecretKey` has neither `Drop` nor `Zeroize` in
//! chia-bls 0.36, so its bytes are not scrubbed. That is a chia-bls limitation, recorded
//! as a follow-up rather than papered over (the workspace denies `unsafe_code`, so it
//! cannot be scrubbed from here).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chia_bls::{PublicKey, SecretKey, Signature};
use sage::Sage;
use sage_database::Derivation;
use tokio::sync::Mutex;
use xchonnect_wallet_kit::{Network, Ownership, Signer, SignerError};

use crate::error::{Error, Result};

/// What `src-tauri` manages, and what `tauri-plugin-klimper-passkey` locks.
pub type AppState = Arc<Mutex<Sage>>;

/// How many derivations are read per database page.
const PAGE: u32 = 500;

/// Receive keys exposed to a paired dApp at most. Sage derives in batches of 500, so this
/// covers a wallet's first two batches; the `session.permissions` message stays ~100 KB,
/// well under the relay's 400 KB body limit.
pub const MAX_EXPOSED_KEYS: usize = 1000;

/// Everything about the active wallet that one request needs. Ownership is read fresh per
/// request; the key and derivation set is cached and re-read whenever the wallet, the
/// network or the highest derivation index changes, so a wallet switch or a new
/// derivation is still picked up on the very next request.
#[derive(Debug, Clone)]
pub struct WalletSnapshot {
    /// Active wallet fingerprint. Sessions are scoped to it (TASK-073 AC #3).
    pub fingerprint: u32,
    /// The wallet's network, with its `AGG_SIG` additional data.
    pub network: Network,
    /// Puzzle hashes the wallet controls.
    pub ownership: Ownership,
    /// Synthetic public keys the wallet can sign for.
    pub keys: HashSet<PublicKey>,
    /// Derivation of each key, so the signer never has to query the database again.
    pub derivations: HashMap<PublicKey, Derivation>,
    /// The wallet's receive keys (unhardened synthetic keys) in derivation order, at most
    /// [`MAX_EXPOSED_KEYS`]. Exposed to a paired dApp so it can read the wallet's balances
    /// from the chain itself instead of asking the wallet for each one.
    pub receive_keys: Vec<PublicKey>,
}

/// Map Sage's configured network onto wallet-kit's. The chain id is the network's own
/// `network_id` (`mainnet`, `testnet11`), which is what CHIP-0002 `chainId` returns and
/// what the session is approved for.
pub fn network_of(network: &sage_config::Network) -> Network {
    match network.name.as_str() {
        "mainnet" => Network::Mainnet,
        "testnet11" => Network::Testnet11,
        _ => Network::Custom {
            chain_id: network.network_id(),
            genesis_challenge: network.genesis_challenge,
            agg_sig_me: network.agg_sig_me(),
        },
    }
}

/// The part of a snapshot that only changes when the wallet derives new keys.
#[derive(Debug)]
struct DerivationSet {
    keys: HashSet<PublicKey>,
    derivations: HashMap<PublicKey, Derivation>,
    receive_keys: Vec<PublicKey>,
}

/// What a cached [`DerivationSet`] is valid for. Derivations are append-only, so the same
/// wallet on the same network with the same highest index has the same set; a wallet
/// switch, a network switch or a new derivation changes the key and the set is re-read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DerivationKey {
    fingerprint: u32,
    chain_id: String,
    max_unhardened: Option<u32>,
    max_hardened: Option<u32>,
}

/// Reading every derivation (two pages of 500 or more, joined) was most of the cost of
/// answering a request; the ownership set is still read fresh every time.
static DERIVATIONS: std::sync::Mutex<Option<(DerivationKey, Arc<DerivationSet>)>> =
    std::sync::Mutex::new(None);

async fn read_derivations(db: &sage_database::Database) -> Result<DerivationSet> {
    let mut keys = HashSet::new();
    let mut derivations = HashMap::new();
    let mut receive: Vec<(u32, PublicKey)> = Vec::new();
    for hardened in [false, true] {
        let mut offset = 0;
        loop {
            let (rows, total) = db.derivations(hardened, PAGE, offset).await?;
            if rows.is_empty() {
                break;
            }
            offset += u32::try_from(rows.len()).unwrap_or(PAGE);
            for row in rows {
                keys.insert(row.synthetic_key);
                derivations.insert(
                    row.synthetic_key,
                    Derivation {
                        derivation_index: row.index,
                        is_hardened: row.hardened,
                    },
                );
                if !hardened {
                    receive.push((row.index, row.synthetic_key));
                }
            }
            if offset >= total {
                break;
            }
        }
    }
    receive.sort_by_key(|(index, _)| *index);
    receive.truncate(MAX_EXPOSED_KEYS);
    Ok(DerivationSet {
        keys,
        derivations,
        receive_keys: receive.into_iter().map(|(_, key)| key).collect(),
    })
}

/// Read the active wallet. Fails with [`Error::NoWallet`] when no wallet is open.
pub async fn snapshot(state: &AppState) -> Result<WalletSnapshot> {
    // Take what is needed under the lock and let go of it: the database queries below
    // would otherwise hold up every other Sage command.
    let (wallet, network) = {
        let sage = state.lock().await;
        let wallet = sage.wallet().map_err(|_| Error::NoWallet)?;
        (wallet, network_of(sage.network()))
    };

    // Ownership is read fresh: it also holds non-derivation puzzles that can change on
    // their own.
    let p2_puzzle_hashes = wallet.db.custody_p2_puzzle_hashes().await?;
    let key = DerivationKey {
        fingerprint: wallet.fingerprint,
        chain_id: network.chain_id().to_owned(),
        max_unhardened: wallet.db.max_derivation_index(false).await?,
        max_hardened: wallet.db.max_derivation_index(true).await?,
    };
    let cached = DERIVATIONS.lock().ok().and_then(|guard| {
        guard
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, set)| set.clone())
    });
    let set = if let Some(set) = cached {
        set
    } else {
        let set = Arc::new(read_derivations(&wallet.db).await?);
        if let Ok(mut guard) = DERIVATIONS.lock() {
            *guard = Some((key, set.clone()));
        }
        set
    };

    Ok(WalletSnapshot {
        fingerprint: wallet.fingerprint,
        network,
        ownership: Ownership {
            p2_puzzle_hashes: p2_puzzle_hashes.into_iter().collect(),
        },
        keys: set.keys.clone(),
        derivations: set.derivations.clone(),
        receive_keys: set.receive_keys.clone(),
    })
}

/// Asked once per planned signature, with that signature's 1-based position; `false`
/// means the user declined and nothing is signed.
pub type SignatureGate = Arc<dyn Fn(usize, &PublicKey) -> bool + Send + Sync>;

/// Signs through Klimper's key storage.
///
/// Lives for exactly one request. `sign` runs on the thread that drives
/// `xchonnect_wallet_kit::handle`, which is a plain OS thread and not a runtime worker, so
/// it may block on the Sage mutex.
pub struct KlimperSigner {
    state: AppState,
    fingerprint: u32,
    derivations: HashMap<PublicKey, Derivation>,
    gate: SignatureGate,
    done: AtomicUsize,
}

impl std::fmt::Debug for KlimperSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KlimperSigner")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

impl KlimperSigner {
    pub fn new(state: AppState, snapshot: &WalletSnapshot, gate: SignatureGate) -> Self {
        KlimperSigner {
            state,
            fingerprint: snapshot.fingerprint,
            derivations: snapshot.derivations.clone(),
            gate,
            done: AtomicUsize::new(0),
        }
    }

    /// The number of signatures actually produced, for the caller's log line.
    pub fn produced(&self) -> usize {
        self.done.load(Ordering::SeqCst)
    }
}

impl Signer for KlimperSigner {
    fn sign(
        &self,
        public_key: &PublicKey,
        message: &[u8],
    ) -> std::result::Result<Signature, SignerError> {
        let Some(derivation) = self.derivations.get(public_key).copied() else {
            // Not one of this wallet's keys. `policy::plan` should already have refused,
            // so this is the belt to that braces.
            return Err(SignerError::KeyUnavailable);
        };
        let index = self.done.load(Ordering::SeqCst) + 1;
        if !(self.gate)(index, public_key) {
            return Err(SignerError::Cancelled);
        }

        // Smallest possible scope for key material: one signature.
        let signature = {
            let state = self.state.clone();
            let fingerprint = self.fingerprint;
            let result = tauri::async_runtime::block_on(async move {
                let sage = state.lock().await;
                // With a passkey enrolled this fails with `Locked` until the passkey has
                // opened the keychain.
                let password = sage.keychain.password()?;
                let (_mnemonic, Some(master)) =
                    sage.keychain.extract_secrets(fingerprint, &password)?
                else {
                    return Err(Error::WalletLocked);
                };
                Ok(derive(&master, derivation))
            });
            match result {
                Ok(secret) => {
                    let signature = chia_bls::sign(&secret, message);
                    drop(secret);
                    signature
                }
                Err(error) => {
                    tracing::warn!("xchonnect signature refused: {error}");
                    return Err(SignerError::KeyUnavailable);
                }
            }
        };

        // Verify before handing it back. `handle()` checks this too; doing it here means a
        // wrong signature never leaves this module, whatever the caller does with it
        // (xchonnect "wallet-kit: verify every signature returned by the host Signer").
        if !chia_bls::verify(&signature, public_key, message) {
            tracing::error!("a signature produced for an xchonnect request did not verify");
            return Err(SignerError::KeyUnavailable);
        }
        self.done.fetch_add(1, Ordering::SeqCst);
        Ok(signature)
    }
}

/// Sage's derivation: unhardened or hardened wallet path, then the synthetic offset.
fn derive(master: &SecretKey, derivation: Derivation) -> SecretKey {
    use chia_bls::{master_to_wallet_hardened, master_to_wallet_unhardened};
    use chia_puzzle_types::DeriveSynthetic;

    if derivation.is_hardened {
        master_to_wallet_hardened(master, derivation.derivation_index)
    } else {
        master_to_wallet_unhardened(master, derivation.derivation_index)
    }
    .derive_synthetic()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_network_name_becomes_the_chain_id() {
        let list = sage_config::NetworkList::default();
        let mainnet = network_of(list.by_name("mainnet").unwrap());
        let testnet = network_of(list.by_name("testnet11").unwrap());
        assert_eq!(mainnet.chain_id(), "mainnet");
        assert_eq!(testnet.chain_id(), "testnet11");
        assert_ne!(mainnet.genesis_challenge(), testnet.genesis_challenge());
    }

    #[test]
    fn derivation_matches_sages_own_signing_path() {
        // The same two lines `crates/sage/src/endpoints/wallet_connect.rs` uses.
        let master = SecretKey::from_seed(&[3; 64]);
        let ours = derive(
            &master,
            Derivation {
                derivation_index: 7,
                is_hardened: false,
            },
        );
        let theirs = {
            use chia_puzzle_types::DeriveSynthetic;
            chia_bls::master_to_wallet_unhardened(&master, 7).derive_synthetic()
        };
        assert_eq!(ours.public_key(), theirs.public_key());
    }
}
