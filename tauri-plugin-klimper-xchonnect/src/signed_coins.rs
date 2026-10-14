// SPDX-License-Identifier: AGPL-3.0-or-later
//! The wallet's record of the signatures it let leave it (xchonnect spec 11.2, TASK-140).
//!
//! wallet-kit treats a request Klimper builds, signs and broadcasts itself as **sealed**:
//! the user's signatures exist only inside the one bundle that goes to the network, so the
//! user's spends reach the chain together or not at all, and the links Sage's builders make
//! between them (`ASSERT_CONCURRENT_SPEND`) count. That holds only while the wallet never
//! makes one of those signatures again, so the kit takes a record from the host with every
//! request ([`SignedCoins`]):
//!
//! * **released**: every unspent coin for which a signature has left the wallet — a
//!   `signCoinSpends` answered to a website, a loan request handed out as an offer, any
//!   bundle broadcast (also one the network refused). A request spending one of the user's
//!   coins in it is never sealed: only direct assertions bind it.
//! * **sealed**: per coin of a sealed bundle Klimper broadcast (or tried to), the
//!   signatures it made for it, as `sha256(public key || message)`. The kit never signs one
//!   of them again, in any request (4001 `sealed_signature`); another spend of the coin signs
//!   another message and is allowed, so nothing is locked.
//!
//! The kit says what to add before every broadcast ([`Broadcaster::record`]); Klimper's
//! broadcaster ([`crate::execute::SageBroadcaster`]) writes it into the record before it
//! returns, and nothing is sent when that fails (4000 `record_failed`). The record lives
//! next to the sessions in the OS keyring, per wallet and network
//! ([`crate::store::SessionStore::load_signed_coins`]), and survives restarts. A coin leaves
//! it only once **that** coin is spent on chain ([`SignedCoins::spent`]), as the wallet's own
//! synced database says, [`CONFIRMATIONS`] blocks deep: checked before every request that
//! signs ([`refresh`]). A coin the database does not hold stays; no peer is asked.
//!
//! The kit never signs a sealed bundle's messages twice, so a sealed bundle is sent again
//! by broadcasting the bytes Klimper kept ([`SealedBundles`], [`resend`]).
//!
//! Requests that sign (coin spends or a message, and every wallet-built broadcast) run one
//! at a time ([`gate`], [`checked_record`]), from reading the record, across the approval
//! prompt, to recording what they signed: the record a request is checked against must still
//! be the record when its signature leaves the wallet.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chia_bls::PublicKey;
use chia_protocol::{Bytes32, CoinSpend, SpendBundle};
use chia_wallet_sdk::chia::traits::Streamable;
use serde::{Deserialize, Serialize};
use xchonnect_wallet_kit::{
    self as kit, Broadcast, BroadcastRecord, Broadcaster, Ownership, SignedCoins,
};

use crate::error::{Error, Result};
use crate::store::SessionStore;
use crate::wallet::AppState;

/// What is stored: coin ids and digests as lowercase hex.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedCoinsRecord {
    /// Coins with a signature outside the wallet ([`SignedCoins::released`]).
    #[serde(default)]
    released: BTreeSet<String>,
    /// Per coin of a sealed bundle, the [`kit::sealed_digest`]s of the signatures Klimper
    /// made for it there ([`SignedCoins::sealed`]).
    #[serde(default)]
    sealed: BTreeMap<String, BTreeSet<String>>,
    /// The first form of the sealed record (a4d8c02f): `<public key>:<signed message>` per
    /// coin. Read once and turned into digests ([`Self::migrated`]); never written.
    #[serde(default, skip_serializing)]
    sealed_messages: BTreeMap<String, BTreeSet<String>>,
}

fn parse(hex_id: &str) -> Option<Bytes32> {
    let bytes = hex::decode(hex_id).ok()?;
    Bytes32::try_from(bytes.as_slice()).ok()
}

