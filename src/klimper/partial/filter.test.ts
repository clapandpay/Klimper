// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from 'vitest';
import { PartialStatus } from './api';
import { matchesOfferFilter } from './filter';

const ALL: PartialStatus[] = [
  'open',
  'partly_filled',
  'filled',
  'cancelled',
  'expired',
  'unknown',
  'cancelling',
];
const shown = (filter: string) =>
  ALL.filter((status) => matchesOfferFilter(status, filter));

describe('the offer filter on partial offers', () => {
  it('shows everything for all', () => {
    expect(shown('all')).toEqual(ALL);
  });

  it('counts a pending cancel as active until the chain confirms it', () => {
    expect(shown('active')).toEqual([
      'open',
      'partly_filled',
      'unknown',
      'cancelling',
    ]);
  });

  it('maps completed, cancelled and expired', () => {
    expect(shown('completed')).toEqual(['filled']);
    expect(shown('cancelled')).toEqual(['cancelled']);
    expect(shown('expired')).toEqual(['expired']);
  });
});
