// SPDX-License-Identifier: AGPL-3.0-or-later
//! The wallet's record of its coins with signatures outside it (xchonnect spec 11.2,
//! TASK-140).
//!
//! wallet-kit treats a request Klimper builds, signs and broadcasts itself as **sealed**:
//! the user's signatures exist only inside the one bundle that goes to the network, so the
//! user's spends reach the chain together or not at all, and the links Sage's builders make
//! between them (`ASSERT_CONCURRENT_SPEND`) count. That holds only while no other signature
//! over one of those coins is out there, so the kit takes two sets from the host with every
//! request ([`SignedCoins`]):
//!
//! * **released**: every unspent coin of the wallet for which a signature has left it — a
//!   `signCoinSpends` answered to a website, a loan request handed out as an offer, any
//!   bundle broadcast (also one the network refused). A request spending one of these is
//!   never sealed: only direct assertions bind it.
//! * **in flight**: the coins of every sealed bundle Klimper broadcast, until any coin of
//!   that bundle is spent on chain (from then on it can never be included). The kit refuses
//!   every request spending one (4001 `coin_in_flight`).
//! * **sealed messages**: per coin of the wallet's in a sealed bundle, the exact
//!   `AGG_SIG_ME` messages Klimper signed for it, kept until that coin itself is spent on
//!   chain (never cleared because another coin of the bundle was spent). wallet-kit at
//!   ba01161 does not read them yet; its follow-up refuses to sign a recorded message again.
//!
//! The record lives next to the sessions in the OS keyring, per wallet and network
//! ([`crate::store::SessionStore::load_signed_coins`]), and survives restarts. Coins leave
//! it once they are seen spent on chain: before every request that signs coin spends, the
//! record is checked against Sage's wallet database and, for coins the database does not
//! hold, against a peer ([`refresh`]).
//!
//! Requests that sign coin spends run one at a time ([`gate`]): the record a request is
//! checked against must still be the record when its signature leaves the wallet.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chia_bls::PublicKey;
use chia_protocol::{Bytes32, CoinSpend, SpendBundle};
use serde::{Deserialize, Serialize};
use xchonnect_wallet_kit::{self as kit, Broadcast, Broadcaster, Ownership, SignedCoins};

use crate::error::{Error, Result};
use crate::store::SessionStore;
use crate::wallet::AppState;

/// What is stored: coin ids and bundle ids as lowercase hex.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedCoinsRecord {
    /// Coins of the wallet with a signature outside it.
    #[serde(default)]
    released: BTreeSet<String>,
    /// Sealed bundles broadcast and not yet settled: bundle id to the coins it spends.
    #[serde(default)]
    in_flight: BTreeMap<String, BTreeSet<String>>,
    /// Per coin of the wallet's spent by a sealed bundle Klimper broadcast: every
    /// `AGG_SIG_ME` signature it made for that spend, as `<public key>:<signed message>`
    /// (the full message the key signed, coin id and network data appended). Kept until
    /// **that** coin is seen spent on chain, whatever happens to the bundle's other coins:
    /// a signature over the same message, made again for another request, would let a
    /// bundle that can no longer be included as a whole be split.
    #[serde(default)]
    sealed_messages: BTreeMap<String, BTreeSet<String>>,
}

/// One signature Klimper made inside a sealed bundle: the coin, the key, the full message.
pub type SealedMessage = (Bytes32, PublicKey, Vec<u8>);

fn parse(hex_id: &str) -> Option<Bytes32> {
    let bytes = hex::decode(hex_id).ok()?;
    Bytes32::try_from(bytes.as_slice()).ok()
}

impl SignedCoinsRecord {
    /// The two sets as wallet-kit takes them.
    pub fn to_kit(&self) -> SignedCoins {
        SignedCoins {
            released: self.released.iter().filter_map(|c| parse(c)).collect(),
            in_flight: self
                .in_flight
                .values()
                .flatten()
                .filter_map(|c| parse(c))
                .collect(),
        }
    }