impl SignedCoinsRecord {
    /// The record as wallet-kit takes it.
    pub fn to_kit(&self) -> SignedCoins {
        SignedCoins {
            released: self.released.iter().filter_map(|c| parse(c)).collect(),
            // wallet-kit keys it by digest, each with the coins it is held for; stored per
            // coin, as the entries leave the record coin by coin.
            sealed: {
                let mut sealed: HashMap<Bytes32, HashSet<Bytes32>> = HashMap::new();
                for (coin, digests) in &self.sealed {
                    let Some(coin) = parse(coin) else { continue };
                    for digest in digests.iter().filter_map(|d| parse(d)) {
                        sealed.entry(digest).or_default().insert(coin);
                    }
                }
                sealed
            },
        }
    }

    /// The record in its current form: messages recorded by the first version become
    /// digests, so nothing it recorded is ever signed again.
    #[must_use]
    pub fn migrated(mut self) -> Self {
        for (coin, entries) in std::mem::take(&mut self.sealed_messages) {
            for entry in entries {
                let Some((key, message)) = entry.split_once(':') else {
                    continue;
                };
                let key = hex::decode(key)
                    .ok()
                    .and_then(|bytes| <[u8; 48]>::try_from(bytes.as_slice()).ok())
                    .and_then(|bytes| PublicKey::from_bytes(&bytes).ok());
                if let (Some(key), Ok(message)) = (key, hex::decode(message)) {
                    self.sealed
                        .entry(coin.clone())
                        .or_default()
                        .insert(hex::encode(kit::sealed_digest(&key, &message)));
                }
            }
        }
        self
    }

    /// Signatures over these coins left the wallet.
    pub fn release(&mut self, coins: impl IntoIterator<Item = Bytes32>) {
        self.released.extend(coins.into_iter().map(hex::encode));
    }

    /// What the kit reported before a broadcast ([`SignedCoins::add`]).
    pub fn add(&mut self, record: &BroadcastRecord) {
        self.release(record.coins.iter().copied());
        for (coin, digest) in &record.sealed {
            self.sealed
                .entry(hex::encode(coin))
                .or_default()
                .insert(hex::encode(digest));
        }
    }

    /// Every coin whose state on chain decides what stays in the record.
    pub fn coins(&self) -> Vec<Bytes32> {
        let mut all: BTreeSet<&String> = self.released.iter().collect();
        all.extend(self.sealed.keys());
        all.into_iter().filter_map(|c| parse(c)).collect()
    }

    /// `coin` is spent on chain: forget it ([`SignedCoins::spent`]). Only that coin: the
    /// other coins of its bundle keep their entries until each is spent itself.
    pub fn spent(&mut self, coin: &Bytes32) {
        let coin = hex::encode(coin);
        self.released.remove(&coin);
        self.sealed.remove(&coin);
    }

    /// Every coin of `spent` is spent on chain.
    pub fn prune(&mut self, spent: &HashSet<Bytes32>) {
        for coin in spent {
            self.spent(coin);
        }
    }

    /// Nothing recorded.
    pub fn is_empty(&self) -> bool {
        self.released.is_empty() && self.sealed.is_empty() && self.sealed_messages.is_empty()
    }
}

/// The most sealed bundles kept to send again, per wallet and network; the oldest goes
/// first. Losing one loses only the way to resend it: its signatures stay in the record.
pub const MAX_KEPT_BUNDLES: usize = 16;

/// One sealed bundle Klimper broadcast (or tried to), as sent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptBundle {
    /// The spend bundle, serialised (`Streamable`), hex.
    pub bundle: String,
    /// The coins it spends, hex.
    pub coins: BTreeSet<String>,
    /// When it was first sent, unix seconds.
    pub sent_at: u64,
}

/// The sealed bundles Klimper broadcast that can still be included, by bundle id (hex): the
/// only way to send one again, since the kit never signs its messages a second time. A
/// bundle is dropped once any coin it spends is spent on chain (it can never be included
/// then), or when more than [`MAX_KEPT_BUNDLES`] are kept.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedBundles {
    #[serde(default)]
    pub bundles: BTreeMap<String, KeptBundle>,
}

