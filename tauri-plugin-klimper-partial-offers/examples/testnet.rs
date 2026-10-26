// SPDX-License-Identifier: AGPL-3.0-or-later
//! Partial offers on testnet11, with two real Sage wallets and the plugin's own code (the same
//! `ops` functions the app's commands call). Not a test: it needs the network and TXCH.
//!
//! ```sh
//! cargo run -p tauri-plugin-klimper-partial-offers --example testnet -- <dir> setup
//! # send a little TXCH to both addresses it prints (0.001 TXCH each is plenty)
//! cargo run -p tauri-plugin-klimper-partial-offers --example testnet -- <dir> run
//! ```
//!
//! `<dir>` keeps the two wallets (`maker/`, `taker/`, each a Sage data directory with fresh,
//! throwaway keys), so `run` can be repeated. `run`: the maker issues a test CAT if it has
//! none, makes a partial offer of 1.000 of it for 0.0001 TXCH (at least a tenth a fill); the
//! taker views it, takes a quarter, then another quarter from the next text; the maker lists
//! it (two fills) and claws back the rest. Every step waits for the chain and checks what it
//! sees. The texts go to `<dir>/offers.txt`.

use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use sage::Sage;
use sage_api::{
    Amount, GenerateMnemonic, GetCats, GetPeers, GetSyncStatus, ImportKey, IssueCat, Login,
    SetNetwork,
};
use tauri_plugin_klimper_partial_offers::{
    AppState, ops,
    records::{MakeRequest, PartialOfferView, Status},
};
use tokio::{sync::Mutex, time::sleep};

const CAT_AMOUNT: u64 = 1_000; // 1.000 of the CAT
const PRICE_MOJOS: u64 = 100_000_000; // 0.0001 TXCH for all of it

async fn open(dir: &Path) -> Result<AppState> {
    std::fs::create_dir_all(dir)?;
    let mut sage = Sage::new(dir, false);
    let mut events = sage.initialize().await?;
    tokio::spawn(async move { while events.recv().await.is_some() {} });
    if sage.network_id() != "testnet11" {
        sage.set_network(SetNetwork {
            name: "testnet11".to_string(),
        })
        .await?;
    }
    let fingerprint = match sage.wallet_config.wallets.first() {
        Some(wallet) => wallet.fingerprint,
        None => {
            let mnemonic = sage
                .generate_mnemonic(GenerateMnemonic { use_24_words: true })?
                .mnemonic;
            sage.import_key(ImportKey {
                name: "partial offers testnet".to_string(),
                key: mnemonic,
                derivation_index: 0,
                hardened: None,
                unhardened: None,
                save_secrets: true,
                login: true,
                emoji: None,
            })
            .await?
            .fingerprint
        }
    };
    sage.login(Login { fingerprint }).await?;
    Ok(Arc::new(Mutex::new(sage)))
}

/// Waits for peers and a finished sync.
async fn synced(name: &str, state: &AppState) -> Result<()> {
    for round in 0..120 {
        let (peers, status) = {
            let sage = state.lock().await;
            (
                sage.get_peers(GetPeers {}).await?.peers.len(),
                // Errors until the wallet has derived its first keys.
                sage.get_sync_status(GetSyncStatus {}).await.ok(),
            )
        };
        if let Some(status) = status {
            if peers > 0 && status.synced_coins >= status.total_coins {
                return Ok(());
            }
            if round % 6 == 0 {
                println!(
                    "  {name}: {peers} peers, {}/{} coins synced",
                    status.synced_coins, status.total_coins
                );
            }
        } else if round % 6 == 0 {
            println!("  {name}: {peers} peers, deriving keys");
        }
        sleep(Duration::from_secs(5)).await;
    }
    bail!("{name}: no sync after ten minutes")
}

async fn xch(state: &AppState) -> Result<u64> {
    let sage = state.lock().await;
    Ok(sage
        .get_sync_status(GetSyncStatus {})
        .await?
        .selectable_balance
        .to_u64()
        .unwrap_or(0))
}

