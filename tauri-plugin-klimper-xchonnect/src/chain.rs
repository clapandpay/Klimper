// SPDX-License-Identifier: AGPL-3.0-or-later
//! Klimper's view of the chain for wallet-kit's optional CHIP-0002 methods (TASK-082):
//! `getAssetCoins`, `getAssetBalance`, `filterUnlockedCoins` and `sendTransaction`.
//!
//! wallet-kit validates the params, bounds them and builds the results; this answers from
//! Sage's database and peers. Every read is scoped to the keys exposed to the dApp (AC #2):
//! a coin is only seen when its p2 puzzle is the standard puzzle of one of those keys,
//! which is exactly what the dApp could read from the chain itself, since the exposed keys
//! are the receive keys it was given at pairing. Coins under any other puzzle (clawback,
//! options, hardened or unexposed keys) are never mentioned.
//!
//! Like the signer, this runs on the request thread and reaches Sage with `block_on`
//! (see `requests::answer`).

use std::collections::HashMap;
use std::future::Future;

use chia_bls::PublicKey;
use chia_protocol::{Bytes32, Program, SpendBundle};
use chia_wallet_sdk::{
    chia::puzzle_types::{Proof, standard::StandardArgs},
    prelude::*,
};
use sage_database::{
    AssetFilter, CoinFilterMode, CoinRow, CoinSortMode, Database, DeserializePrimitive,
};
use sage_wallet::{Status, SyncCommand, Transaction, insert_transaction, submit_to_peers};
use xchonnect_wallet_kit::{
    AssetBalance, AssetKind, ChainData, ChainError, CoinQuery, LineageProof, SpendableCoin,
    TxStatus,
};

use crate::wallet::AppState;

/// Coin rows read per database query while scanning.
const PAGE: u32 = 500;

/// Answers wallet-kit's chain queries from the active Sage wallet.
pub struct KlimperChain {
    state: AppState,
}

impl std::fmt::Debug for KlimperChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KlimperChain").finish_non_exhaustive()
    }
}

impl KlimperChain {
    pub fn new(state: AppState) -> Self {
        KlimperChain { state }
    }

    fn run<T>(work: impl Future<Output = Result<T, ChainError>>) -> Result<T, ChainError> {
        tauri::async_runtime::block_on(work)
    }

    /// The wallet's database, taken under the Sage lock and used after letting go of it.
    async fn db(&self) -> Result<Database, ChainError> {
        let sage = self.state.lock().await;
        let wallet = sage.wallet().map_err(|_| ChainError::Unavailable)?;
        Ok(wallet.db.clone())
    }
}

fn unavailable<E: std::fmt::Display>(error: E) -> ChainError {
    tracing::warn!("xchonnect chain read failed: {error}");
    ChainError::Unavailable
}

/// The standard p2 puzzle hash of each key, so a coin row can be scoped without a
/// database query per coin.
fn owners(keys: &[PublicKey]) -> HashMap<Bytes32, PublicKey> {
    keys.iter()
        .map(|key| (StandardArgs::curry_tree_hash(*key).into(), *key))
        .collect()
}

fn asset_filter(asset: AssetKind) -> AssetFilter {
    match asset {
        AssetKind::Xch => AssetFilter::Id(Bytes32::default()),
        AssetKind::Cat(id) | AssetKind::Did(Some(id)) | AssetKind::Nft(Some(id)) => {
            AssetFilter::Id(id)
        }
        AssetKind::Did(None) => AssetFilter::Dids,
        AssetKind::Nft(None) => AssetFilter::Nfts,
    }
}

/// Pending in a transaction or an offer: Sage's own definition for `WalletConnect`.
fn is_locked(row: &CoinRow) -> bool {
    row.mempool_item_hash.is_some() || row.offer_hash.is_some()
}

/// Walk the coin records of `asset` in Sage's order and hand every row the dApp may see to
/// `visit`, which returns `false` to stop.
async fn scan(
    db: &Database,
    asset: AssetKind,
    mode: CoinFilterMode,
    owners: &HashMap<Bytes32, PublicKey>,
    mut visit: impl FnMut(CoinRow, PublicKey) -> bool,
) -> Result<(), ChainError> {
    let mut offset = 0;
    loop {
        let (rows, total) = db
            .coin_records(
                asset_filter(asset),
                PAGE,
                offset,
                CoinSortMode::CreatedHeight,
                true,
                mode,
            )
            .await
            .map_err(unavailable)?;
        if rows.is_empty() {
            return Ok(());
        }
        offset += u32::try_from(rows.len()).unwrap_or(PAGE);
        for row in rows {
            if let Some(key) = owners.get(&row.p2_puzzle_hash).copied()
                && !visit(row, key)
            {
                return Ok(());
            }
        }
        if offset >= total {
            return Ok(());
        }
    }
}