    /// Signatures over these coins left the wallet.
    pub fn release(&mut self, coins: impl IntoIterator<Item = Bytes32>) {
        self.released.extend(coins.into_iter().map(hex::encode));
    }

    /// A sealed bundle is being broadcast: every coin it spends is in flight until one of them
    /// is spent on chain, and the user's coins in it (`user_coins`) are released. (Spec 11.2
    /// says every coin of the bundle goes in both sets; wallet-kit reads `released` only for
    /// the user's own spends, and another party's coin, or a coin the bundle would create and
    /// spend, might never be seen spent and would stay in the record for good.)
    pub fn seal(
        &mut self,
        bundle: &SpendBundle,
        user_coins: Vec<Bytes32>,
        messages: Vec<SealedMessage>,
    ) {
        for (coin, key, message) in messages {
            self.sealed_messages
                .entry(hex::encode(coin))
                .or_default()
                .insert(format!(
                    "{}:{}",
                    hex::encode(key.to_bytes()),
                    hex::encode(message)
                ));
        }
        self.in_flight
            .entry(hex::encode(bundle.name()))
            .or_default()
            .extend(
                bundle
                    .coin_spends
                    .iter()
                    .map(|cs| hex::encode(cs.coin.coin_id())),
            );
        self.release(user_coins);
    }

    /// Every coin whose state on chain decides what stays in the record.
    pub fn coins(&self) -> Vec<Bytes32> {
        let mut all: BTreeSet<&String> = self.released.iter().collect();
        all.extend(self.in_flight.values().flatten());
        all.extend(self.sealed_messages.keys());
        all.into_iter().filter_map(|c| parse(c)).collect()
    }

    /// Forget what `spent` settles: a released coin spent on chain, and every bundle in
    /// flight one of whose coins is spent on chain.
    pub fn prune(&mut self, spent: &HashSet<Bytes32>) {
        let spent: HashSet<String> = spent.iter().map(hex::encode).collect();
        self.released.retain(|c| !spent.contains(c));
        self.in_flight
            .retain(|_, coins| !coins.iter().any(|c| spent.contains(c)));
        self.sealed_messages.retain(|coin, _| !spent.contains(coin));
    }

    /// Every signature recorded for a sealed bundle, per coin ([`SealedMessage`]).
    pub fn sealed_messages(&self) -> Vec<SealedMessage> {
        let mut out = Vec::new();
        for (coin, entries) in &self.sealed_messages {
            let Some(coin) = parse(coin) else { continue };
            for entry in entries {
                let Some((key, message)) = entry.split_once(':') else {
                    continue;
                };
                let key = hex::decode(key)
                    .ok()
                    .and_then(|bytes| <[u8; 48]>::try_from(bytes.as_slice()).ok())
                    .and_then(|bytes| PublicKey::from_bytes(&bytes).ok());
                if let (Some(key), Ok(message)) = (key, hex::decode(message)) {
                    out.push((coin, key, message));
                }
            }
        }
        out
    }

    /// Nothing recorded.
    pub fn is_empty(&self) -> bool {
        self.released.is_empty() && self.in_flight.is_empty() && self.sealed_messages.is_empty()
    }
}

/// The record of one wallet on one network, in the session store.
#[derive(Debug, Clone)]
pub struct SignedCoinsHandle {
    store: Arc<SessionStore>,
    fingerprint: u32,
    chain_id: String,
}

impl SignedCoinsHandle {
    pub fn new(store: Arc<SessionStore>, fingerprint: u32, chain_id: &str) -> Self {
        SignedCoinsHandle {
            store,
            fingerprint,
            chain_id: chain_id.to_owned(),
        }
    }

    pub fn load(&self) -> Result<SignedCoinsRecord> {
        self.store
            .load_signed_coins(self.fingerprint, &self.chain_id)
    }

    fn update(&self, change: impl FnOnce(&mut SignedCoinsRecord)) -> Result<SignedCoinsRecord> {
        self.store
            .update_signed_coins(self.fingerprint, &self.chain_id, change)
    }

