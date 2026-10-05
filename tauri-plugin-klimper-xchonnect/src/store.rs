// SPDX-License-Identifier: AGPL-3.0-or-later
//! Session storage (clapandpay TASK-073, xchonnect spec 11.3 and integration guide §2).
//!
//! Xchonnect session state is epoch keys, mailbox ids, capability tokens and sequence
//! counters: a transport secret, not signing material. It lives in the **OS keyring**
//! under its own service name, `com.clapandpay.klimper.xchonnect`, and never in
//! `keys.bin` (the Sage signing keychain), the wallet `SQLite` database, a log line or a
//! device backup. Compromising a session key must not reach signing material, so the two
//! stores share nothing: `sage-keychain` is untouched by this module.
//!
//! Two keyring entries exist per session, so that the message loop (which owns the
//! `Session`) and the wallet-kit limit store (which runs inside `handle()` on a blocking
//! thread) never overwrite each other:
//!
//! | Entry | Holds |
//! |---|---|
//! | `session:<fingerprint>:<id>` | the session record and `Session::to_bytes()` |
//! | `limits:<fingerprint>:<id>` | the per-dApp daily spend totals |
//!
//! A small plaintext index next to the wallet data lists the session ids per wallet
//! fingerprint together with the last `state_version` that was written. The keyring entry
//! is written **before** the index, so a crash between the two leaves the keyring ahead of
//! the index, never behind it. An entry whose `state_version` is *below* the index is
//! restored state: `seq` must never be reused, so the session is marked as needing
//! re-pairing instead (spec 5.3).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use xchonnect_wallet_kit::{AssetId, AssetLimit, DailySpend, DappPermissions, PermissionError};

use crate::error::{Error, Result};

/// Keyring service name. Deliberately distinct from anything Sage uses: Klimper's signing
/// keys are a file (`keys.bin`) opened by the passkey, and no Xchonnect secret may ever be
/// written there (clapandpay TASK-073, decision-008 point 4).
pub const KEYRING_SERVICE: &str = "com.clapandpay.klimper.xchonnect";

/// Name of the index file inside the Sage app data directory.
const INDEX_FILE: &str = "xchonnect-sessions.json";

// ---------------------------------------------------------------------------------------
// Secret backend
// ---------------------------------------------------------------------------------------

/// A place to keep bytes that must not be readable without the user's login session.
pub trait SecretStore: std::fmt::Debug + Send + Sync {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()>;
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    fn delete(&self, key: &str) -> Result<()>;
}

/// The OS keyring: macOS and iOS Keychain, Windows Credential Manager, the Linux Secret
/// Service. On Apple platforms `keyring` stores a generic password item, which the system
/// keeps out of iCloud Keychain unless it is marked synchronisable — it is not.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeyring;

#[cfg(any(target_vendor = "apple", target_os = "windows", target_os = "linux"))]
impl SecretStore for OsKeyring {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        entry(key)?
            .set_secret(bytes)
            .map_err(|e| Error::Keyring(e.to_string()))
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match entry(key)?.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Keyring(e.to_string())),
        }
    }

    fn delete(&self, key: &str) -> Result<()> {
        match entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Keyring(e.to_string())),
        }
    }
}

#[cfg(any(target_vendor = "apple", target_os = "windows", target_os = "linux"))]
fn entry(key: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, key).map_err(|e| Error::Keyring(e.to_string()))
}

/// Platforms without a credential store: refuse rather than fall back to a file, which
/// would put session keys on disk unencrypted.
#[cfg(not(any(target_vendor = "apple", target_os = "windows", target_os = "linux")))]
impl SecretStore for OsKeyring {
    fn put(&self, _key: &str, _bytes: &[u8]) -> Result<()> {
        Err(Error::Keyring("no OS keyring on this platform".to_owned()))
    }
    fn get(&self, _key: &str) -> Result<Option<Vec<u8>>> {
        Err(Error::Keyring("no OS keyring on this platform".to_owned()))
    }
    fn delete(&self, _key: &str) -> Result<()> {
        Ok(())
    }
}

