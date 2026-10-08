// SPDX-License-Identifier: AGPL-3.0-or-later
// Which coins go into the pouch and how big each one is (TASK-124). Pure.
//
// Size is the coin's amount on a log scale between the smallest and the largest coin: a mojo of
// dust is still a visible coin, and a coin a thousand times bigger is clearly bigger, not a
// thousand times. All discs together fill at most `FILL` of the box, so a pouch never overflows.
// Beyond `MAX_DISCS` coins, the smallest are dust: counted, not drawn (they would be too many
// to see or to simulate, and a pile of dust is the hint to combine it).

import type { CoinRecord } from '@/bindings';
import BigNumber from 'bignumber.js';

export const MAX_DISCS = 120;
/** At most this many tiny coins are selected at once: one combine stays a sensible spend. */
export const DUST_SELECT = 200;
const FILL = 0.42;
const R_MIN = 7;
const R_MAX = 42;

export interface Disc {
  id: string;
  r: number;
  /** In a pending transaction or an offer: shown, not selectable. */
  locked: boolean;
  /** The pending transaction spending it: its coins lean together until it confirms. */
  pending: string | null;
}

export interface Layout {
  discs: Disc[];
  /** Free coins too small to draw, smallest first, at most DUST_SELECT: to select for a combine. */
  dust: string[];
}

export function layoutCoins(
  coins: CoinRecord[],
  width: number,
  height: number,
  uniform: boolean,
): Layout {
  const byAmount = coins
    .slice()
    .sort((a, b) => BigNumber(b.amount).comparedTo(BigNumber(a.amount)) ?? 0);
  const shown = byAmount.slice(0, MAX_DISCS);
  // Dust to select for a combine: the smallest free coins (not in a transaction or an offer,
  // which would make the combine impossible), at most DUST_SELECT of them in one go.
  const dust = byAmount
    .slice(MAX_DISCS)
    .reverse()
    .filter((c) => c.transaction_id === null && c.offer_id === null)
    .slice(0, DUST_SELECT)
    .map((c) => c.coin_id);
  if (shown.length === 0) return { discs: [], dust };

  const logs = shown.map((c) => Math.log10(Math.max(1, Number(c.amount))));
  const hi = Math.max(...logs);
  const lo = Math.min(...logs);
  const radii = logs.map((l) =>
    // Privacy mode: every coin the same, the sizes would tell the amounts.
    uniform || hi === lo
      ? (R_MIN + R_MAX) / 2
      : R_MIN + ((l - lo) / (hi - lo)) * (R_MAX - R_MIN),
  );

  // Fit: scale every radius by the same factor until the coins cover at most FILL of the box.
  const area = radii.reduce((sum, r) => sum + Math.PI * r * r, 0);
  const room = width * height * FILL;
  const scale = Math.min(
    1,
    Math.sqrt(room / area),
    Math.min(width, height) / 2 / R_MAX,
  );

  return {
    discs: shown.map((c, i) => ({
      id: c.coin_id,
      r: Math.max(3, radii[i] * scale),
      locked: c.transaction_id !== null || c.offer_id !== null,
      pending: c.transaction_id,
    })),
    dust,
  };
}

/** A colour per asset that stays the same: Chia green for XCH, a hue from the id otherwise. */
export function assetHue(assetId: string | null): number {
  if (!assetId) return 140;
  let h = 0;
  for (let i = 0; i < assetId.length; i++)
    h = (h * 31 + assetId.charCodeAt(i)) % 360;
  return h;
}