impl SealedBundles {
    pub fn keep(&mut self, bundle: &SpendBundle, now: u64) {
        let Ok(bytes) = bundle.to_bytes() else {
            return;
        };
        self.bundles
            .entry(hex::encode(bundle.name()))
            .or_insert_with(|| KeptBundle {
                bundle: hex::encode(bytes),
                coins: bundle
                    .coin_spends
                    .iter()
                    .map(|cs| hex::encode(cs.coin.coin_id()))
                    .collect(),
                sent_at: now,
            });
        while self.bundles.len() > MAX_KEPT_BUNDLES {
            let Some(oldest) = self
                .bundles
                .iter()
                .min_by_key(|(_, kept)| kept.sent_at)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            self.bundles.remove(&oldest);
        }
    }

    pub fn coins(&self) -> impl Iterator<Item = Bytes32> + '_ {
        self.bundles
            .values()
            .flat_map(|kept| kept.coins.iter())
            .filter_map(|c| parse(c))
    }

    /// Drop every bundle one of whose coins is spent on chain.
    pub fn prune(&mut self, spent: &HashSet<Bytes32>) {
        let spent: HashSet<String> = spent.iter().map(hex::encode).collect();
        self.bundles
            .retain(|_, kept| !kept.coins.iter().any(|c| spent.contains(c)));
    }

    /// The bundle with this id, if kept.
    pub fn get(&self, bundle_id: &Bytes32) -> Option<SpendBundle> {
        let kept = self.bundles.get(&hex::encode(bundle_id))?;
        SpendBundle::from_bytes(&hex::decode(&kept.bundle).ok()?).ok()
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

    /// Store what the kit reported before a broadcast ([`Broadcaster::record`]).
    pub fn add(&self, record: &BroadcastRecord) -> Result<()> {
        self.update(|stored| stored.add(record)).map(drop)
    }

    /// The sealed bundles kept to send again.
    pub fn sealed_bundles(&self) -> Result<SealedBundles> {
        self.store
            .load_sealed_bundles(self.fingerprint, &self.chain_id)
    }

    /// Keep a sealed bundle's bytes to send it again ([`resend`]).
    pub fn keep_bundle(&self, bundle: &SpendBundle, now: u64) -> Result<()> {
        self.store
            .update_sealed_bundles(self.fingerprint, &self.chain_id, |kept| {
                kept.keep(bundle, now);
            })
            .map(drop)
    }

    /// Every coin whose state on chain decides what stays: the record's and the kept
    /// bundles'.
    pub fn coins(&self) -> Result<Vec<Bytes32>> {
        let mut coins: BTreeSet<Bytes32> = self.load()?.coins().into_iter().collect();
        coins.extend(self.sealed_bundles()?.coins());
        Ok(coins.into_iter().collect())
    }

    /// Every coin of `spent` is spent on chain: it leaves the record, and every kept bundle
    /// spending one of them is dropped.
    pub fn prune(&self, spent: &HashSet<Bytes32>) -> Result<SignedCoinsRecord> {
        self.store
            .update_sealed_bundles(self.fingerprint, &self.chain_id, |kept| {
                kept.prune(spent);
            })?;
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

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Puts what the kit reports into the record before anything is sent ([`Broadcaster::record`]:
/// written to the keyring before it returns, refusing the broadcast when it cannot be), and
/// keeps every sealed bundle's bytes to send it again. Sending is `send`'s.
pub struct Recorder<'a> {
    record: &'a SignedCoinsHandle,
    /// The last record reported was sealed: keep the bundle that follows.
    sealed: AtomicBool,
}

impl std::fmt::Debug for Recorder<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder").finish_non_exhaustive()
    }
}

impl<'a> Recorder<'a> {
    pub fn new(record: &'a SignedCoinsHandle) -> Self {
        Recorder {
            record,
            sealed: AtomicBool::new(false),
        }
    }