async fn address(state: &AppState) -> Result<String> {
    let sage = state.lock().await;
    Ok(sage
        .get_sync_status(GetSyncStatus {})
        .await?
        .receive_address)
}

async fn cat_balance(state: &AppState, asset_id: &str) -> Result<u64> {
    let sage = state.lock().await;
    Ok(sage
        .get_cats(GetCats {})
        .await?
        .cats
        .iter()
        .find(|cat| cat.asset_id.as_deref() == Some(asset_id))
        .and_then(|cat| cat.balance.to_u64())
        .unwrap_or(0))
}

/// The maker's test CAT: one it holds at least `CAT_AMOUNT` of, or a new one.
async fn test_cat(maker: &AppState) -> Result<String> {
    let held = {
        let sage = maker.lock().await;
        sage.get_cats(GetCats {}).await?.cats
    };
    if let Some(cat) = held
        .iter()
        .find(|cat| cat.balance.to_u64().unwrap_or(0) >= CAT_AMOUNT)
    {
        return Ok(cat.asset_id.clone().context("a CAT without an id")?);
    }
    println!("maker: issuing a test CAT of {CAT_AMOUNT} mojos");
    maker
        .lock()
        .await
        .issue_cat(IssueCat {
            name: "Partial offer test".to_string(),
            ticker: "PART".to_string(),
            amount: Amount::u64(CAT_AMOUNT),
            revocable: false,
            fee: Amount::u64(0),
            auto_submit: true,
        })
        .await?;
    for _ in 0..120 {
        sleep(Duration::from_secs(10)).await;
        let cats = maker.lock().await.get_cats(GetCats {}).await?.cats;
        if let Some(cat) = cats
            .iter()
            .find(|cat| cat.balance.to_u64().unwrap_or(0) >= CAT_AMOUNT)
        {
            return Ok(cat.asset_id.clone().context("a CAT without an id")?);
        }
    }
    bail!("the test CAT never confirmed")
}

/// Follows `text` until `done` holds for what is seen (a fill or a cancel confirming).
async fn until(
    state: &AppState,
    text: &str,
    what: &str,
    done: impl Fn(&PartialOfferView) -> bool,
) -> Result<PartialOfferView> {
    for _ in 0..90 {
        match ops::view(state, text).await {
            Ok(view) if done(&view) => return Ok(view),
            Ok(_) => {}
            Err(error) => println!("  (view: {error})"),
        }
        sleep(Duration::from_secs(10)).await;
    }
    bail!("waited fifteen minutes for {what}")
}

fn log_offer(dir: &Path, label: &str, text: &str) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("offers.txt"))?;
    writeln!(file, "{label}: {text}")?;
    Ok(())
}