fn lineage(proof: Proof) -> LineageProof {
    match proof {
        Proof::Eve(p) => LineageProof {
            parent_name: p.parent_parent_coin_info,
            inner_puzzle_hash: None,
            amount: p.parent_amount,
        },
        Proof::Lineage(p) => LineageProof {
            parent_name: p.parent_parent_coin_info,
            inner_puzzle_hash: Some(p.parent_inner_puzzle_hash),
            amount: p.parent_amount,
        },
    }
}

/// The full puzzle of a scoped coin, built as Sage's `WalletConnect` handler builds it.
async fn spendable(
    db: &Database,
    asset: AssetKind,
    row: CoinRow,
    key: PublicKey,
) -> Result<SpendableCoin, ChainError> {
    let mut ctx = SpendContext::new();
    let id = row.coin.coin_id();
    let p2 = StandardLayer::new(key)
        .construct_puzzle(&mut ctx)
        .map_err(unavailable)?;
    let (puzzle, proof) = match asset {
        AssetKind::Xch => (p2, None),
        AssetKind::Cat(_) => {
            let cat = db
                .cat_coin(id)
                .await
                .map_err(unavailable)?
                .ok_or(ChainError::Unavailable)?;
            let puzzle = cat
                .info
                .construct_puzzle(&mut ctx, p2)
                .map_err(unavailable)?;
            (puzzle, cat.lineage_proof.map(Proof::Lineage))
        }
        AssetKind::Did(_) => {
            let did = db
                .did_coin(id)
                .await
                .map_err(unavailable)?
                .ok_or(ChainError::Unavailable)?
                .deserialize(&mut ctx)
                .map_err(unavailable)?;
            let puzzle = did
                .info
                .into_layers(p2)
                .construct_puzzle(&mut ctx)
                .map_err(unavailable)?;
            (puzzle, Some(did.proof))
        }
        AssetKind::Nft(_) => {
            let nft = db
                .nft_coin(id)
                .await
                .map_err(unavailable)?
                .ok_or(ChainError::Unavailable)?
                .deserialize(&mut ctx)
                .map_err(unavailable)?;
            let puzzle = nft
                .info
                .into_layers(p2)
                .construct_puzzle(&mut ctx)
                .map_err(unavailable)?;
            (puzzle, Some(nft.proof))
        }
    };
    let puzzle: Program = ctx.serialize(&puzzle).map_err(unavailable)?;
    Ok(SpendableCoin {
        coin: row.coin,
        puzzle,
        confirmed_block_index: row.created_height.unwrap_or(0),
        locked: is_locked(&row),
        lineage_proof: proof.map(lineage),
    })
}

/// `getAssetCoins` over one wallet database.
async fn read_coins(
    db: &Database,
    query: &CoinQuery<'_>,
) -> Result<Vec<SpendableCoin>, ChainError> {
    let owners = owners(query.keys);
    let mode = if query.include_locked {
        CoinFilterMode::Owned
    } else {
        CoinFilterMode::Selectable
    };
    let (mut skip, limit) = (query.offset, query.limit as usize);
    let mut page = Vec::new();
    if limit > 0 {
        scan(db, query.asset, mode, &owners, |row, key| {
            if skip > 0 {
                skip -= 1;
            } else {
                page.push((row, key));
            }
            page.len() < limit
        })
        .await?;
    }
    let mut out = Vec::with_capacity(page.len());
    for (row, key) in page {
        out.push(spendable(db, query.asset, row, key).await?);
    }
    Ok(out)
}

/// `getAssetBalance` over one wallet database.
async fn read_balance(
    db: &Database,
    asset: AssetKind,
    keys: &[PublicKey],
) -> Result<AssetBalance, ChainError> {
    let mut balance = AssetBalance::default();
    scan(db, asset, CoinFilterMode::Owned, &owners(keys), |row, _| {
        let amount = u128::from(row.coin.amount);
        balance.confirmed += amount;
        if !is_locked(&row) {
            balance.spendable += amount;
            balance.spendable_coin_count += 1;
        }
        true
    })
    .await?;
    Ok(balance)
}