    /// [`Broadcaster::record`]: stored durably, or the bundle is not sent.
    pub fn record(&self, record: &BroadcastRecord) -> std::result::Result<(), String> {
        self.sealed.store(false, Ordering::SeqCst);
        if let Err(error) = self.record.add(record) {
            tracing::error!("xchonnect: the record of signed coins could not be written: {error}");
            return Err("the wallet could not record this transaction".to_owned());
        }
        self.sealed
            .store(!record.sealed.is_empty(), Ordering::SeqCst);
        Ok(())
    }

    /// Before [`Broadcaster::submit`] sends `bundle`: keep a sealed bundle's bytes. A failure
    /// here costs only the way to send it again.
    pub fn before_send(&self, bundle: &SpendBundle) {
        if self.sealed.swap(false, Ordering::SeqCst)
            && let Err(error) = self.record.keep_bundle(bundle, now())
        {
            tracing::warn!("xchonnect: a sealed bundle could not be kept to resend: {error}");
        }
    }
}

/// Wraps a broadcaster with Klimper's [`Recorder`] (tests: a simulated chain behind it).
pub struct Recording<'a> {
    inner: &'a dyn Broadcaster,
    recorder: Recorder<'a>,
}

impl std::fmt::Debug for Recording<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recording").finish_non_exhaustive()
    }
}

impl<'a> Recording<'a> {
    pub fn new(inner: &'a dyn Broadcaster, record: &'a SignedCoinsHandle) -> Self {
        Recording {
            inner,
            recorder: Recorder::new(record),
        }
    }
}

impl Broadcaster for Recording<'_> {
    fn record(&self, record: &BroadcastRecord) -> std::result::Result<(), String> {
        self.recorder.record(record)?;
        self.inner.record(record)
    }

    fn submit(&self, bundle: &SpendBundle) -> std::result::Result<Broadcast, String> {
        self.recorder.before_send(bundle);
        self.inner.submit(bundle)
    }
}

/// Send a kept sealed bundle again, byte for byte: the way to push a sealed transaction
/// that is stuck (no peer took it, or it was dropped from the mempool), since the kit never
/// signs its messages a second time. Its signatures are in the record already; nothing is
/// signed. `None`: no such bundle is kept (it was never sent, or a coin it spends is spent).
pub fn resend(
    record: &SignedCoinsHandle,
    bundle_id: &Bytes32,
    send: impl FnOnce(&SpendBundle) -> std::result::Result<Broadcast, String>,
) -> Result<Option<std::result::Result<Broadcast, String>>> {
    let Some(bundle) = record.sealed_bundles()?.get(bundle_id) else {
        return Ok(None);
    };
    Ok(Some(send(&bundle)))
}

/// Requests that sign coin spends run one at a time, from reading the record to writing it.
static GATE: Mutex<()> = Mutex::new(());

/// Wait for the other requests that sign coin spends to finish.
pub fn gate() -> MutexGuard<'static, ()> {
    GATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How deep below the wallet's synced peak a coin's spend must be before the coin leaves
/// the record ([`SignedCoins::spent`]): a spend in a block a reorg could still undo leaves
/// the coin unspent again, with its sealed signatures out there.
pub const CONFIRMATIONS: u32 = 6;

/// Which of `rows` (coin id, height it was spent at) are spent at least [`CONFIRMATIONS`]
/// blocks below `peak`. None without a peak.
pub fn confirmed_spent(
    rows: impl IntoIterator<Item = (Bytes32, Option<u32>)>,
    peak: Option<u32>,
) -> HashSet<Bytes32> {
    let Some(peak) = peak else {
        return HashSet::new();
    };
    rows.into_iter()
        .filter(|(_, spent)| {
            spent.is_some_and(|height| peak.saturating_sub(height) >= CONFIRMATIONS)
        })
        .map(|(coin, _)| coin)
        .collect()
}