/// In-memory backend for tests. Never used in the app.
#[derive(Debug, Default)]
pub struct MemorySecrets(Mutex<BTreeMap<String, Vec<u8>>>);

impl SecretStore for MemorySecrets {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let mut map = self.0.lock().map_err(Error::internal)?;
        map.insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let map = self.0.lock().map_err(Error::internal)?;
        Ok(map.get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<()> {
        let mut map = self.0.lock().map_err(Error::internal)?;
        map.remove(key);
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------------------

/// Per-asset limit, in a form that round-trips through JSON (`AssetId` is
/// serialise-only in wallet-kit).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredLimit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_request: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_day: Option<u128>,
}

/// `AssetId` as a storage key: `xch`, or `cat:<64 hex>`.
pub fn asset_key(asset: AssetId) -> String {
    match asset {
        AssetId::Xch => "xch".to_owned(),
        AssetId::Cat(tail) => format!("cat:{}", hex::encode(tail)),
    }
}

/// Inverse of [`asset_key`]; unknown keys are dropped rather than failing a load.
pub fn parse_asset_key(key: &str) -> Option<AssetId> {
    if key == "xch" {
        return Some(AssetId::Xch);
    }
    let hash: [u8; 32] = hex::decode(key.strip_prefix("cat:")?)
        .ok()?
        .try_into()
        .ok()?;
    Some(AssetId::Cat(chia_protocol::Bytes32::from(hash)))
}

/// What this dApp may do, persisted. Keys are hex `0x…` so the record is readable and
/// stable across BLS crate versions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredPermissions {
    pub methods: Vec<String>,
    pub exposed_keys: Vec<String>,
    #[serde(default)]
    pub limits: BTreeMap<String, StoredLimit>,
}

impl StoredPermissions {
    pub fn from_kit(perms: &DappPermissions) -> Self {
        StoredPermissions {
            methods: perms.methods.clone(),
            exposed_keys: perms
                .exposed_keys
                .iter()
                .map(|k| format!("0x{}", hex::encode(k.to_bytes())))
                .collect(),
            limits: perms
                .limits
                .iter()
                .map(|(asset, limit)| {
                    (
                        asset_key(*asset),
                        StoredLimit {
                            per_request: limit.per_request,
                            per_day: limit.per_day,
                        },
                    )
                })
                .collect(),
        }
    }

    /// Rebuild the wallet-kit permissions. A stored key that no longer parses is dropped:
    /// a dApp must never end up with *more* than what was granted, and `handle()` refuses
    /// a key the wallet cannot sign for anyway.
    pub fn to_kit(&self) -> DappPermissions {
        DappPermissions {
            methods: self.methods.clone(),
            exposed_keys: self
                .exposed_keys
                .iter()
                .filter_map(|hex_key| parse_public_key(hex_key))
                .collect(),
            limits: self
                .limits
                .iter()
                .filter_map(|(key, limit)| {
                    Some((
                        parse_asset_key(key)?,
                        AssetLimit {
                            per_request: limit.per_request,
                            per_day: limit.per_day,
                        },
                    ))
                })
                .collect(),
        }
    }
}

pub fn parse_public_key(value: &str) -> Option<chia_bls::PublicKey> {
    let bytes: [u8; 48] = hex::decode(value.strip_prefix("0x").unwrap_or(value))
        .ok()?
        .try_into()
        .ok()?;
    chia_bls::PublicKey::from_bytes(&bytes).ok()
}

/// Everything Klimper keeps about one connected dApp, besides the opaque session bytes.
///
/// The flags stay flags: this is a stored record whose JSON shape has to be readable and
/// forward compatible, and folding them into enums would change the on-disk format for
/// no gain.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Opaque session id (16 random bytes, base64url). Not a secret.
    pub id: String,
    /// Wallet this session belongs to. Sessions never cross wallets (TASK-073 AC #3).
    pub fingerprint: u32,
    /// Verified dApp domain in ASCII A-label form.
    pub dapp: String,
    /// Name from the origin document.
    pub dapp_name: String,
    /// Relay base URL from the pairing URI.
    pub relay: String,
    /// Chain the user approved the session for (`chainId`).
    pub chain_id: String,
    pub created_at: u64,
    /// Bumped on every write; compared with the index to detect a rollback.
    pub state_version: u64,
    /// Developer-mode session (plain-HTTP loopback relay). Debug builds only.
    pub developer_mode: bool,
    /// The user compared the six digits and confirmed. Until then the session is unusable.
    pub sas_confirmed: bool,
    /// Stored state was lost or rolled back: re-pair before using this session.
    #[serde(default)]
    pub needs_repair: bool,
    pub permissions: StoredPermissions,
    #[serde(default)]
    pub allow_agg_sig_unsafe: bool,
    #[serde(default)]
    pub allow_unknown_contracts: bool,
}

