// SPDX-License-Identifier: Apache-2.0
//! A partial offer: its coin, its terms, and the maker's spends that create the coin (before the
//! first fill). Reading, filling, following and cancelling it.

use chia_wallet_sdk::{
    chia::{
        bls::Signature,
        protocol::{Bytes32, Coin, CoinSpend, SpendBundle},
        puzzle_types::{
            CoinProof, LineageProof, Memos,
            cat::CatSolution,
            offer::{NotarizedPayment, Payment, SettlementPaymentsSolution},
        },
    },
    driver::{Cat, CatInfo, CatSpend, DriverError, Offer, Spend, SpendContext},
    puzzles::SETTLEMENT_PAYMENT_HASH,
    types::run_puzzle,
    types::{
        Condition, Conditions,
        puzzles::{P2OneOfManyArgs, P2OneOfManySolution, RevocationSolution, SettlementPayment},
    },
};
use clvm_traits::{FromClvm, ToClvm, clvm_tuple};
use clvmr::{Allocator, NodePtr, serde::node_from_bytes};

use crate::{
    PARTIAL_HINT_PUZZLE_HASH, PartialOfferError, PartialOfferInfo, PartialSolution, Result,
    decode_partial_offer, encode_partial_offer,
};

#[derive(Debug, Clone)]
pub struct PartialOffer {
    /// The coin that holds what is left: a fill or a cancel spends it.
    pub coin: Coin,
    pub info: PartialOfferInfo,
    /// The maker's spends that create the coin (empty after the first fill), signed.
    pub spend_bundle: SpendBundle,
}

impl PartialOffer {
    pub fn new(parent_coin_id: Bytes32, amount: u64, info: PartialOfferInfo) -> Self {
        Self {
            coin: Coin::new(parent_coin_id, info.puzzle_hash(), amount),
            info,
            spend_bundle: SpendBundle::new(Vec::new(), Signature::default()),
        }
    }

    /// Before the first fill: the offer still carries the maker's spends that create the coin.
    pub fn is_first(&self) -> bool {
        !self.spend_bundle.coin_spends.is_empty()
    }

    /// The coins the maker spends to create the partial coin (none after the first fill).
    pub fn input_coin_ids(&self) -> Vec<Bytes32> {
        self.spend_bundle
            .coin_spends
            .iter()
            .filter(|spend| spend.coin.parent_coin_info != Bytes32::default())
            .map(|spend| spend.coin.coin_id())
            .collect()
    }

    pub fn from_text(text: &str) -> Result<Self> {
        Self::from_spend_bundle(decode_partial_offer(text)?)
    }

    pub fn to_text(&self) -> Result<String> {
        encode_partial_offer(&self.to_spend_bundle()?)
    }

    pub fn from_spend_bundle(spend_bundle: SpendBundle) -> Result<Self> {
        let mut inputs = SpendBundle::new(Vec::new(), spend_bundle.aggregated_signature);
        let mut hint_spend = None;
        for coin_spend in spend_bundle.coin_spends {
            if coin_spend.coin.puzzle_hash == PARTIAL_HINT_PUZZLE_HASH {
                if hint_spend.replace(coin_spend).is_some() {
                    return Err(PartialOfferError::Invalid(
                        "more than one partial hint".to_string(),
                    ));
                }
            } else {
                inputs.coin_spends.push(coin_spend);
            }
        }
        let hint_spend = hint_spend
            .ok_or_else(|| PartialOfferError::NotPartial("no partial hint".to_string()))?;
        let mut allocator = Allocator::new();
        let hint = node_from_bytes(&mut allocator, hint_spend.puzzle_reveal.as_ref())
            .map_err(|error| PartialOfferError::Invalid(error.to_string()))?;
        let info = PartialOfferInfo::from_hint(&allocator, hint)?;
        Ok(Self {
            coin: Coin::new(
                hint_spend.coin.parent_coin_info,
                info.puzzle_hash(),
                hint_spend.coin.amount,
            ),
            info,
            spend_bundle: inputs,
        })
    }

    /// The maker's spends and the hint spend: what the text carries.
    pub fn to_spend_bundle(&self) -> Result<SpendBundle> {
        let mut ctx = SpendContext::new();
        let hint = ctx.alloc(&self.info.to_hint())?;
        let mut spend_bundle = self.spend_bundle.clone();
        spend_bundle.coin_spends.push(CoinSpend::new(
            Coin::new(
                self.coin.parent_coin_info,
                PARTIAL_HINT_PUZZLE_HASH,
                self.coin.amount,
            ),
            ctx.serialize(&hint)?,
            ctx.serialize(&NodePtr::NIL)?,
        ));
        Ok(spend_bundle)
    }

