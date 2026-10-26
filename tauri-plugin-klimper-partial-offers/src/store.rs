// SPDX-License-Identifier: AGPL-3.0-or-later
//! The partial offers this wallet made: one JSON file per wallet and network, next to Sage's
//! wallet database. Not a table in that database: Sage numbers its own migrations, and one of
//! ours would collide with upstream's next (docs/partial-offers.md).

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

use crate::{
    Error,
    records::{Fill, PendingCancel, Status},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredOffer {
    /// The first partial coin's id, `0x` hex.
    pub id: String,
    /// Unix seconds.
    pub created_at: u64,
    /// The newest text known (the first until a fill is seen).
    pub latest: String,
    pub fills: Vec<Fill>,
    /// How it ended (filled or cancelled): set once, and then it is not followed again.
    #[serde(default)]
    pub ended: Option<Status>,
    /// A cancel sent and not yet seen on the chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel: Option<PendingCancel>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct File {
    offers: Vec<StoredOffer>,
}

/// Every change reads, edits and writes the whole file: one at a time, across the commands
/// that run at once (a list rewriting entries while an offer is made or deleted).
static WRITES: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Result<File, Error> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| Error::Store(format!("{}: {error}", self.path.display()))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(File::default()),
            Err(error) => Err(Error::Store(error.to_string())),
        }
    }

    fn write(&self, file: &File) -> Result<(), Error> {
        let bytes =
            serde_json::to_vec_pretty(file).map_err(|error| Error::Store(error.to_string()))?;
        // Write beside and rename, so a crash never leaves half a file.
        let next = self.path.with_extension("json.next");
        std::fs::write(&next, bytes).map_err(|error| Error::Store(error.to_string()))?;
        std::fs::rename(&next, &self.path).map_err(|error| Error::Store(error.to_string()))
    }

    pub fn list(&self) -> Result<Vec<StoredOffer>, Error> {
        Ok(self.read()?.offers)
    }

    pub fn get(&self, id: &str) -> Result<Option<StoredOffer>, Error> {
        Ok(self.read()?.offers.into_iter().find(|offer| offer.id == id))
    }

    /// Adds the offer, or replaces the one with its id.
    pub fn put(&self, offer: StoredOffer) -> Result<(), Error> {
        let _one_at_a_time = WRITES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut file = self.read()?;
        match file.offers.iter_mut().find(|stored| stored.id == offer.id) {
            Some(stored) => *stored = offer,
            None => file.offers.push(offer),
        }
        self.write(&file)
    }

    /// Replaces the offer with its id, if it is still there (a list that followed it on the
    /// chain must not bring back an offer deleted meanwhile).
    pub fn update(&self, offer: StoredOffer) -> Result<bool, Error> {
        let _one_at_a_time = WRITES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut file = self.read()?;
        let Some(stored) = file.offers.iter_mut().find(|stored| stored.id == offer.id) else {
            return Ok(false);
        };
        *stored = offer;
        self.write(&file)?;
        Ok(true)
    }

    pub fn delete(&self, id: &str) -> Result<bool, Error> {
        let _one_at_a_time = WRITES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut file = self.read()?;
        let before = file.offers.len();
        file.offers.retain(|offer| offer.id != id);
        let deleted = file.offers.len() != before;
        if deleted {
            self.write(&file)?;
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_are_kept_replaced_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("partial-offers.json"));
        assert!(store.list().unwrap().is_empty());
        let offer = StoredOffer {
            id: "0x01".to_string(),
            created_at: 1,
            latest: "partial1a".to_string(),
            fills: Vec::new(),
            ended: None,
            cancel: None,
        };
        store.put(offer.clone()).unwrap();
        store
            .put(StoredOffer {
                latest: "partial1b".to_string(),
                ..offer.clone()
            })
            .unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(store.get("0x01").unwrap().unwrap().latest, "partial1b");
        assert!(store.delete("0x01").unwrap());
        assert!(!store.delete("0x01").unwrap());
        // A list that followed it meanwhile does not bring it back.
        assert!(!store.update(offer.clone()).unwrap());
        assert!(store.list().unwrap().is_empty());
    }
}