/// `filterUnlockedCoins` over one wallet database.
async fn read_unlocked(
    db: &Database,
    coin_ids: &[Bytes32],
    keys: &[PublicKey],
) -> Result<Vec<Bytes32>, ChainError> {
    let owners = owners(keys);
    // Validated 32-byte ids, encoded here: Sage's queries splice these strings into SQL,
    // so nothing the dApp sent reaches them as text.
    let ids: Vec<String> = coin_ids.iter().map(hex::encode).collect();
    let rows = db.coins_by_ids(&ids).await.map_err(unavailable)?;
    let mut out = Vec::new();
    for row in rows {
        if !owners.contains_key(&row.p2_puzzle_hash) {
            continue;
        }
        let id = row.coin.coin_id();
        if db
            .are_coins_spendable(&[hex::encode(id)])
            .await
            .map_err(unavailable)?
        {
            out.push(id);
        }
    }
    Ok(out)
}

impl ChainData for KlimperChain {
    fn asset_coins(&self, query: &CoinQuery<'_>) -> Result<Vec<SpendableCoin>, ChainError> {
        Self::run(async { read_coins(&self.db().await?, query).await })
    }

    fn asset_balance(
        &self,
        asset: AssetKind,
        keys: &[PublicKey],
    ) -> Result<AssetBalance, ChainError> {
        Self::run(async { read_balance(&self.db().await?, asset, keys).await })
    }

    fn unlocked_coins(
        &self,
        coin_ids: &[Bytes32],
        keys: &[PublicKey],
    ) -> Result<Vec<Bytes32>, ChainError> {
        Self::run(async { read_unlocked(&self.db().await?, coin_ids, keys).await })
    }