async fn run(dir: &Path, maker: &AppState, taker: &AppState) -> Result<()> {
    let asset_id = test_cat(maker).await?;
    println!("maker: CAT {asset_id}");
    let maker_cat_before = cat_balance(maker, &asset_id).await?;
    let taker_cat_before = cat_balance(taker, &asset_id).await?;

    // Make: 1.000 of the CAT for 0.0001 TXCH, at least a tenth a fill, for a day.
    let made = ops::make(
        maker,
        &MakeRequest {
            offered_asset_id: Some(asset_id.clone()),
            offered_amount: CAT_AMOUNT.to_string(),
            requested_asset_id: None,
            requested_amount: PRICE_MOJOS.to_string(),
            min_fill_requested: (PRICE_MOJOS / 10).to_string(),
            required_fee: None,
            expires_at: Some(tauri_plugin_klimper_partial_offers::now() + 86_400),
        },
    )
    .await?;
    log_offer(dir, "made", &made.offer)?;
    println!("maker: made {} ({} chars)", made.id, made.offer.len());

    // View, from the taker's side.
    let seen = ops::view(taker, &made.offer).await?;
    ensure!(
        seen.status == Status::Open,
        "a new offer is {:?}",
        seen.status
    );
    ensure!(!seen.is_mine && seen.first);
    ensure!(seen.remaining_offered == CAT_AMOUNT.to_string());
    println!("taker: sees it open, {} left", seen.remaining_offered);

    // Take a quarter.
    let first = ops::take(taker, &made.offer, &(PRICE_MOJOS / 4).to_string(), "0").await?;
    println!(
        "taker: took {} for {} (tx {})",
        first.bought, first.paid, first.transaction_id
    );
    let next = first.next_offer.context("a quarter left nothing")?;
    log_offer(dir, "after the first fill", &next)?;
    let after = until(taker, &made.offer, "the first fill", |view| {
        view.status == Status::PartlyFilled && view.fills.len() == 1
    })
    .await?;
    ensure!(
        after.offer == next,
        "the chain's next offer is not the one built"
    );
    println!(
        "chain: first fill confirmed, {} left",
        after.remaining_offered
    );

    // Taking from the first text again is refused before signing.
    match ops::take(taker, &made.offer, &(PRICE_MOJOS / 4).to_string(), "0").await {
        Err(tauri_plugin_klimper_partial_offers::Error::OfferChanged) => {
            println!("taker: the stale text is refused (offer_changed)");
        }
        other => bail!("a stale take answered {other:?}"),
    }

    // Another quarter, from the next text.
    let second = ops::take(taker, &next, &(PRICE_MOJOS / 4).to_string(), "0").await?;
    println!("taker: took {} for {}", second.bought, second.paid);
    let rest = second.next_offer.context("half left nothing")?;
    log_offer(dir, "after the second fill", &rest)?;
    until(taker, &made.offer, "the second fill", |view| {
        view.fills.len() == 2
    })
    .await?;
    println!("chain: second fill confirmed");

    // The maker's list, then the claw back.
    let mine = ops::list(maker).await?;
    let listed = mine
        .iter()
        .find(|offer| offer.id == made.id)
        .context("the offer is not in the maker's list")?;
    ensure!(
        listed.fills.len() == 2,
        "the list has {} fills",
        listed.fills.len()
    );
    ensure!(listed.view.status == Status::PartlyFilled);
    println!(
        "maker: lists it with {} fills, {} left",
        listed.fills.len(),
        listed.view.remaining_offered
    );
    let cancelled = ops::cancel(maker, &made.id, "0").await?;
    println!("maker: clawing back (tx {})", cancelled.transaction_id);
    until(maker, &made.offer, "the claw back", |view| {
        view.status == Status::Cancelled
    })
    .await?;
    println!("chain: claw back confirmed");

    // Balances: the taker got half the CAT, the maker the rest back.
    for _ in 0..30 {
        let taker_cat = cat_balance(taker, &asset_id).await?;
        let maker_cat = cat_balance(maker, &asset_id).await?;
        if taker_cat == taker_cat_before + CAT_AMOUNT / 2
            && maker_cat == maker_cat_before - CAT_AMOUNT / 2
        {
            println!(
                "balances: taker {taker_cat} (+{}), maker {maker_cat} (-{})",
                CAT_AMOUNT / 2,
                CAT_AMOUNT / 2
            );
            println!("partial offers on testnet11: everything checked");
            return Ok(());
        }
        sleep(Duration::from_secs(10)).await;
    }
    bail!("the CAT balances never settled")
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().context("usage: testnet <dir> setup|run")?);
    let step = args.next().unwrap_or_else(|| "setup".to_string());

    let maker = open(&dir.join("maker")).await?;
    let taker = open(&dir.join("taker")).await?;
    synced("maker", &maker).await?;
    synced("taker", &taker).await?;
    println!(
        "maker: {} ({} mojos)",
        address(&maker).await?,
        xch(&maker).await?
    );
    println!(
        "taker: {} ({} mojos)",
        address(&taker).await?,
        xch(&taker).await?
    );

    match step.as_str() {
        "setup" => Ok(()),
        "run" => {
            ensure!(xch(&maker).await? > CAT_AMOUNT, "fund the maker first");
            ensure!(xch(&taker).await? > PRICE_MOJOS, "fund the taker first");
            run(&dir, &maker, &taker).await
        }
        other => bail!("unknown step {other}"),
    }
}
