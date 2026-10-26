// SPDX-License-Identifier: AGPL-3.0-or-later
// A partial offer's price, computed as the puzzle computes it: integers, rounding down. The
// plugin checks every fill again; this is for the screens (the quote shown while choosing).

export interface Price {
  /** The maker's initial offered amount, mojos. */
  offered: bigint;
  /** The maker's initial requested amount, mojos. */
  requested: bigint;
}

export function price(view: {
  initial_offered: string;
  initial_requested: string;
}): Price {
  return {
    offered: BigInt(view.initial_offered),
    requested: BigInt(view.initial_requested),
  };
}

/** What paying `paid` of the requested asset buys of the offered asset. */
export function quote(p: Price, paid: bigint): bigint {
  if (p.requested === 0n) return 0n;
  return (paid * p.offered) / p.requested;
}

/** What buying `bought` of the offered asset costs at least. */
export function reverseQuote(p: Price, bought: bigint): bigint {
  if (p.offered === 0n) return 0n;
  return (bought * p.requested) / p.offered;
}

/** The least payment that buys as much as `paid` does (more buys nothing extra). */
export function trimPayment(p: Price, paid: bigint): bigint {
  const receives = quote(p, paid);
  let least = reverseQuote(p, receives);
  if (quote(p, least) < receives) least += 1n;
  return least < paid ? least : paid;
}

/** `paid` kept between the least and the most one fill may pay. */
export function clampPayment(paid: bigint, min: bigint, max: bigint): bigint {
  if (max < min) return max;
  if (paid < min) return min;
  if (paid > max) return max;
  return paid;
}

/** The share of `part` in `whole`, 0 to 100. */
export function percent(part: bigint, whole: bigint): number {
  if (whole <= 0n) return 0;
  return Number((part * 10_000n) / whole) / 100;
}
