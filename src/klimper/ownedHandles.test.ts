// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from 'vitest';
import { handleFromNftName, judge } from './ownedHandles';

const OURS = 'ab'.repeat(32);
const OTHER = 'cd'.repeat(32);
const NOW = 1_800_000_000;

describe('handleFromNftName', () => {
  it('reads the handle NFT name', () => {
    expect(handleFromNftName('@MempoolXCH')).toBe('mempoolxch');
    expect(handleFromNftName('anna')).toBe('anna');
  });
  it('refuses what cannot be a handle', () => {
    expect(handleFromNftName(null)).toBeNull();
    expect(handleFromNftName('@ab')).toBeNull();
    expect(handleFromNftName('@an na')).toBeNull();
    expect(handleFromNftName('Cool NFT #1')).toBeNull();
  });
});

describe('judge', () => {
  it('is active while the slot points at our NFT and has not run out', () => {
    expect(
      judge(
        {
          slot: {
            expiration: NOW + 10,
            owner_launcher_id: OURS,
            resolved_launcher_id: OURS,
          },
        },
        OURS,
        NOW,
      ),
    ).toEqual({ status: 'active', expiration: NOW + 10 });
  });
  it('counts an NFT that only owns the handle', () => {
    expect(
      judge(
        {
          slot: {
            expiration: NOW + 10,
            owner_launcher_id: `0x${OURS}`,
            resolved_launcher_id: OTHER,
          },
        },
        OURS,
        NOW,
      ).status,
    ).toBe('active');
  });
  it('says when it expired', () => {
    expect(
      judge(
        { slot: { expiration: NOW - 1, owner_launcher_id: OURS } },
        OURS,
        NOW,
      ).status,
    ).toBe('expired');
  });
  it('drops a leftover NFT whose name points elsewhere now', () => {
    expect(
      judge(
        {
          slot: {
            expiration: NOW + 10,
            owner_launcher_id: OTHER,
            resolved_launcher_id: OTHER,
          },
        },
        OURS,
        NOW,
      ).status,
    ).toBe('elsewhere');
  });
  it('does not guess from a reply without a slot', () => {
    expect(judge({}, OURS, NOW)).toEqual({
      status: 'unknown',
      expiration: null,
    });
  });
});
