// SPDX-License-Identifier: AGPL-3.0-or-later
//! Two Sage wallets on one simulated chain, for the partial offer tests (TASK-164). A copy of
//! the Xchonnect plugin's harness (`tauri-plugin-klimper-xchonnect/src/execute/sage_sim.rs`,
//! TASK-137), with [`SimWallet::ctx`] added.
//!
//! Adapted from Sage's own test harness (`crates/sage-wallet/src/test.rs`, Apache-2.0,
//! the Sage authors): that one is compiled only inside `sage-wallet`'s own tests, and
//! Klimper never changes Sage's crates (docs/UPSTREAM.md), so the parts these tests use are
//! kept here, on `sage-wallet`'s public API. A real wallet database, a real sync, real
//! spends: an offer made by one wallet is taken by the other and settles on the chain.

use std::{sync::Arc, time::Duration};

use chia_wallet_sdk::{
    chia::{
        bls::{
            DerivableKey, master_to_wallet_hardened_intermediate,
            master_to_wallet_unhardened_intermediate,
        },
        puzzle_types::{DeriveSynthetic, standard::StandardArgs},
    },
    client::Connector,
    prelude::*,
    test::PeerSimulator,
};
use sage_config::TESTNET11;
use sage_database::{Database, Derivation};
use sage_wallet::{
    PeerState, SyncCommand, SyncEvent, SyncManager, SyncOptions, Timeouts, Transaction, Wallet,
    insert_transaction,
};
use sqlx::SqlitePool;
use tokio::{
    sync::{Mutex, mpsc::Receiver, mpsc::Sender},
    time::timeout,
};

static INDEX: Mutex<u32> = Mutex::const_new(0);

pub struct SimWallet {
    pub sim: Arc<PeerSimulator>,
    pub agg_sig: AggSigConstants,
    pub wallet: Arc<Wallet>,
    pub master_sk: SecretKey,
    sender: Sender<SyncCommand>,
    events: Receiver<SyncEvent>,
    index: u32,
    state: Arc<Mutex<PeerState>>,
}

impl SimWallet {
    /// A wallet with `balance` mojos on a chain of its own.
    pub async fn new(balance: u64) -> Self {
        let sim = PeerSimulator::new().await.unwrap();
        Self::with_sim(Arc::new(sim), balance, 0).await
    }

    /// A second wallet with `balance` mojos on the same chain.
    pub async fn next(&self, balance: u64) -> Self {
        Self::with_sim(self.sim.clone(), balance, self.index + 1).await
    }