    /// The next partial offer after a fill that left `child_amount`: the same terms, a new
    /// lineage for a CAT, and no maker spends. Everyone who rebuilds it reaches the same text.
    #[must_use]
    pub fn child(&self, child_amount: u64) -> Self {
        Self {
            coin: Coin::new(self.coin.coin_id(), self.coin.puzzle_hash, child_amount),
            info: self
                .info
                .clone()
                .with_lineage_proof(self.info.lineage_proof.map(|_| self.child_lineage_proof())),
            spend_bundle: SpendBundle::new(Vec::new(), Signature::default()),
        }
    }

    /// The lineage proof of any CAT child of the partial coin.
    fn child_lineage_proof(&self) -> LineageProof {
        LineageProof {
            parent_parent_coin_info: self.coin.parent_coin_info,
            parent_inner_puzzle_hash: self.info.cat_inner_puzzle_hash(),
            parent_amount: self.coin.amount,
        }
    }

    /// The maker's notarized payment for a fill of `amount`: what the partial puzzle asserts.
    pub fn notarized_payment(
        &self,
        ctx: &mut SpendContext,
        amount: u64,
    ) -> Result<NotarizedPayment, DriverError> {
        Ok(NotarizedPayment {
            nonce: self.coin.parent_coin_info,
            payments: vec![Payment::new(
                self.info.maker_puzzle_hash,
                amount,
                ctx.hint(self.info.maker_puzzle_hash)?,
            )],
        })
    }

    /// Wraps `inner_solution` (for the `P2_1_OF_N` inner puzzle) in the asset's layers.
    fn spend_partial_coin(
        &self,
        ctx: &mut SpendContext,
        inner_puzzle: NodePtr,
        inner_solution: NodePtr,
    ) -> Result<Spend, DriverError> {
        let inner_solution = if self.info.offered_asset_info.is_revocable() {
            ctx.alloc(&RevocationSolution::new(
                false,
                inner_puzzle,
                inner_solution,
            ))?
        } else {
            inner_solution
        };
        let puzzle =
            PartialOfferInfo::full_puzzle(ctx, self.info.offered_asset_info, inner_puzzle)?;
        let solution = if self.info.offered_asset_info.asset_id.is_some() {
            ctx.alloc(&CatSolution {
                inner_puzzle_solution: inner_solution,
                lineage_proof: self.info.lineage_proof,
                prev_coin_id: self.coin.coin_id(),
                this_coin_info: self.coin,
                next_coin_proof: CoinProof {
                    parent_coin_info: self.coin.parent_coin_info,
                    inner_puzzle_hash: self.info.cat_inner_puzzle_hash(),
                    amount: self.coin.amount,
                },
                prev_subtotal: 0,
                extra_delta: 0,
            })?
        } else {
            inner_solution
        };
        Ok(Spend::new(puzzle, solution))
    }

    /// The fill leaf: pay `other_asset_amount` to the maker, put what it buys in a settlement
    /// coin (`create_coin`), and the rest in the next partial coin.
    pub fn partial_coin_spend<T>(
        &self,
        ctx: &mut SpendContext,
        other_asset_amount: u64,
        create_coin: Option<(Bytes32, u64, T)>,
    ) -> Result<(Spend, NotarizedPayment), DriverError>
    where
        T: ToClvm<Allocator>,
    {
        let args = self.info.to_args(ctx)?;
        let partial_puzzle = ctx.curry(&args)?;
        let partial_puzzle_hash = self.info.partial_puzzle_hash().into();
        let merkle_tree = self.info.merkle_tree();
        let inner_puzzle = ctx.curry(P2OneOfManyArgs::new(merkle_tree.root()))?;
        let inner_solution = ctx.alloc(&P2OneOfManySolution {
            merkle_proof: merkle_tree
                .proof(partial_puzzle_hash)
                .ok_or(DriverError::InvalidMerkleProof)?,
            puzzle: partial_puzzle,
            solution: PartialSolution {
                my_data: CoinProof {
                    parent_coin_info: self.coin.parent_coin_info,
                    inner_puzzle_hash: self.info.inner_puzzle_hash().into(),
                    amount: self.coin.amount,
                },
                other_asset_amount,
                create_coin_rest: create_coin.map(|(puzzle_hash, amount, memos)| {
                    clvm_tuple!(puzzle_hash, clvm_tuple!(amount, memos))
                }),
                cat_maker_solution: (),
                inner_puzzle_solution: (),
            },
        })?;
        Ok((
            self.spend_partial_coin(ctx, inner_puzzle, inner_solution)?,
            self.notarized_payment(ctx, other_asset_amount)?,
        ))
    }

