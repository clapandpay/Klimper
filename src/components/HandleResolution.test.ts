// SPDX-License-Identifier: AGPL-3.0-or-later
// The re-check at submit: only the address the user saw on screen is ever sent to.

import { i18n } from '@lingui/core';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { recheckHandle } from './HandleResolution';

i18n.loadAndActivate({ locale: 'en-US', messages: {} });

const PH = 'ab'.repeat(32);

function registry(puzzleHash: string) {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          handle: 'anna',
          slot: { expiration: Date.now() / 1000 + 86_400 },
          resolved_singleton: {
            melted: false,
            nft: { p2_puzzle_hash: puzzleHash },
          },
          pending_transfer: null,
        }),
      ),
    ),
  );
}

afterEach(() => vi.unstubAllGlobals());

describe('recheckHandle', () => {
  it('passes an address through untouched', async () => {
    expect(await recheckHandle('xch1abc', 'xch', () => null)).toEqual({
      address: 'xch1abc',
    });
  });

  it('refuses when the handle now points elsewhere than shown', async () => {
    registry(PH);
    const checked = await recheckHandle(
      '@anna',
      'xch',
      () => 'xch1somewhereelse',
    );
    expect(checked).toHaveProperty('error');
  });

  it('refuses before any address was shown', async () => {
    registry(PH);
    const checked = await recheckHandle('@anna', 'xch', () => null);
    expect(checked).toHaveProperty('error');
  });

  it('accepts when the resolved address is the one shown', async () => {
    registry(PH);
    const { toAddress } = await import('@/lib/utils');
    const address = toAddress(PH, 'xch');
    expect(await recheckHandle('@anna', 'xch', () => address)).toEqual({
      address,
    });
  });

  it('counts an address that appeared while the registry was asked', async () => {
    const { toAddress } = await import('@/lib/utils');
    const address = toAddress(PH, 'xch');
    let shown: string | null = null;
    registry(PH);
    const answer = vi.mocked(fetch).getMockImplementation();
    vi.mocked(fetch).mockImplementation(async (...args) => {
      shown = address; // the live lookup lands first
      return answer ? answer(...args) : new Response('{}');
    });
    expect(await recheckHandle('@anna', 'xch', () => shown)).toEqual({
      address,
    });
  });
});