    pub fn release(&self, coins: Vec<Bytes32>) -> Result<()> {
        if coins.is_empty() {
            return Ok(());
        }
        self.update(|record| record.release(coins)).map(drop)
    }

    pub fn seal(
        &self,
        bundle: &SpendBundle,
        user_coins: Vec<Bytes32>,
        messages: Vec<SealedMessage>,
    ) -> Result<()> {
        self.update(|record| record.seal(bundle, user_coins, messages))
            .map(drop)
    }

    pub fn prune(&self, spent: &HashSet<Bytes32>) -> Result<SignedCoinsRecord> {
        self.update(|record| record.prune(spent))
    }
}

/// The coins of `spends` that are the user's, as wallet-kit's simulation counts them (the
/// coins the wallet signs for). Every coin of `spends` when the simulation fails: more
/// coins in the record only makes later requests stricter.
pub fn user_coins(spends: &[CoinSpend], ownership: &Ownership) -> Vec<Bytes32> {
    let all = || spends.iter().map(|cs| cs.coin.coin_id()).collect();
    let Ok(summary) = kit::simulate(spends, ownership, kit::simulate::DEFAULT_MAX_COST) else {
        return all();
    };
    let owned: Vec<Bytes32> = summary
        .spends
        .iter()
        .filter(|s| s.owned)
        .filter_map(|s| parse(&s.coin_id))
        .collect();
    if owned.len() == summary.spends.iter().filter(|s| s.owned).count() {
        owned
    } else {
        all()
    }
}

/// Every `AGG_SIG_ME` signature one of `keys` makes for `spends`: the coin, the key and the
/// full message signed (as the wallet's signer receives it).
#[allow(clippy::implicit_hasher)]
pub fn me_messages(
    spends: &[CoinSpend],
    keys: &HashSet<PublicKey>,
    network: &kit::Network,
) -> Vec<SealedMessage> {
    use chia_wallet_sdk::prelude::{Allocator, Condition, FromClvm, ToClvm};
    use chia_wallet_sdk::signer::RequiredBlsSignature;

    let constants = crate::execute::agg_sig_constants(network);
    let mut out = Vec::new();
    for cs in spends {
        let mut allocator = Allocator::new();
        let (Ok(puzzle), Ok(solution)) = (
            cs.puzzle_reveal.to_clvm(&mut allocator),
            cs.solution.to_clvm(&mut allocator),
        ) else {
            continue;
        };
        let Ok(output) = chia_wallet_sdk::types::run_puzzle(&mut allocator, puzzle, solution)
        else {
            continue;
        };
        let Ok(conditions) = Vec::<Condition>::from_clvm(&allocator, output) else {
            continue;
        };
        for condition in conditions {
            if !matches!(condition, Condition::AggSigMe(_)) {
                continue;
            }
            let Some(sig) = condition.into_agg_sig() else {
                continue;
            };
            if keys.contains(&sig.public_key) {
                let key = sig.public_key;
                let message =
                    RequiredBlsSignature::from_condition(&cs.coin, sig, &constants).message();
                out.push((cs.coin.coin_id(), key, message));
            }
        }
    }
    out
}

/// A broadcaster that writes the bundle's coins into the record **before** the bundle goes
/// out (a crash in between leaves the record stricter, never looser), and refuses to send
/// if the record cannot be written.
pub struct Recording<'a> {
    inner: &'a dyn Broadcaster,
    record: &'a SignedCoinsHandle,
    ownership: &'a Ownership,
    /// For a sealed bundle: the keys the wallet signs with and the network, to record the
    /// messages it signed. `None`: the bundle's signatures are released.
    sealed: Option<(&'a HashSet<PublicKey>, &'a kit::Network)>,
}

impl std::fmt::Debug for Recording<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recording")
            .field("sealed", &self.sealed.is_some())
            .finish_non_exhaustive()
    }
}