    /// What Sage's `send_transaction_immediately` does, but without the wallet lock while
    /// the peers are tried one after another, which can take a while: the rest of the wallet
    /// must not wait on a website's broadcast (TASK-119).
    fn send_transaction(&self, bundle: &SpendBundle) -> Result<TxStatus, ChainError> {
        Self::run(async {
            let (wallet, peer_state, commands) = {
                let sage = self.state.lock().await;
                (
                    sage.wallet().map_err(unavailable)?,
                    sage.peer_state.clone(),
                    sage.command_sender.clone(),
                )
            };
            let peers = peer_state.lock().await.peers();
            match submit_to_peers(&peers, bundle.clone())
                .await
                .map_err(unavailable)?
            {
                Status::Pending => {
                    // Recording it locks again, and only for the wallet it was sent from: if
                    // the user switched wallet or network meanwhile, the new wallet's database
                    // and subscriptions must not get the old one's coins.
                    let sage = self.state.lock().await;
                    let same = sage.wallet().is_ok_and(|now| {
                        now.fingerprint == wallet.fingerprint
                            && now.genesis_challenge == wallet.genesis_challenge
                    });
                    if !same {
                        return Ok(TxStatus {
                            status: 1,
                            error: None,
                        });
                    }
                    let peer = peer_state
                        .lock()
                        .await
                        .acquire_peer()
                        .ok_or_else(|| unavailable("no peers"))?;
                    let subscriptions = insert_transaction(
                        &wallet.db,
                        &peer,
                        wallet.genesis_challenge,
                        bundle.name(),
                        Transaction::from_coin_spends(bundle.coin_spends.clone())
                            .map_err(unavailable)?,
                        bundle.aggregated_signature.clone(),
                    )
                    .await
                    .map_err(unavailable)?;
                    commands
                        .send(SyncCommand::SubscribeCoins {
                            coin_ids: subscriptions,
                        })
                        .await
                        .map_err(unavailable)?;
                    drop(sage);
                    Ok(TxStatus {
                        status: 1,
                        error: None,
                    })
                }
                Status::Failed(status, error) => Ok(TxStatus { status, error }),
                // As Sage answers it: a status no peer gave a verdict on.
                Status::Unknown => Ok(TxStatus {
                    status: 3,
                    error: None,
                }),
            }
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use chia_protocol::{Coin, CoinState};
    use chia_sdk_test::BlsPair;
    use sage_database::Derivation;
    use sqlx::sqlite::SqlitePoolOptions;

    /// A wallet database with Sage's schema, two keys and their XCH coins: `exposed` holds
    /// 1, 2 and 3 mojos, `hidden` holds 100. Only `exposed` is given to the dApp.
    struct Wallet {
        db: Database,
        exposed: PublicKey,
        hidden: PublicKey,
        coins: Vec<Coin>,
    }

    async fn wallet() -> Wallet {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../migrations").run(&pool).await.unwrap();
        let db = Database::new(pool);
        db.run_rust_migrations("TXCH".to_owned()).await.unwrap();

        let (exposed, hidden) = (BlsPair::new(1).pk, BlsPair::new(2).pk);
        let mut coins = Vec::new();
        let mut tx = db.tx().await.unwrap();
        for (index, (key, amounts)) in [(exposed, vec![1, 2, 3]), (hidden, vec![100])]
            .into_iter()
            .enumerate()
        {
            let p2: Bytes32 = StandardArgs::curry_tree_hash(key).into();
            tx.insert_custody_p2_puzzle(
                p2,
                key,
                Derivation {
                    derivation_index: u32::try_from(index).unwrap(),
                    is_hardened: false,
                },
            )
            .await
            .unwrap();
            for (height, amount) in amounts.into_iter().enumerate() {
                let coin = Coin::new(
                    Bytes32::new([u8::try_from(coins.len()).unwrap(); 32]),
                    p2,
                    amount,
                );
                let created = u32::try_from(height).unwrap() + 1;
                tx.insert_height(created).await.unwrap();
                tx.insert_coin(CoinState::new(coin, None, Some(created)))
                    .await
                    .unwrap();
                tx.update_coin(coin.coin_id(), Bytes32::default(), p2)
                    .await
                    .unwrap();
                coins.push(coin);
            }
        }
        tx.commit().await.unwrap();
        Wallet {
            db,
            exposed,
            hidden,
            coins,
        }
    }

    fn query(keys: &[PublicKey], offset: u32, limit: u32) -> CoinQuery<'_> {
        CoinQuery {
            asset: AssetKind::Xch,
            include_locked: false,
            offset,
            limit,
            keys,
        }
    }

    #[tokio::test]
    async fn the_balance_counts_only_coins_of_the_exposed_keys() {
        let w = wallet().await;
        let balance = read_balance(&w.db, AssetKind::Xch, &[w.exposed])
            .await
            .unwrap();
        assert_eq!(
            balance,
            AssetBalance {
                confirmed: 6,
                spendable: 6,
                spendable_coin_count: 3,
            }
        );
        let both = read_balance(&w.db, AssetKind::Xch, &[w.exposed, w.hidden])
            .await
            .unwrap();
        assert_eq!(both.confirmed, 106);
        let none = read_balance(&w.db, AssetKind::Xch, &[]).await.unwrap();
        assert_eq!(none, AssetBalance::default());
    }

    #[tokio::test]
    async fn coins_are_scoped_paged_and_carry_a_puzzle_that_hashes_to_them() {
        let w = wallet().await;
        let keys = [w.exposed];
        let first = read_coins(&w.db, &query(&keys, 0, 2)).await.unwrap();
        let rest = read_coins(&w.db, &query(&keys, 2, 2)).await.unwrap();
        assert_eq!((first.len(), rest.len()), (2, 1));

        let mut seen: Vec<u64> = first.iter().chain(&rest).map(|c| c.coin.amount).collect();
        seen.sort_unstable();
        assert_eq!(
            seen,
            [1, 2, 3],
            "every exposed coin once, the hidden one never"
        );
        for c in first.iter().chain(&rest) {
            let hash = clvm_utils::tree_hash_from_bytes(c.puzzle.as_ref()).unwrap();
            assert_eq!(Bytes32::from(hash), c.coin.puzzle_hash);
            assert!(!c.locked);
            assert!(c.lineage_proof.is_none());
            assert!(c.confirmed_block_index > 0);
        }
        assert!(
            read_coins(&w.db, &query(&keys, 0, 0))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn filter_unlocked_coins_never_confirms_a_coin_outside_the_grant() {
        let w = wallet().await;
        let (mine, theirs) = (w.coins[0].coin_id(), w.coins[3].coin_id());
        let unknown = Bytes32::new([0xee; 32]);
        let out = read_unlocked(&w.db, &[mine, theirs, unknown], &[w.exposed])
            .await
            .unwrap();
        assert_eq!(out, [mine]);
    }
}
