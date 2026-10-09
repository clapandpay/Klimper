use chia_wallet_sdk::prelude::*;

use crate::WalletError;

use super::Wallet;

impl Wallet {
    pub async fn combine(
        &self,
        selected_coin_ids: Vec<Bytes32>,
        fee: u64,
    ) -> Result<Vec<CoinSpend>, WalletError> {
        let mut ctx = SpendContext::new();

        self.spend(&mut ctx, selected_coin_ids, &[Action::fee(fee)])
            .await?;

        Ok(ctx.take())
    }

    pub async fn split(
        &self,
        selected_coin_ids: Vec<Bytes32>,
        output_count: usize,
        fee: u64,
    ) -> Result<Vec<CoinSpend>, WalletError> {
        let mut ctx = SpendContext::new();

        let mut spends = self
            .prepare_spends_for_selection(&mut ctx, &selected_coin_ids)
            .await?;

        let asset_id = spends
            .cats
            .values()
            .next()
            .and_then(|cat| cat.items.first().map(|item| item.asset.info.asset_id));

        let mut actions = vec![Action::fee(fee)];

        let total = if let Some(asset_id) = asset_id {
            self.select_spends(&mut ctx, &mut spends, &actions).await?;
            spends.cats[&Id::Existing(asset_id)].selected_amount()
        } else {
            let total = spends.xch.selected_amount();
            if fee > total {
                return Err(WalletError::InsufficientFunds);
            }
            total
        };

        let mut remaining_count = output_count;
        let mut remaining_amount = total - if asset_id.is_none() { fee } else { 0 };

        let max_individual_amount = remaining_amount.div_ceil(output_count as u64);
        let derivations_needed = output_count.div_ceil(selected_coin_ids.len()) as u32;

        let puzzle_hashes = if let Some(change_p2_puzzle_hash) = self.change_p2_puzzle_hash {
            [change_p2_puzzle_hash].repeat(derivations_needed as usize)
        } else {
            self.p2_puzzle_hashes(derivations_needed, false, true)
                .await?
        };

        for &puzzle_hash in &puzzle_hashes {
            for _ in 0..selected_coin_ids.len() {
                if remaining_count == 0 {
                    break;
                }

                let amount = max_individual_amount.min(remaining_amount);
                remaining_amount -= amount;
                remaining_count -= 1;

                actions.push(Action::send(
                    asset_id.map_or(Id::Xch, Id::Existing),
                    puzzle_hash,
                    amount,
                    if asset_id.is_some() {
                        ctx.hint(puzzle_hash)?
                    } else {
                        Memos::None
                    },
                ));
            }
        }

        let deltas = spends.apply(&mut ctx, &actions)?;
        self.complete_spends(&mut ctx, &deltas, spends).await?;

        Ok(ctx.take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::TestWallet;

    #[test(tokio::test)]
    async fn test_xch_coin_management() -> anyhow::Result<()> {
        let mut test = TestWallet::new(1000).await?;

        let coins = test.wallet.db.selectable_xch_coins().await?;
        let coin_spends = test
            .wallet
            .split(coins.iter().map(Coin::coin_id).collect(), 3, 0)
            .await?;
        assert_eq!(coin_spends.len(), 1);

        test.transact(coin_spends).await?;
        test.wait_for_coins().await;

        assert_eq!(test.wallet.db.xch_balance().await?, 1000);
        assert_eq!(test.wallet.db.selectable_xch_coins().await?.len(), 3);

        let coins = test.wallet.db.selectable_xch_coins().await?;
        let coin_spends = test
            .wallet
            .combine(coins.iter().map(Coin::coin_id).collect(), 0)
            .await?;
        assert_eq!(coin_spends.len(), 3);

        test.transact(coin_spends).await?;
        test.wait_for_coins().await;

        assert_eq!(test.wallet.db.xch_balance().await?, 1000);
        assert_eq!(test.wallet.db.selectable_xch_coins().await?.len(), 1);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_cat_coin_management() -> anyhow::Result<()> {
        let mut test = TestWallet::new(100).await?;

        let (coin_spends, asset_id) = test.wallet.issue_cat(100, 0, None).await?;
        test.transact(coin_spends).await?;
        test.wait_for_coins().await;

        let mut cats = test.wallet.db.selectable_cat_coins(asset_id).await?;
        assert_eq!(test.wallet.db.cat_balance(asset_id).await?, 100);
        assert_eq!(cats.len(), 1);

        let cat = cats.remove(0);
        let coin_spends = test.wallet.split(vec![cat.coin.coin_id()], 2, 0).await?;
        test.transact(coin_spends).await?;
        test.wait_for_coins().await;

        let cats = test.wallet.db.selectable_cat_coins(asset_id).await?;
        assert_eq!(test.wallet.db.cat_balance(asset_id).await?, 100);
        assert_eq!(cats.len(), 2);

        let coin_spends = test
            .wallet
            .combine(cats.iter().map(|cat| cat.coin.coin_id()).collect(), 0)
            .await?;
        test.transact(coin_spends).await?;
        test.wait_for_coins().await;

        assert_eq!(test.wallet.db.cat_balance(asset_id).await?, 100);
        assert_eq!(
            test.wallet.db.selectable_cat_coins(asset_id).await?.len(),
            1
        );

        Ok(())
    }

    /// Klimper (TASK-124): a coin locked in an open offer can still be combined. The combine is
    /// the offer's cancellation (Sage cancels an offer the same way, by spending one of its
    /// coins back to the wallet): the offer is marked cancelled, its coins are free again, and
    /// the combined coin is an ordinary, selectable coin.
    #[test(tokio::test)]
    async fn test_combine_coins_locked_in_an_offer() -> anyhow::Result<()> {
        use sage_database::OfferStatus;

        use crate::{Offered, Requested};

        let mut test = TestWallet::new(1000).await?;

        let coins = test.wallet.db.selectable_xch_coins().await?;
        let coin_spends = test
            .wallet
            .split(coins.iter().map(Coin::coin_id).collect(), 2, 0)
            .await?;
        test.transact(coin_spends).await?;
        test.wait_for_coins().await;

        // Lock one coin in an offer, the way `import_offer` does.
        let coins = test.wallet.db.selectable_xch_coins().await?;
        assert_eq!(coins.len(), 2);
        let locked = coins[0].coin_id();
        let offer = test
            .wallet
            .make_offer(
                Offered {
                    xch: 100,
                    selected_coin_ids: vec![locked],
                    ..Default::default()
                },
                Requested {
                    xch: 1,
                    ..Default::default()
                },
                None,
            )
            .await?;
        let offer = test
            .wallet
            .sign_transaction(offer, &test.agg_sig, test.master_sk.clone(), true)
            .await?;
        let offer_id = offer.name();
        let offer_coin_ids = Offer::from_spend_bundle(&mut SpendContext::new(), &offer)?
            .cancellable_coin_spends()?
            .into_iter()
            .map(|cs| cs.coin.coin_id())
            .collect::<Vec<_>>();
        assert_eq!(offer_coin_ids, vec![locked]);

        let mut tx = test.wallet.db.tx().await?;
        tx.insert_offer(sage_database::OfferRow {
            offer_id,
            encoded_offer: chia_wallet_sdk::driver::encode_offer(&offer)?,
            expiration_height: None,
            expiration_timestamp: None,
            fee: 0,
            status: OfferStatus::Active,
            inserted_timestamp: 0,
        })
        .await?;
        tx.insert_offered_coin(offer_id, locked).await?;
        tx.commit().await?;

        // Locked: never picked by a send, but still spendable when chosen on purpose.
        let selectable = test.wallet.db.selectable_xch_coins().await?;
        assert_eq!(selectable.len(), 1);
        assert!(selectable.iter().all(|coin| coin.coin_id() != locked));
        let ids = coins
            .iter()
            .map(|coin| hex::encode(coin.coin_id()))
            .collect::<Vec<_>>();
        assert!(test.wallet.db.are_coins_spendable(&ids).await?);

        let coin_spends = test
            .wallet
            .combine(coins.iter().map(Coin::coin_id).collect(), 0)
            .await?;
        test.transact(coin_spends).await?;
        test.wait_for_coins().await;

        // The offer is cancelled once its coin is spent (the offer queue looks every 100 ms in
        // tests; its event may come before or after the coins'), and nothing stays locked.
        let mut status = None;
        for _ in 0..100 {
            status = test.wallet.db.offer(offer_id).await?.map(|row| row.status);
            if status == Some(OfferStatus::Cancelled) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(status, Some(OfferStatus::Cancelled));
        assert_eq!(test.wallet.db.xch_balance().await?, 1000);
        let selectable = test.wallet.db.selectable_xch_coins().await?;
        assert_eq!(selectable.len(), 1);
        assert_eq!(selectable[0].amount, 1000);

        Ok(())
    }
}