impl<'a> Recording<'a> {
    /// For bundles Klimper built and broadcasts itself, of which only the transaction id
    /// and status go back to the website (`signatures_released: false`): the bundle's coins
    /// go in flight and are released.
    pub fn sealed(
        inner: &'a dyn Broadcaster,
        record: &'a SignedCoinsHandle,
        ownership: &'a Ownership,
        keys: &'a HashSet<PublicKey>,
        network: &'a kit::Network,
    ) -> Self {
        Recording {
            inner,
            record,
            ownership,
            sealed: Some((keys, network)),
        }
    }

    /// For any other bundle (a website's own): the user's coins in it are released.
    pub fn released(
        inner: &'a dyn Broadcaster,
        record: &'a SignedCoinsHandle,
        ownership: &'a Ownership,
    ) -> Self {
        Recording {
            inner,
            record,
            ownership,
            sealed: None,
        }
    }
}

impl Broadcaster for Recording<'_> {
    fn submit(&self, bundle: &SpendBundle) -> std::result::Result<Broadcast, String> {
        let recorded = if let Some((keys, network)) = self.sealed {
            self.record.seal(
                bundle,
                user_coins(&bundle.coin_spends, self.ownership),
                me_messages(&bundle.coin_spends, keys, network),
            )
        } else {
            self.record
                .release(user_coins(&bundle.coin_spends, self.ownership))
        };
        if let Err(error) = recorded {
            tracing::error!("xchonnect: the record of signed coins could not be written: {error}");
            return Err("the wallet could not record this transaction".to_owned());
        }
        self.inner.submit(bundle)
    }
}

/// Requests that sign coin spends run one at a time, from reading the record to writing it.
static GATE: Mutex<()> = Mutex::new(());

/// Wait for the other requests that sign coin spends to finish.
pub fn gate() -> MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Which of `coins` are spent on chain: from the wallet's database, and for coins it does
/// not hold (another party's, or created and spent in one bundle) from `peer`. A coin
/// neither can tell about counts as unspent: it stays in the record.
pub async fn spent_coins(
    wallet: &sage_wallet::Wallet,
    peer: Option<&sage_wallet::WalletPeer>,
    coins: &[Bytes32],
) -> HashSet<Bytes32> {
    let mut spent = HashSet::new();
    if coins.is_empty() {
        return spent;
    }
    let ids: Vec<String> = coins.iter().map(hex::encode).collect();
    let mut known = HashSet::new();
    match wallet.db.coins_by_ids(&ids).await {
        Ok(rows) => {
            for row in rows {
                let id = row.coin.coin_id();
                known.insert(id);
                if row.spent_height.is_some() {
                    spent.insert(id);
                }
            }
        }
        Err(error) => tracing::warn!("xchonnect: could not read coin states: {error}"),
    }
    let unknown: Vec<Bytes32> = coins
        .iter()
        .filter(|c| !known.contains(*c))
        .copied()
        .collect();
    if let (false, Some(peer)) = (unknown.is_empty(), peer) {
        match peer.fetch_coins(unknown, wallet.genesis_challenge).await {
            Ok(states) => spent.extend(
                states
                    .into_iter()
                    .filter(|state| state.spent_height.is_some())
                    .map(|state| state.coin.coin_id()),
            ),
            Err(error) => {
                tracing::info!("xchonnect: could not ask a peer for coin states: {error}");
            }
        }
    }
    spent
}

