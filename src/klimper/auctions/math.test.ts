// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from 'vitest';
import { bidCost, bpsOf, sellerGets } from './math';

describe('auction arithmetic', () => {
  it('rounds basis points down, as the puzzles do', () => {
    expect(bpsOf(1_000n, 300)).toBe(30n);
    expect(bpsOf(1_099n, 300)).toBe(32n);
    expect(bpsOf(5n, 1)).toBe(0n);
  });

  it('adds the premium and the royalty to a bid', () => {
    expect(bidCost(1_000n, 0, 300)).toBe(1_030n);
    expect(bidCost(1_100n, 250, 300)).toBe(1_100n + 27n + 33n);
  });

  it('takes the commission out of the winning bid', () => {
    expect(sellerGets(1_000n, 100)).toBe(990n);
    expect(sellerGets(1_000n, 0)).toBe(1_000n);
  });
});
