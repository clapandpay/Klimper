// SPDX-License-Identifier: AGPL-3.0-or-later
// The Klimper sound, in tiers (clapandpay TASK-111, decision-009).
//
// A real wallet is heard and felt by how much is in it. The tiers are coarse on purpose: the
// sound is for the holder of the phone, not a readout, and four weights are enough to feel
// the difference between a coin and a pouch. Bounds are in US dollars so a tier means the same
// on every network; where there is no price (testnet, or none fetched yet) everything is one
// coin. Two sounds per tier, written by scripts/klimper-sound.py: the ding (coins arrive) and
// the rattle (the phone is shaken). docs/sound.md quotes this table.

export type Tier = 'coin' | 'few' | 'handful' | 'pouch';

/** Lower bound in USD of each tier above the first, ascending. */
export const TIER_BOUNDS_USD: readonly (readonly [Tier, number])[] = [
  ['few', 1],
  ['handful', 20],
  ['pouch', 200],
];

export const TIERS: readonly Tier[] = ['coin', 'few', 'handful', 'pouch'];

/**
 * The tier of an amount of money. `usd` is the value of the coins involved: what was received,
 * or the whole wallet for a shake. `null` or anything not a positive finite number is a coin.
 */
export function tierOf(usd: number | null | undefined): Tier {
  if (usd === null || usd === undefined || !Number.isFinite(usd) || usd <= 0) {
    return 'coin';
  }
  let tier: Tier = 'coin';
  for (const [name, bound] of TIER_BOUNDS_USD) {
    if (usd >= bound) tier = name;
  }
  return tier;
}

/** Value in USD of an amount in mojos at a price per XCH; null when there is no price. */
export function usdOf(
  mojos: bigint,
  xchPriceUsd: number | null,
): number | null {
  if (
    xchPriceUsd === null ||
    !Number.isFinite(xchPriceUsd) ||
    xchPriceUsd <= 0
  ) {
    return null;
  }
  // 1 XCH = 10^12 mojos. Two-step division keeps the result exact enough for tier bounds.
  return (Number(mojos / 1_000_000n) / 1_000_000) * xchPriceUsd;
}

/** The two bundled files of a tier, as the plugin and the web both name them. */
export function soundFilesOf(tier: Tier): { ding: string; rattle: string } {
  return {
    ding: `klimper-ding-${tier}.wav`,
    rattle: `klimper-rattle-${tier}.wav`,
  };
}
