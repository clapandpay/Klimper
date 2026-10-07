// SPDX-License-Identifier: AGPL-3.0-or-later
// The Klimper sound, in tiers (clapandpay TASK-111, decision-009).
//
// A real wallet is heard and felt by how much is in it. The tiers are coarse on purpose: the
// sound is for the holder of the phone, not a readout, and four weights are enough to feel
// the difference between a coin and a pouch. Bounds are in US dollars so a tier means the same
// on every network; where there is no price (testnet, or none fetched yet) everything is one
// coin. The files are written by scripts/klimper-sound.py; docs/sound.md quotes this table.

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

/** The bundled file of a tier, as the plugin and the web both resolve it. */
export function soundFileOf(tier: Tier): string {
  return `klimper-${tier}.wav`;
}

/** One haptic impact: when, how hard (0 to 1). The native side maps strength to a style. */
export interface Impact {
  atMs: number;
  strength: number;
}

/**
 * What the hand feels on a shake, per tier: a coin is one light tap; a pouch is a heavy thud
 * followed by a rattle that settles. Timings follow the sound's strikes roughly, not exactly:
 * the hand is slower than the ear.
 */
export const HAPTICS: Readonly<Record<Tier, readonly Impact[]>> = {
  coin: [{ atMs: 0, strength: 0.35 }],
  few: [
    { atMs: 0, strength: 0.45 },
    { atMs: 90, strength: 0.35 },
    { atMs: 210, strength: 0.3 },
  ],
  handful: [
    { atMs: 0, strength: 0.65 },
    { atMs: 70, strength: 0.5 },
    { atMs: 150, strength: 0.5 },
    { atMs: 260, strength: 0.4 },
    { atMs: 400, strength: 0.3 },
  ],
  pouch: [
    { atMs: 0, strength: 1 },
    { atMs: 60, strength: 0.8 },
    { atMs: 130, strength: 0.7 },
    { atMs: 220, strength: 0.6 },
    { atMs: 330, strength: 0.5 },
    { atMs: 470, strength: 0.4 },
    { atMs: 650, strength: 0.3 },
  ],
};