    async fn with_sim(sim: Arc<PeerSimulator>, balance: u64, key_index: u32) -> Self {
        let db_index = {
            let mut lock = INDEX.lock().await;
            let index = *lock;
            *lock += 1;
            index
        };
        let pool = SqlitePool::connect(&format!(
            "file:klimper-partial-{db_index}?mode=memory&cache=shared"
        ))
        .await
        .unwrap();
        sqlx::migrate!("../migrations").run(&pool).await.unwrap();
        let db = Database::new(pool);
        db.run_rust_migrations("TXCH".to_string()).await.unwrap();

        let sk = BlsPair::default().sk.derive_unhardened(key_index);
        let pk = sk.public_key();
        let intermediate_pk = master_to_wallet_unhardened_intermediate(&pk);
        let intermediate_hardened_sk = master_to_wallet_hardened_intermediate(&sk);

        let mut tx = db.tx().await.unwrap();
        for index in 0..100 {
            let synthetic_key = intermediate_hardened_sk
                .derive_hardened(index)
                .derive_synthetic()
                .public_key();
            let p2_puzzle_hash = StandardArgs::curry_tree_hash(synthetic_key).into();
            tx.insert_custody_p2_puzzle(
                p2_puzzle_hash,
                synthetic_key,
                Derivation {
                    derivation_index: index,
                    is_hardened: true,
                },
            )
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();

        let puzzle_hash =
            StandardArgs::curry_tree_hash(intermediate_pk.derive_unhardened(0).derive_synthetic());
        if balance > 0 {
            sim.lock().await.new_coin(puzzle_hash.into(), balance);
        }

        let agg_sig = AggSigConstants::new(TESTNET11_CONSTANTS.agg_sig_me_additional_data);
        let state = Arc::new(Mutex::new(PeerState::default()));
        let wallet = Arc::new(Wallet::new(
            db,
            pk.get_fingerprint(),
            intermediate_pk,
            TESTNET11_CONSTANTS.genesis_challenge,
            agg_sig,
            None,
        ));
        let (sync_manager, sender, events) = SyncManager::new(
            options(),
            state.clone(),
            Some(wallet.clone()),
            TESTNET11.clone(),
            Connector::Plain,
        );
        tokio::spawn(sync_manager.sync());
        // The simulated chain as the wallet's one peer (Sage's harness adds it directly;
        // `AddPeer` is the public way to the same call).
        let (peer, receiver) = sim.connect_raw().await.unwrap();
        sender
            .send(SyncCommand::AddPeer { peer, receiver })
            .await
            .unwrap();

        let mut wallet = SimWallet {
            sim,
            agg_sig,
            wallet,
            master_sk: sk,
            sender,
            events,
            index: key_index,
            state,
        };
        wallet
            .consume_until(|event| matches!(event, SyncEvent::Subscribed))
            .await;
        assert_eq!(
            wallet.wallet.db.xch_balance().await.unwrap(),
            u128::from(balance)
        );
        wallet
    }

    /// Sign with this wallet's own key and send, as the wallet does for its own spends.
    pub async fn transact(&self, coin_spends: Vec<CoinSpend>) {
        let bundle = self
            .wallet
            .sign_transaction(
                SpendBundle::new(coin_spends, Signature::default()),
                &self.agg_sig,
                self.master_sk.clone(),
                false,
            )
            .await
            .unwrap();
        self.push_bundle(bundle).await.unwrap();
    }

    /// Send a signed bundle to the simulated chain; the error is the chain's refusal.
    pub async fn push_bundle(&self, bundle: SpendBundle) -> anyhow::Result<()> {
        let peer = self.state.lock().await.acquire_peer().expect("no peer");
        let subscriptions = insert_transaction(
            &self.wallet.db,
            &peer,
            TESTNET11_CONSTANTS.genesis_challenge,
            bundle.name(),
            Transaction::from_coin_spends(bundle.coin_spends)?,
            bundle.aggregated_signature,
        )
        .await?;
        self.sender
            .send(SyncCommand::SubscribeCoins {
                coin_ids: subscriptions,
            })
            .await?;
        Ok(())
    }

    /// Have this wallet watch coins someone else created for it. The simulated chain tells
    /// a wallet about new coins only when it asks for them by id, as `push_bundle` does
    /// for the sender's own outputs.
    pub async fn watch_coins(&self, coin_ids: Vec<Bytes32>) {
        self.sender
            .send(SyncCommand::SubscribeCoins { coin_ids })
            .await
            .unwrap();
    }

    async fn consume_until(&mut self, done: impl Fn(SyncEvent) -> bool) {
        loop {
            let next = timeout(Duration::from_secs(10), self.events.recv())
                .await
                .expect("timed out listening for a sync event")
                .expect("the sync stopped");
            if done(next) {
                return;
            }
        }
    }

    /// The wallet's peer: the simulated chain (for reads Sage makes through a peer).
    pub async fn peer(&self) -> sage_wallet::WalletPeer {
        self.state.lock().await.acquire_peer().expect("no peer")
    }

    /// Wait for sync events until `cat`'s balance is `amount` (a CAT someone paid this wallet
    /// is counted once Sage has looked its puzzle up, a few events after the coin arrives).
    pub async fn wait_for_cat_balance(&mut self, cat: Bytes32, amount: u128) {
        for _ in 0..50 {
            if self.wallet.db.cat_balance(cat).await.unwrap() == amount {
                return;
            }
            self.consume_until(|_| true).await;
        }
        panic!("the CAT balance never reached {amount}");
    }

    pub async fn wait_for_coins(&mut self) {
        self.consume_until(|event| matches!(event, SyncEvent::CoinsUpdated))
            .await;
    }
}

fn options() -> SyncOptions {
    SyncOptions {
        target_peers: 0,
        discover_peers: false,
        dns_batch_size: 0,
        connection_batch_size: 0,
        max_peer_age_seconds: 0,
        delta_sync: true,
        puzzle_batch_size_per_peer: 5,
        timeouts: Timeouts {
            sync_delay: Duration::from_millis(100),
            nft_uri_delay: Duration::from_millis(100),
            cat_delay: Duration::from_millis(100),
            puzzle_delay: Duration::from_millis(100),
            transaction_delay: Duration::from_millis(100),
            offer_delay: Duration::from_millis(100),
            ..Default::default()
        },
        testing: true,
    }
}

impl SimWallet {
    /// What the plugin's functions take: this wallet, its key, the simulated chain, its sync.
    pub async fn ctx(&self) -> crate::wallet::WalletCtx {
        crate::wallet::WalletCtx {
            wallet: self.wallet.clone(),
            agg_sig: self.agg_sig,
            master_sk: Some(self.master_sk.clone()),
            peer: Some(self.peer().await),
            sync: Some(self.sender.clone()),
            prefix: "txch".to_string(),
        }
    }
}
