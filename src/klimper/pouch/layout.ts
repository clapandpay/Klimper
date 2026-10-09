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
  /**
   * Locked in an open offer: drawn with a lock, and still selectable. A combine or split of it
   * cancels the offer, so the coin list asks first (`OfferCoinsWarning`).
   */
  offered: boolean;
  /**
   * The pending transaction already spending it: faded, not selectable (a second spend of it
   * could only fail). Its coins lean together until it confirms.
   */
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
  // Dust to select for a combine: the smallest free coins, at most DUST_SELECT of them in one
  // go. Not coins in a transaction (a combine of them could only fail) nor in an offer (one tap
  // must not cancel offers; such a coin is selected on its own, and the list asks first).
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
      offered: c.offer_id !== null,
      pending: c.transaction_id,
    })),
    dust,
  };
}

/** Chia's green: XCH's coins only. */
const XCH_HUE = 140;

/**
 * A colour per asset that stays the same: Chia green for XCH, a hue from the id otherwise,
 * never near the green (a token's id can be ground to land on any hue it likes).
 */
export function assetHue(assetId: string | null): number {
  if (!assetId) return XCH_HUE;
  let h = 0;
  for (let i = 0; i < assetId.length; i++)
    h = (h * 31 + assetId.charCodeAt(i)) % 360;
  return Math.abs(h - XCH_HUE) < 30 ? (h + 90) % 360 : h;
}
