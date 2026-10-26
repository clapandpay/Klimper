// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from 'vitest';
import { isPartialOffer } from './isPartialOffer';
import {
  clampPayment,
  percent,
  quote,
  reverseQuote,
  trimPayment,
} from './math';

// 1,000 CAT mojos for 5 XCH, as in the driver's tests.
const p = { offered: 1_000n, requested: 5_000_000_000_000n };

describe('partial offer price', () => {
  it('quotes as the puzzle does, rounding down', () => {
    expect(quote(p, 100_000_000_000n)).toBe(20n);
    expect(quote(p, 4_999_999_999n)).toBe(0n);
    expect(reverseQuote(p, 20n)).toBe(100_000_000_000n);
  });

  it('trims a payment to the least that buys the same', () => {
    expect(trimPayment(p, 100_000_000_001n)).toBe(100_000_000_000n);
    expect(trimPayment(p, 100_000_000_000n)).toBe(100_000_000_000n);
    const odd = { offered: 7n, requested: 3n };
    for (let paid = 1n; paid < 40n; paid++) {
      const least = trimPayment(odd, paid);
      expect(quote(odd, least)).toBe(quote(odd, paid));
      expect(least <= paid).toBe(true);
    }
  });

  it('keeps a payment between the minimum and the maximum', () => {
    expect(clampPayment(5n, 10n, 100n)).toBe(10n);
    expect(clampPayment(500n, 10n, 100n)).toBe(100n);
    expect(clampPayment(50n, 10n, 100n)).toBe(50n);
    // Less left than the minimum: the rest is all there is.
    expect(clampPayment(50n, 10n, 5n)).toBe(5n);
  });

  it('gives shares in percent', () => {
    expect(percent(250n, 1_000n)).toBe(25);
    expect(percent(1n, 3n)).toBe(33.33);
    expect(percent(1n, 0n)).toBe(0);
  });
});

describe('isPartialOffer', () => {
  it('knows a partial offer by its prefix', () => {
    expect(isPartialOffer('partial1qqqsq9')).toBe(true);
    expect(isPartialOffer('  PARTIAL1QQQSQ9 ')).toBe(true);
    expect(isPartialOffer('offer1qqqsq9')).toBe(false);
    expect(isPartialOffer('partial1 with spaces')).toBe(false);
    expect(isPartialOffer('partial')).toBe(false);
  });
});
