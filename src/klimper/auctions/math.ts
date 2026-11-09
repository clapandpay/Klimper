// SPDX-License-Identifier: AGPL-3.0-or-later
// The auction's arithmetic as its puzzles do it (chia-wallet-sdk's auction): basis points of an
// amount, rounded down.

/** `bps` basis points of `amount`, rounded down. */
export function bpsOf(amount: bigint, bps: number): bigint {
  return (amount * BigInt(bps)) / 10_000n;
}

/** What a bid of `bid` locks up: the bid, the buyer's premium and the NFT's royalty. */
export function bidCost(
  bid: bigint,
  premiumBps: number,
  royaltyBps: number,
): bigint {
  return bid + bpsOf(bid, premiumBps) + bpsOf(bid, royaltyBps);
}

/** What the seller receives of a winning bid: the bid less the commission. */
export function sellerGets(bid: bigint, commissionBps: number): bigint {
  return bid - bpsOf(bid, commissionBps);
}