    /// The maker's leaf: spend the partial coin with the maker's own puzzle (`inner_spend`, a
    /// standard spend the maker signs).
    pub fn claw_back(&self, ctx: &mut SpendContext, inner_spend: Spend) -> Result<(), DriverError> {
        let merkle_tree = self.info.merkle_tree();
        let inner_puzzle = ctx.curry(P2OneOfManyArgs::new(merkle_tree.root()))?;
        let inner_solution = ctx.alloc(&P2OneOfManySolution {
            merkle_proof: merkle_tree
                .proof(self.info.maker_puzzle_hash)
                .ok_or(DriverError::InvalidMerkleProof)?,
            puzzle: inner_spend.puzzle,
            solution: inner_spend.solution,
        })?;
        let spend = self.spend_partial_coin(ctx, inner_puzzle, inner_solution)?;
        ctx.spend(self.coin, spend)
    }

    /// The conditions a claw back's maker spend outputs: everything left back to the maker,
    /// hinted so the wallet finds it. For a CAT the coin is created inside the CAT layer.
    pub fn claw_back_conditions(&self, ctx: &mut SpendContext) -> Result<Conditions, DriverError> {
        Ok(Conditions::new().create_coin(
            self.info.maker_puzzle_hash,
            self.coin.amount,
            ctx.hint(self.info.maker_puzzle_hash)?,
        ))
    }

    /// Completes a fill with the taker's offer, which offers `x` of the requested asset (plus
    /// the required fee in XCH) and requests what `x` buys of the offered asset, paid as
    /// [`Self::notarized_payment`] describes. The result is ready to send.
    pub fn accept_offer(self, ctx: &mut SpendContext, offer: Offer) -> Result<SpendBundle> {
        let settlement = ctx.alloc_mod::<SettlementPayment>()?;
        let required_fee = self.info.required_fee.unwrap_or(0);
        let price = self.info.price_data;

        // What the taker pays: a settlement coin of the requested asset, which this fill pays
        // on to the maker.
        let paid = if let Some(requested_asset_id) = self.info.requested_asset_info.asset_id {
            let Some(cat) = offer
                .offered_coins()
                .cats
                .get(&requested_asset_id)
                .and_then(|cats| cats.first().copied())
            else {
                return Err(DriverError::IncompatibleAssetInfo.into());
            };
            let paid = cat.coin.amount;
            let (spend, payment) = self.partial_coin_spend(
                ctx,
                paid,
                Some((
                    SETTLEMENT_PAYMENT_HASH.into(),
                    price.quote(paid),
                    Memos::<NodePtr>::None,
                )),
            )?;
            ctx.spend(self.coin, spend)?;
            let inner = Spend::new(
                settlement,
                ctx.alloc(&SettlementPaymentsSolution {
                    notarized_payments: vec![payment],
                })?,
            );
            let _ = Cat::spend_all(ctx, &[CatSpend::new(cat, inner)])?;
            if required_fee > 0 {
                // The taker's XCH settlement coin, spent to nothing: it becomes the fee.
                let Some(fee_coin) = offer.offered_coins().xch.first().copied() else {
                    return Err(PartialOfferError::Invalid(
                        "the fill does not pay the required fee".to_string(),
                    ));
                };
                let solution = ctx.alloc(&SettlementPaymentsSolution::<NodePtr> {
                    notarized_payments: vec![],
                })?;
                ctx.spend(fee_coin, Spend::new(settlement, solution))?;
            }
            paid
        } else {
            let Some(coin) = offer.offered_coins().xch.first().copied() else {
                return Err(DriverError::IncompatibleAssetInfo.into());
            };
            // One XCH coin pays both the maker and the required fee.
            let paid = coin.amount.checked_sub(required_fee).ok_or_else(|| {
                PartialOfferError::Invalid("the fill does not pay the required fee".to_string())
            })?;
            let (spend, payment) = self.partial_coin_spend(
                ctx,
                paid,
                Some((
                    SETTLEMENT_PAYMENT_HASH.into(),
                    price.quote(paid),
                    Memos::<NodePtr>::None,
                )),
            )?;
            ctx.spend(self.coin, spend)?;
            let solution = ctx.alloc(&SettlementPaymentsSolution {
                notarized_payments: vec![payment],
            })?;
            ctx.spend(coin, Spend::new(settlement, solution))?;
            paid
        };

        if paid < self.info.min_fill_requested() {
            return Err(PartialOfferError::Invalid(format!(
                "a fill of {paid} is below the minimum of {}",
                self.info.min_fill_requested()
            )));
        }
        let bought = price.quote(paid);
        if bought > self.coin.amount {
            return Err(PartialOfferError::Invalid(format!(
                "a fill of {paid} buys {bought}, more than the {} left",
                self.coin.amount
            )));
        }

        // What the taker receives: the settlement coin this fill created, paid to the taker as
        // the taker's offer requested.
        if let Some(offered_asset_id) = self.info.offered_asset_info.asset_id {
            let Some(payments) = offer.requested_payments().cats.get(&offered_asset_id) else {
                return Err(DriverError::IncompatibleAssetInfo.into());
            };
            let inner = Spend::new(
                settlement,
                ctx.alloc(&SettlementPaymentsSolution {
                    notarized_payments: payments.clone(),
                })?,
            );
            let cat = Cat::new(
                Coin::new(
                    self.coin.coin_id(),
                    self.info.settlement_puzzle_hash(),
                    bought,
                ),
                Some(self.child_lineage_proof()),
                CatInfo::new(
                    offered_asset_id,
                    self.info.offered_asset_info.hidden_puzzle_hash,
                    SETTLEMENT_PAYMENT_HASH.into(),
                ),
            );
            let _ = Cat::spend_all(ctx, &[CatSpend::new(cat, inner)])?;
        } else {
            let payments = offer.requested_payments().xch.clone();
            let solution = ctx.alloc(&SettlementPaymentsSolution {
                notarized_payments: payments,
            })?;
            ctx.spend(
                Coin::new(self.coin.coin_id(), SETTLEMENT_PAYMENT_HASH.into(), bought),
                Spend::new(settlement, solution),
            )?;
        }

        let fill = SpendBundle::new(ctx.take(), Signature::default());
        let taken = offer.take(fill);
        Ok(SpendBundle::new(
            [self.spend_bundle.coin_spends, taken.coin_spends].concat(),
            self.spend_bundle.aggregated_signature + &taken.aggregated_signature,
        ))
    }

