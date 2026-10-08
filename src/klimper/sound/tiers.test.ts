// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from 'vitest';
import { TIER_BOUNDS_USD, tierOf, usdOf } from './tiers';

describe('the tier of an amount', () => {
  it('is a coin below a dollar, and silence without a value or below a cent', () => {
    expect(tierOf(0.99)).toBe('coin');
    expect(tierOf(0.01)).toBe('coin');
    expect(tierOf(0.009)).toBeNull();
    expect(tierOf(0)).toBeNull();
    expect(tierOf(null)).toBeNull();
    expect(tierOf(undefined)).toBeNull();
    expect(tierOf(Number.NaN)).toBeNull();
    expect(tierOf(-5)).toBeNull();
  });

  it('changes exactly at the bounds', () => {
    for (const [tier, bound] of TIER_BOUNDS_USD) {
      expect(tierOf(bound)).toBe(tier);
      expect(tierOf(bound - 0.01)).not.toBe(tier);
    }
    expect(tierOf(1)).toBe('few');
    expect(tierOf(19.99)).toBe('few');
    expect(tierOf(20)).toBe('handful');
    expect(tierOf(199.99)).toBe('handful');
    expect(tierOf(200)).toBe('pouch');
    expect(tierOf(1_000_000)).toBe('pouch');
  });

  it('values mojos at the XCH price, and has no value without one', () => {
    expect(usdOf(1_000_000_000_000n, 10)).toBe(10);
    expect(usdOf(500_000_000_000n, 10)).toBe(5);
    expect(usdOf(1_000_000_000_000n, null)).toBeNull();
    expect(usdOf(1_000_000_000_000n, 0)).toBeNull();
    expect(tierOf(usdOf(25_000_000_000_000n, 10))).toBe('pouch');
  });
});
