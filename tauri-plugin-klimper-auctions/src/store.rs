// SPDX-License-Identifier: AGPL-3.0-or-later
//! The auctions this wallet started or bid on: one JSON file per wallet and network, next to
//! Sage's wallet database (not a table there: Sage numbers its own migrations). Losing it loses
//! no funds: the launcher id is also in the memo of the NFT's send to the auction and in a
//! remark of every bid this wallet makes.

use std::{path::PathBuf, sync::Mutex};

use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredAuction {
    /// The launcher id, `0x` hex.
    pub id: String,
    /// `started` or `bid`.
    pub role: String,
    pub created_at: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct File {
    auctions: Vec<StoredAuction>,
}

static WRITES: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
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
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, bytes).map_err(|error| Error::Store(error.to_string()))?;
        std::fs::rename(&temp, &self.path).map_err(|error| Error::Store(error.to_string()))
    }

    pub fn list(&self) -> Result<Vec<StoredAuction>, Error> {
        let mut auctions = self.read()?.auctions;
        auctions.sort_by_key(|auction| std::cmp::Reverse(auction.created_at));
        Ok(auctions)
    }

    /// Records an auction; one already there keeps its first role.
    pub fn put(&self, auction: StoredAuction) -> Result<(), Error> {
        let _guard = WRITES
            .lock()
            .map_err(|_| Error::Store("lock".to_string()))?;
        let mut file = self.read()?;
        if !file.auctions.iter().any(|stored| stored.id == auction.id) {
            file.auctions.push(auction);
            self.write(&file)?;
        }
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<bool, Error> {
        let _guard = WRITES
            .lock()
            .map_err(|_| Error::Store("lock".to_string()))?;
        let mut file = self.read()?;
        let before = file.auctions.len();
        file.auctions.retain(|stored| stored.id != id);
        let removed = file.auctions.len() != before;
        if removed {
            self.write(&file)?;
        }
        Ok(removed)
    }
}