/// Which of `coins` are spent on chain deeply enough to forget: from the wallet's own synced
/// database only (Sage's coin states and its peak), [`CONFIRMATIONS`] blocks deep. A coin
/// the database does not hold (another party's, or created inside a bundle) is never
/// pruned on a peer's word: it stays in the record, at about 64 bytes per entry.
pub async fn spent_coins(wallet: &sage_wallet::Wallet, coins: &[Bytes32]) -> HashSet<Bytes32> {
    if coins.is_empty() {
        return HashSet::new();
    }
    let ids: Vec<String> = coins.iter().map(hex::encode).collect();
    let rows = match wallet.db.coins_by_ids(&ids).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!("xchonnect: could not read coin states: {error}");
            return HashSet::new();
        }
    };
    let peak = match wallet.db.latest_peak().await {
        Ok(peak) => peak.map(|(height, _)| height),
        Err(error) => {
            tracing::warn!("xchonnect: could not read the synced peak: {error}");
            return HashSet::new();
        }
    };
    confirmed_spent(
        rows.into_iter()
            .map(|row| (row.coin.coin_id(), row.spent_height)),
        peak,
    )
}

/// The record, with every coin spent [`CONFIRMATIONS`] deep in the wallet's synced database
/// taken out (each coin on its own: [`SignedCoins::spent`]), and the kept bundles that can
/// no longer be included dropped. Run on the request thread (see `requests::answer`).
pub fn refresh(state: &AppState, handle: &SignedCoinsHandle) -> Result<SignedCoinsRecord> {
    let wallet = {
        let state = state.clone();
        tauri::async_runtime::block_on(async move {
            state.lock().await.wallet().map_err(Error::internal)
        })?
    };
    let by_sage = {
        let wallet = wallet.clone();
        tauri::async_runtime::block_on(async move { signed_by_sage(&wallet).await })?
    };
    handle.release(by_sage)?;
    let record = handle.load()?;
    let coins = handle.coins()?;
    if coins.is_empty() {
        return Ok(record);
    }
    let spent = tauri::async_runtime::block_on(async move { spent_coins(&wallet, &coins).await });
    if spent.is_empty() {
        return Ok(record);
    }
    handle.prune(&spent)
}

/// The wallet's unspent coins with a signature Sage made outside Xchonnect, as far as Sage's
/// database shows them (xchonnect spec 11.2: the host records signatures made outside the
/// kit): every coin a transaction in Sage's mempool table spends (Sage's own send screen,
/// its offer takes, anything Sage submitted), and every coin of the wallet's an offer Sage
/// made (any status: an expired or cancelled offer's signature is still out there). Only
/// coins Sage's database holds unspent count; Sage's code is not changed for this.
///
/// Not covered, since they leave no trace in Sage's database: signatures Sage hands to a
/// website through its own `WalletConnect` (`signCoinSpends`) and a signed transaction shown
/// for copying instead of being submitted.
pub async fn signed_by_sage(wallet: &sage_wallet::Wallet) -> Result<Vec<Bytes32>> {
    let mut coins = BTreeSet::new();
    for item in wallet
        .db
        .mempool_items_with_coins()
        .await
        .map_err(Error::internal)?
    {
        coins.extend(item.spent.iter().map(|spent| spent.coin.coin_id()));
    }
    for offer in wallet.db.offers(None).await.map_err(Error::internal)? {
        match chia_wallet_sdk::driver::decode_offer(&offer.encoded_offer) {
            Ok(bundle) => coins.extend(
                bundle
                    .coin_spends
                    .iter()
                    .filter(|cs| cs.coin.parent_coin_info != Bytes32::default())
                    .map(|cs| cs.coin.coin_id()),
            ),
            Err(error) => tracing::warn!("xchonnect: an offer Sage made does not decode: {error}"),
        }
    }
    if coins.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = coins.iter().map(hex::encode).collect();
    Ok(wallet
        .db
        .coins_by_ids(&ids)
        .await
        .map_err(Error::internal)?
        .into_iter()
        .filter(|row| row.spent_height.is_none())
        .map(|row| row.coin.coin_id())
        .collect())
}