/// A session record plus the opaque `Session::to_bytes()` blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSession {
    pub record: SessionRecord,
    /// `xchonnect_core::session::Session::to_bytes()`, hex encoded. Contains epoch keys
    /// and capability tokens.
    pub session_hex: String,
}

impl StoredSession {
    pub fn session_bytes(&self) -> Result<Vec<u8>> {
        hex::decode(&self.session_hex)
            .map_err(|_| Error::Keyring("stored session is corrupt".to_owned()))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct IndexEntry {
    id: String,
    fingerprint: u32,
    state_version: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    sessions: Vec<IndexEntry>,
}

// ---------------------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------------------

/// Session storage. Cheap to clone the `Arc` around it; every method is synchronous
/// because both the OS keyring and the index file are.
#[derive(Debug)]
pub struct SessionStore {
    secrets: Box<dyn SecretStore>,
    index_path: PathBuf,
    index: Mutex<Index>,
}

impl SessionStore {
    /// Open the store. `data_dir` is the Sage app data directory.
    pub fn open(data_dir: &Path, secrets: Box<dyn SecretStore>) -> Result<Self> {
        let index_path = data_dir.join(INDEX_FILE);
        let index = match std::fs::read(&index_path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Index::default(),
            Err(error) => return Err(Error::internal(error)),
        };
        Ok(SessionStore {
            secrets,
            index_path,
            index: Mutex::new(index),
        })
    }

    fn session_key(fingerprint: u32, id: &str) -> String {
        format!("session:{fingerprint}:{id}")
    }

    fn limits_key(fingerprint: u32, id: &str) -> String {
        format!("limits:{fingerprint}:{id}")
    }

    fn write_index(&self, index: &Index) -> Result<()> {
        if let Some(parent) = self.index_path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::internal)?;
        }
        let bytes = serde_json::to_vec_pretty(index).map_err(Error::internal)?;
        // Write then rename, so a crash never leaves a truncated index (which would look
        // like state loss for every session).
        let tmp = self.index_path.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes).map_err(Error::internal)?;
        std::fs::rename(&tmp, &self.index_path).map_err(Error::internal)?;
        Ok(())
    }

    /// Persist a session. Bumps `state_version`, writes the keyring entry and only then
    /// the index. Callers must do this **before** posting the envelope the change
    /// produced (integration guide §2).
    pub fn save(&self, stored: &mut StoredSession) -> Result<()> {
        let (fingerprint, id) = (stored.record.fingerprint, stored.record.id.clone());
        stored.record.state_version = stored.record.state_version.saturating_add(1);
        let blob = serde_json::to_vec(stored).map_err(Error::internal)?;
        self.secrets
            .put(&Self::session_key(fingerprint, &id), &blob)?;

        let mut index = self.index.lock().map_err(Error::internal)?;
        match index.sessions.iter_mut().find(|e| e.id == id) {
            Some(entry) => entry.state_version = stored.record.state_version,
            None => index.sessions.push(IndexEntry {
                id,
                fingerprint,
                state_version: stored.record.state_version,
            }),
        }
        self.write_index(&index)
    }

    /// Load a session. A `state_version` below the index means stored state was restored
    /// from a backup or lost: the session is marked as needing re-pairing and never used
    /// again, because reusing a `seq` would let a replayed envelope through (spec 5.3).
    pub fn load(&self, fingerprint: u32, id: &str) -> Result<StoredSession> {
        let Some(blob) = self.secrets.get(&Self::session_key(fingerprint, id))? else {
            return Err(Error::UnknownSession);
        };
        let mut stored: StoredSession =
            serde_json::from_slice(&blob).map_err(|_| Error::UnknownSession)?;
        let expected = {
            let index = self.index.lock().map_err(Error::internal)?;
            index
                .sessions
                .iter()
                .find(|e| e.id == id)
                .map_or(0, |e| e.state_version)
        };
        if stored.record.state_version < expected {
            tracing::warn!(
                session = %id,
                "xchonnect session state rolled back ({} < {}): re-pairing required",
                stored.record.state_version,
                expected
            );
            stored.record.needs_repair = true;
        }
        Ok(stored)
    }

    /// Every session of one wallet, oldest first. Unreadable entries are skipped rather
    /// than failing the whole list.
    pub fn list(&self, fingerprint: u32) -> Result<Vec<StoredSession>> {
        let ids: Vec<String> = {
            let index = self.index.lock().map_err(Error::internal)?;
            index
                .sessions
                .iter()
                .filter(|e| e.fingerprint == fingerprint)
                .map(|e| e.id.clone())
                .collect()
        };
        let mut out = Vec::new();
        for id in ids {
            match self.load(fingerprint, &id) {
                Ok(stored) => out.push(stored),
                Err(error) => tracing::warn!(session = %id, "unreadable session: {error}"),
            }
        }
        out.sort_by_key(|s| s.record.created_at);
        Ok(out)
    }

    /// Forget a session: both keyring entries and the index row.
    pub fn remove(&self, fingerprint: u32, id: &str) -> Result<()> {
        self.secrets.delete(&Self::session_key(fingerprint, id))?;
        self.secrets.delete(&Self::limits_key(fingerprint, id))?;
        let mut index = self.index.lock().map_err(Error::internal)?;
        index.sessions.retain(|e| e.id != id);
        self.write_index(&index)
    }

    /// Deleting a wallet deletes its sessions (TASK-073 AC #3). Mailboxes are deleted by
    /// the caller, which has the relay client.
    pub fn remove_wallet(&self, fingerprint: u32) -> Result<Vec<String>> {
        let ids: Vec<String> = {
            let index = self.index.lock().map_err(Error::internal)?;
            index
                .sessions
                .iter()
                .filter(|e| e.fingerprint == fingerprint)
                .map(|e| e.id.clone())
                .collect()
        };
        for id in &ids {
            self.remove(fingerprint, id)?;
        }
        Ok(ids)
    }

    fn load_limits(&self, fingerprint: u32, id: &str) -> Result<DailySpend> {
        let Some(blob) = self.secrets.get(&Self::limits_key(fingerprint, id))? else {
            return Ok(DailySpend::default());
        };
        let stored: StoredDailySpend = serde_json::from_slice(&blob).unwrap_or_default();
        Ok(DailySpend {
            day: stored.day,
            spent: stored
                .spent
                .iter()
                .filter_map(|(key, amount)| Some((parse_asset_key(key)?, *amount)))
                .collect(),
        })
    }

    fn save_limits(&self, fingerprint: u32, id: &str, record: &DailySpend) -> Result<()> {
        let stored = StoredDailySpend {
            day: record.day,
            spent: record
                .spent
                .iter()
                .map(|(asset, amount)| (asset_key(*asset), *amount))
                .collect(),
        };
        let blob = serde_json::to_vec(&stored).map_err(Error::internal)?;
        self.secrets.put(&Self::limits_key(fingerprint, id), &blob)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredDailySpend {
    day: u64,
    #[serde(default)]
    spent: BTreeMap<String, u128>,
}

/// wallet-kit's [`xchonnect_wallet_kit::LimitStore`] backed by the keyring, so daily
/// totals survive a restart and a reinstall of the app data directory (TASK-079 AC #3).
/// `check_spend`/`commit_spend` already refuse to reset a day when the clock moves
/// backwards; a storage failure refuses the request rather than signing unaccounted.
#[derive(Debug)]
pub struct PersistentLimits {
    store: std::sync::Arc<SessionStore>,
    fingerprint: u32,
    session_id: String,
}

impl PersistentLimits {
    pub fn new(store: std::sync::Arc<SessionStore>, fingerprint: u32, session_id: String) -> Self {
        PersistentLimits {
            store,
            fingerprint,
            session_id,
        }
    }
}

impl xchonnect_wallet_kit::LimitStore for PersistentLimits {
    fn load(&self) -> std::result::Result<DailySpend, PermissionError> {
        self.store
            .load_limits(self.fingerprint, &self.session_id)
            .map_err(|error| {
                tracing::error!("xchonnect limit store read failed: {error}");
                PermissionError::Storage
            })
    }

    fn save(&self, record: &DailySpend) -> std::result::Result<(), PermissionError> {
        self.store
            .save_limits(self.fingerprint, &self.session_id, record)
            .map_err(|error| {
                tracing::error!("xchonnect limit store write failed: {error}");
                PermissionError::Storage
            })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use xchonnect_wallet_kit::LimitStore;

    fn record(id: &str) -> SessionRecord {
        SessionRecord {
            id: id.to_owned(),
            fingerprint: 1234,
            dapp: "pengui.xyz".to_owned(),
            dapp_name: "Pengui".to_owned(),
            relay: "https://relay.example".to_owned(),
            chain_id: "testnet11".to_owned(),
            created_at: 1_790_000_000,
            state_version: 0,
            developer_mode: false,
            sas_confirmed: false,
            needs_repair: false,
            permissions: StoredPermissions::default(),
            allow_agg_sig_unsafe: false,
            allow_unknown_contracts: false,
        }
    }

    fn store(dir: &Path) -> SessionStore {
        SessionStore::open(dir, Box::new(MemorySecrets::default())).unwrap()
    }

    /// The real OS keyring, under the real service name. `#[ignore]` because it writes to
    /// the developer's login keychain and may prompt; run it by hand with
    /// `cargo test -p tauri-plugin-klimper-xchonnect -- --ignored os_keyring`.
    #[test]
    #[ignore = "writes to the OS keyring"]
    fn the_os_keyring_round_trips_a_session() {
        let keyring = OsKeyring;
        let key = "session:0:keyring-self-test";
        let secret = b"not a real session";
        keyring.put(key, secret).expect("write to the keyring");
        assert_eq!(keyring.get(key).unwrap().as_deref(), Some(&secret[..]));
        keyring.delete(key).expect("delete from the keyring");
        assert!(keyring.get(key).unwrap().is_none(), "gone after delete");
        // Deleting what is not there is not an error: cleanup must be idempotent.
        keyring.delete(key).unwrap();
    }

    /// A backend sharing one `MemorySecrets`, so a test can plant an older blob behind
    /// the store's back — what a device restore does.
    #[derive(Debug)]
    struct Shared(Arc<MemorySecrets>);

    impl SecretStore for Shared {
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

    #[test]
    fn a_session_round_trips_and_is_scoped_to_its_wallet() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let mut stored = StoredSession {
            record: record("one"),
            session_hex: "0a0b".to_owned(),
        };
        store.save(&mut stored).unwrap();
        assert_eq!(stored.record.state_version, 1);

        let back = store.load(1234, "one").unwrap();
        assert_eq!(back.session_bytes().unwrap(), vec![0x0a, 0x0b]);
        assert!(!back.record.needs_repair);
        // Another wallet's fingerprint does not see it.
        assert!(store.load(4321, "one").is_err());
        assert_eq!(store.list(1234).unwrap().len(), 1);
        assert!(store.list(4321).unwrap().is_empty());

        store.remove_wallet(1234).unwrap();
        assert!(store.list(1234).unwrap().is_empty());
        assert!(store.load(1234, "one").is_err());
    }

    /// The index is the newer of the two writes, so a restored keyring entry is detected.
    #[test]
    fn restored_state_is_marked_as_needing_re_pairing() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let store = SessionStore::open(dir.path(), Box::new(Shared(secrets.clone()))).unwrap();

        let mut stored = StoredSession {
            record: record("two"),
            session_hex: "00".to_owned(),
        };
        store.save(&mut stored).unwrap(); // version 1
        let old_blob = secrets.get("session:1234:two").unwrap().unwrap();
        store.save(&mut stored).unwrap(); // version 2, index says 2
        assert!(!store.load(1234, "two").unwrap().record.needs_repair);

        // A backup restore puts version 1 back while the index still says 2.
        secrets.put("session:1234:two", &old_blob).unwrap();
        assert!(store.load(1234, "two").unwrap().record.needs_repair);
    }

    /// Persisting before posting must make `seq` monotonic even if the app dies in
    /// between: reloading the saved bytes continues from the saved counter (TASK-073 AC #2).
    #[test]
    fn a_crash_between_saving_and_posting_never_reuses_a_seq() {
        use xchonnect_core::crypto::OsEntropy;
        use xchonnect_core::message::Message;

        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let (mut dapp, mut wallet) = crate::testing::paired_sessions();
        let now = 1_790_000_000;

        let mut stored = StoredSession {
            record: record("three"),
            session_hex: hex::encode(wallet.to_bytes().unwrap()),
        };
        store.save(&mut stored).unwrap();

        // Seal a ping, persist, then "crash" before the post ever happened.
        let first = wallet
            .seal(&mut OsEntropy, now, Message::SessionPing, 300)
            .unwrap();
        stored.session_hex = hex::encode(wallet.to_bytes().unwrap());
        store.save(&mut stored).unwrap();
        drop(wallet);

        // Restart: load the persisted state and seal again.
        let reloaded = store.load(1234, "three").unwrap();
        let mut wallet =
            xchonnect_core::session::Session::from_bytes(&reloaded.session_bytes().unwrap())
                .unwrap();
        let second = wallet
            .seal(&mut OsEntropy, now, Message::SessionPing, 300)
            .unwrap();

        // The dApp accepts both in order, which it only can if the second seq is higher.
        let d_mbx = dapp.own_mailbox();
        let a = dapp.open(now, &d_mbx, &first.envelope).unwrap();
        let b = dapp.open(now, &d_mbx, &second.envelope).unwrap();
        assert!(b.seq > a.seq, "seq must not be reused after a restart");
    }

    #[test]
    fn daily_limits_survive_a_restart_of_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let open =
            || Arc::new(SessionStore::open(dir.path(), Box::new(Shared(secrets.clone()))).unwrap());

        let limits = PersistentLimits::new(open(), 1234, "four".to_owned());
        let mut record = DailySpend {
            day: 20_000,
            spent: BTreeMap::new(),
        };
        record.spent.insert(AssetId::Xch, 500);
        limits.save(&record).unwrap();

        // A fresh store object (a restarted app) sees the same totals.
        let after_restart = PersistentLimits::new(open(), 1234, "four".to_owned());
        let loaded = after_restart.load().unwrap();
        assert_eq!(loaded.day, 20_000);
        assert_eq!(loaded.spent.get(&AssetId::Xch), Some(&500));
        // A different session has its own record.
        let other = PersistentLimits::new(open(), 1234, "five".to_owned());
        assert!(other.load().unwrap().spent.is_empty());
    }

    #[test]
    fn permissions_round_trip() {
        let pk = chia_bls::SecretKey::from_seed(&[7; 64]).public_key();
        let mut perms = DappPermissions::new_default(pk);
        perms.limits.insert(
            AssetId::Xch,
            AssetLimit {
                per_request: Some(1_000),
                per_day: Some(5_000),
            },
        );
        let back = StoredPermissions::from_kit(&perms).to_kit();
        assert_eq!(back, perms);
    }
}
