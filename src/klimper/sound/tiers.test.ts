// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from 'vitest';
import {
  HAPTICS,
  TIER_BOUNDS_USD,
  TIERS,
  soundFileOf,
  tierOf,
  usdOf,
} from './tiers';

describe('the tier of an amount', () => {
  it('is a coin below a dollar and when there is no price', () => {
    expect(tierOf(0.99)).toBe('coin');
    expect(tierOf(0)).toBe('coin');
    expect(tierOf(null)).toBe('coin');
    expect(tierOf(undefined)).toBe('coin');
    expect(tierOf(Number.NaN)).toBe('coin');
    expect(tierOf(-5)).toBe('coin');
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

describe('the files and the haptics', () => {
  it('have one entry per tier, in ascending weight', () => {
    expect(TIERS).toEqual(['coin', 'few', 'handful', 'pouch']);
    for (const tier of TIERS) {
      expect(soundFileOf(tier)).toBe(`klimper-${tier}.wav`);
      const impacts = HAPTICS[tier];
      expect(impacts.length).toBeGreaterThan(0);
      expect(impacts[0].atMs).toBe(0);
      for (const { atMs, strength } of impacts) {
        expect(atMs).toBeGreaterThanOrEqual(0);
        expect(strength).toBeGreaterThan(0);
        expect(strength).toBeLessThanOrEqual(1);
      }
    }
    // Heavier tiers hit more and harder.
    const weight = (tier: (typeof TIERS)[number]) =>
      HAPTICS[tier].reduce((sum, i) => sum + i.strength, 0);
    expect(weight('coin')).toBeLessThan(weight('few'));
    expect(weight('few')).toBeLessThan(weight('handful'));
    expect(weight('handful')).toBeLessThan(weight('pouch'));
  });
});