    /// What the spend of this offer's coin did: a fill (with the next offer if some is left),
    /// or the maker's claw back. `coin_spend` is the coin's spend as the chain recorded it.
    pub fn follow(&self, coin_spend: &CoinSpend) -> Result<PartialSpend> {
        if coin_spend.coin != self.coin {
            return Err(PartialOfferError::Invalid(
                "that spend is of another coin".to_string(),
            ));
        }
        let mut allocator = Allocator::new();
        let puzzle = node_from_bytes(&mut allocator, coin_spend.puzzle_reveal.as_ref())
            .map_err(|error| PartialOfferError::Invalid(error.to_string()))?;
        let solution = node_from_bytes(&mut allocator, coin_spend.solution.as_ref())
            .map_err(|error| PartialOfferError::Invalid(error.to_string()))?;
        let output = run_puzzle(&mut allocator, puzzle, solution).map_err(|error| {
            PartialOfferError::Invalid(format!("the spend does not run: {error}"))
        })?;
        let conditions = Vec::<Condition<NodePtr>>::from_clvm(&allocator, output)
            .map_err(|error| PartialOfferError::Invalid(error.to_string()))?;
        let settlement = self.info.settlement_puzzle_hash();
        let mut bought = None;
        let mut rest = None;
        for condition in conditions {
            if let Some(create_coin) = condition.into_create_coin() {
                if create_coin.puzzle_hash == settlement && bought.is_none() {
                    bought = Some(create_coin.amount);
                } else if create_coin.puzzle_hash == self.coin.puzzle_hash {
                    rest = Some(create_coin.amount);
                }
            }
        }
        Ok(match bought {
            Some(bought) => PartialSpend::Filled {
                bought,
                next: rest
                    .filter(|amount| *amount > 0)
                    .map(|amount| Box::new(self.child(amount))),
            },
            None => PartialSpend::ClawedBack,
        })
    }
}

/// What happened to a partial offer's coin.
#[derive(Debug, Clone)]
pub enum PartialSpend {
    /// A taker bought `bought` of the offered asset; `next` holds what is left, if anything.
    Filled {
        bought: u64,
        next: Option<Box<PartialOffer>>,
    },
    /// The maker took what was left back.
    ClawedBack,
}