/// Every 30 seconds while the app runs, the coins Sage signed outside Xchonnect
/// ([`signed_by_sage`]) go into the active wallet's record, so one Sage submitted and then
/// dropped between two requests is not missed.
pub fn spawn_sweeper<R: tauri::Runtime>(app: tauri::AppHandle<R>, store: Arc<SessionStore>) {
    use tauri::Manager as _;
    tauri::async_runtime::spawn(async move {
        let mut ticks = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            ticks.tick().await;
            // Sage's state is managed once the app has set up, after the plugins.
            let Some(state) = app.try_state::<AppState>().map(|s| s.inner().clone()) else {
                continue;
            };
            let (wallet, chain_id) = {
                let sage = state.lock().await;
                let Ok(wallet) = sage.wallet() else { continue };
                let chain_id = crate::wallet::network_of(sage.network())
                    .chain_id()
                    .to_owned();
                (wallet, chain_id)
            };
            let coins = match signed_by_sage(&wallet).await {
                Ok(coins) if !coins.is_empty() => coins,
                Ok(_) => continue,
                Err(error) => {
                    tracing::warn!("xchonnect: could not read what Sage signed: {error}");
                    continue;
                }
            };
            let handle = SignedCoinsHandle::new(store.clone(), wallet.fingerprint, &chain_id);
            let written = tauri::async_runtime::spawn_blocking(move || {
                let _one_at_a_time = gate();
                handle.release(coins)
            })
            .await;
            if let Ok(Err(error)) = written {
                tracing::error!(
                    "xchonnect: the record of signed coins could not be written: {error}"
                );
            }
        }
    });
}