/// The record, with every coin seen spent on chain taken out. Run on the request thread
/// (see `requests::answer`).
pub fn refresh(state: &AppState, handle: &SignedCoinsHandle) -> Result<SignedCoinsRecord> {
    let record = handle.load()?;
    let coins = record.coins();
    if coins.is_empty() {
        return Ok(record);
    }
    let state = state.clone();
    let spent = tauri::async_runtime::block_on(async move {
        let (wallet, peer) = {
            let sage = state.lock().await;
            let wallet = sage.wallet().map_err(Error::internal)?;
            let peer = sage.peer_state.lock().await.acquire_peer();
            (wallet, peer)
        };
        Ok::<_, Error>(spent_coins(&wallet, peer.as_ref(), &coins).await)
    })?;
    if spent.is_empty() {
        return Ok(record);
    }
    handle.prune(&spent)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::store::{MemorySecrets, SecretStore};
    use chia_bls::Signature;
    use chia_protocol::{Coin, Program};

    fn coin(byte: u8) -> Coin {
        Coin::new(Bytes32::new([byte; 32]), Bytes32::new([0x42; 32]), 1)
    }

    fn bundle(coins: &[Coin]) -> SpendBundle {
        SpendBundle::new(
            coins
                .iter()
                .map(|c| CoinSpend::new(*c, Program::from(vec![0x80]), Program::from(vec![0x80])))
                .collect(),
            Signature::default(),
        )
    }

    #[test]
    fn a_sealed_bundle_is_in_flight_and_released_until_one_of_its_coins_is_spent() {
        let (a, b, c) = (coin(1), coin(2), coin(3));
        let mut record = SignedCoinsRecord::default();
        record.release([c.coin_id()]);
        // `b` is someone else's coin: in flight, not released.
        let key = chia_bls::SecretKey::from_seed(&[9; 32]).public_key();
        record.seal(
            &bundle(&[a, b]),
            vec![a.coin_id()],
            vec![(a.coin_id(), key, b"signed".to_vec())],
        );
        assert_eq!(
            record.sealed_messages(),
            [(a.coin_id(), key, b"signed".to_vec())]
        );
        let kit = record.to_kit();
        assert_eq!(
            kit.in_flight,
            [a.coin_id(), b.coin_id()].into_iter().collect()
        );
        assert_eq!(
            kit.released,
            [a.coin_id(), c.coin_id()].into_iter().collect()
        );

        // One coin of the bundle spent (the other party's): the bundle can never be included;
        // the user's coin is still released (its signature is out there) until it is spent.
        record.prune(&[b.coin_id()].into_iter().collect());
        let kit = record.to_kit();
        assert!(kit.in_flight.is_empty());
        // What Klimper signed for `a` stays recorded until `a` itself is spent.
        assert_eq!(record.sealed_messages().len(), 1);
        assert_eq!(
            kit.released,
            [a.coin_id(), c.coin_id()].into_iter().collect()
        );

        record.prune(&[a.coin_id(), c.coin_id()].into_iter().collect());
        assert!(record.is_empty());
    }

    #[test]
    fn the_record_survives_a_reload_and_is_kept_per_wallet_and_network() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let open = || {
            Arc::new(
                SessionStore::open(dir.path(), Box::new(SharedSecrets(secrets.clone()))).unwrap(),
            )
        };
        let (a, b) = (coin(1), coin(2));
        let handle = SignedCoinsHandle::new(open(), 7, "testnet11");
        handle.release(vec![a.coin_id()]).unwrap();
        handle
            .seal(&bundle(&[b]), vec![b.coin_id()], vec![])
            .unwrap();

        // A new store over the same keyring: the app started again.
        let again = SignedCoinsHandle::new(open(), 7, "testnet11")
            .load()
            .unwrap();
        assert_eq!(again, handle.load().unwrap());
        assert!(again.to_kit().in_flight.contains(&b.coin_id()));
        assert!(again.to_kit().released.contains(&a.coin_id()));

        assert!(
            SignedCoinsHandle::new(open(), 7, "mainnet")
                .load()
                .unwrap()
                .is_empty()
        );
        assert!(
            SignedCoinsHandle::new(open(), 8, "testnet11")
                .load()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn an_unreadable_record_is_an_error_not_an_empty_record() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        secrets.put("signed:7:testnet11", b"not json").unwrap();
        let store = SessionStore::open(dir.path(), Box::new(SharedSecrets(secrets))).unwrap();
        assert!(store.load_signed_coins(7, "testnet11").is_err());
    }

    /// One `MemorySecrets` behind several stores, as the OS keyring is behind every start.
    #[derive(Debug)]
    struct SharedSecrets(Arc<MemorySecrets>);

    impl SecretStore for SharedSecrets {
        fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
            self.0.put(key, bytes)
        }
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
            self.0.get(key)
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.0.delete(key)
        }
    }
}
