// SPDX-License-Identifier: AGPL-3.0-or-later
import { OfferState } from '@/state';
import { describe, expect, it } from 'vitest';

import { partialPair } from './pair';

const state = (
  offered: string | null,
  requested: string | null,
): OfferState => ({
  fee: '',
  expiration: null,
  offered: {
    tokens: [{ asset_id: offered, amount: '5' }],
    nfts: [],
    options: [],
  },
  requested: {
    tokens: [{ asset_id: requested, amount: '1' }],
    nfts: [],
    options: [],
  },
});

describe('partialPair', () => {
  it('takes one token for one other token', () => {
    expect(partialPair(state('a'.repeat(64), null))).not.toBeNull();
  });

  it('refuses a token row without an asset picked yet', () => {
    expect(partialPair(state('', null))).toBeNull();
    expect(partialPair(state(null, ''))).toBeNull();
  });

  it('refuses an asset for itself', () => {
    expect(partialPair(state(null, null))).toBeNull();
  });
});