/// The record a coin-signing request is checked against, with the lock that keeps every
/// other coin-signing request out until this one is answered ([`gate`]). The caller holds the
/// guard from wallet-kit's check of the record, across the approval prompt, until what it
/// signed is recorded: a request that starts while another waits for its approval sees the
/// other's record. The record is read only once the lock is held. `signs` false: no lock and
/// an empty record (the request signs nothing).
pub fn checked_record(
    handle: &SignedCoinsHandle,
    signs: bool,
    refresh: impl FnOnce(&SignedCoinsHandle) -> Result<SignedCoinsRecord>,
) -> Result<(Option<MutexGuard<'static, ()>>, SignedCoins)> {
    if !signs {
        return Ok((None, SignedCoins::default()));
    }
    let guard = gate();
    let record = refresh(handle)?;
    Ok((Some(guard), record.to_kit()))
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

    fn key(seed: u8) -> PublicKey {
        chia_bls::SecretKey::from_seed(&[seed; 32]).public_key()
    }

    /// Spec 11.2: each coin's sealed signatures stay until **that** coin is spent, never
    /// because another coin of the bundle was spent.
    #[test]
    fn a_sealed_coin_leaves_the_record_only_when_it_is_spent_itself() {
        let (a, b, c) = (coin(1), coin(2), coin(3));
        let mut record = SignedCoinsRecord::default();
        record.release([c.coin_id()]);
        let digest = kit::sealed_digest(&key(9), b"signed");
        // `b` is someone else's coin in the bundle: released, with no signature of ours.
        record.add(&BroadcastRecord {
            coins: vec![a.coin_id(), b.coin_id()],
            sealed: vec![(a.coin_id(), digest)],
        });
        let kit = record.to_kit();
        assert!(kit.is_sealed(&key(9), b"signed"));
        assert!(!kit.is_sealed(&key(9), b"another spend"));
        assert_eq!(
            kit.released,
            [a.coin_id(), b.coin_id(), c.coin_id()]
                .into_iter()
                .collect()
        );

        // The other party's coin is spent elsewhere: the bundle can never be included, but
        // what Klimper signed for `a` stays recorded, and `a` stays released.
        record.prune(&[b.coin_id()].into_iter().collect());
        let kit = record.to_kit();
        assert!(kit.is_sealed(&key(9), b"signed"));
        assert!(kit.released.contains(&a.coin_id()));

        record.prune(&[a.coin_id(), c.coin_id()].into_iter().collect());
        assert!(record.is_empty());
    }

    /// A spend leaves the record only `CONFIRMATIONS` blocks below the synced peak; an
    /// unspent coin, or any coin without a synced peak, never.
    #[test]
    fn a_spend_counts_only_once_it_is_deep_enough() {
        let (a, b) = (coin(1).coin_id(), coin(2).coin_id());
        let rows = || [(a, Some(100)), (b, None)];
        assert!(confirmed_spent(rows(), None).is_empty());
        assert!(confirmed_spent(rows(), Some(100)).is_empty());
        assert!(confirmed_spent(rows(), Some(100 + CONFIRMATIONS - 1)).is_empty());
        assert_eq!(
            confirmed_spent(rows(), Some(100 + CONFIRMATIONS)),
            [a].into_iter().collect()
        );
    }

    /// The record's own pruning is wallet-kit's `SignedCoins::spent`, coin by coin.
    #[test]
    fn pruning_is_the_kits_spent() {
        let (a, b) = (coin(1), coin(2));
        let report = BroadcastRecord {
            coins: vec![a.coin_id(), b.coin_id()],
            sealed: vec![
                (a.coin_id(), kit::sealed_digest(&key(1), b"a")),
                (b.coin_id(), kit::sealed_digest(&key(1), b"b")),
            ],
        };
        let mut ours = SignedCoinsRecord::default();
        ours.add(&report);
        let mut theirs = SignedCoins::default();
        theirs.add(&report);
        assert_eq!(ours.to_kit(), theirs);
        ours.prune(&[a.coin_id()].into_iter().collect());
        theirs.spent(&a.coin_id());
        assert_eq!(ours.to_kit(), theirs);
    }

    /// What the first version stored (`sealed_messages`) is read as digests, never lost.
    #[test]
    fn messages_recorded_by_the_first_version_are_read_as_digests() {
        let a = coin(1);
        let stored = serde_json::json!({
            "released": [hex::encode(a.coin_id())],
            "in_flight": { "00": [hex::encode(a.coin_id())] },
            "sealed_messages": {
                hex::encode(a.coin_id()): [format!(
                    "{}:{}",
                    hex::encode(key(4).to_bytes()),
                    hex::encode(b"message")
                )]
            }
        });
        let record: SignedCoinsRecord = serde_json::from_value(stored).unwrap();
        let record = record.migrated();
        assert!(record.to_kit().is_sealed(&key(4), b"message"));
        let written = serde_json::to_value(&record).unwrap();
        assert!(written.get("sealed_messages").is_none());
        assert!(written.get("in_flight").is_none());
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
            .add(&BroadcastRecord {
                coins: vec![b.coin_id()],
                sealed: vec![(b.coin_id(), kit::sealed_digest(&key(2), b"m"))],
            })
            .unwrap();
        handle.keep_bundle(&bundle(&[b]), 1).unwrap();

        // A new store over the same keyring: the app started again.
        let again = SignedCoinsHandle::new(open(), 7, "testnet11");
        assert_eq!(again.load().unwrap(), handle.load().unwrap());
        assert!(again.load().unwrap().to_kit().is_sealed(&key(2), b"m"));
        assert!(
            again
                .load()
                .unwrap()
                .to_kit()
                .released
                .contains(&a.coin_id())
        );
        assert_eq!(
            again.sealed_bundles().unwrap().get(&bundle(&[b]).name()),
            Some(bundle(&[b]))
        );

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

    /// A kept bundle goes once a coin it spends is spent; at most `MAX_KEPT_BUNDLES` are kept.
    #[test]
    fn kept_bundles_go_when_they_can_no_longer_be_included() {
        let (a, b) = (coin(1), coin(2));
        let mut kept = SealedBundles::default();
        kept.keep(&bundle(&[a, b]), 1);
        assert!(kept.get(&bundle(&[a, b]).name()).is_some());
        kept.prune(&[b.coin_id()].into_iter().collect());
        assert!(kept.bundles.is_empty());

        for i in 0..=u8::try_from(MAX_KEPT_BUNDLES).unwrap() {
            kept.keep(&bundle(&[coin(10 + i)]), u64::from(i));
        }
        assert_eq!(kept.bundles.len(), MAX_KEPT_BUNDLES);
        assert!(kept.get(&bundle(&[coin(10)]).name()).is_none());
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
