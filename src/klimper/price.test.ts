// SPDX-License-Identifier: AGPL-3.0-or-later
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  fetchPrices,
  lastPrices,
  NODEXCH_PRICES_URL,
  parseNodexch,
  resetPrices,
} from './price';

const CAT = 'a'.repeat(64);

function answers(table: Record<string, unknown | Error>) {
  const calls: string[] = [];
  const fetchFn = vi.fn(async (input: RequestInfo | URL): Promise<Response> => {
    const url = String(input);
    calls.push(url);
    const key = Object.keys(table).find((k) => url.includes(k));
    const body = key === undefined ? new Error('no route') : table[key];
    if (body instanceof Error) throw body;
    return new Response(JSON.stringify(body), { status: 200 });
  });
  return {
    deps: { fetch: fetchFn as unknown as typeof fetch, now: () => 1_000 },
    calls,
  };
}

const nodexch = {
  at: 1,
  xch: { usd: 10, eur: 9 },
  cats: { [CAT]: { xch: 0.5, usd: 5, ask: 0.6 } },
};
const coingecko = { chia: { usd: 12 } };
const dexie = {
  tickers: [
    { base_currency: CAT.toUpperCase(), last_price: '0.25', ask: '0.3' },
  ],
};

describe('prices on mainnet', () => {
  beforeEach(() => {
    resetPrices();
    vi.spyOn(console, 'warn').mockImplementation(() => undefined);
  });

  it('come from nodexch, and nothing else is asked', async () => {
    const { deps, calls } = answers({ 'nodexch.space': nodexch });
    const prices = await fetchPrices(false, deps);
    expect(prices.source).toBe('nodexch');
    expect(prices.xchUsd).toBe(10);
    expect(prices.cats[CAT]).toEqual({ lastPrice: 0.5, askPrice: 0.6 });
    expect(calls).toEqual([NODEXCH_PRICES_URL]);
    expect(lastPrices()).toBe(prices);
    expect(console.warn).not.toHaveBeenCalled();
  });

  it('fall back to CoinGecko and dexie while nodexch does not answer, warning once', async () => {
    const { deps, calls } = answers({
      'nodexch.space': new Error('ECONNREFUSED'),
      coingecko: coingecko,
      'dexie.space': dexie,
    });
    const prices = await fetchPrices(false, deps);
    expect(prices.source).toBe('fallback');
    expect(prices.xchUsd).toBe(12);
    expect(prices.cats[CAT]).toEqual({ lastPrice: 0.25, askPrice: 0.3 });
    expect(calls.some((u) => u.includes('api.dexie.space'))).toBe(true);
    await fetchPrices(false, deps);
    expect(console.warn).toHaveBeenCalledTimes(1);
  });

  it('treat a malformed nodexch answer as no answer', async () => {
    const { deps } = answers({
      'nodexch.space': { xch: { usd: 'ten' }, cats: {} },
      coingecko: coingecko,
      'dexie.space': dexie,
    });
    expect((await fetchPrices(false, deps)).source).toBe('fallback');
    expect(parseNodexch({ xch: { usd: -1 }, cats: {} })).toBeNull();
    expect(parseNodexch(null)).toBeNull();
    // Odd asset ids are skipped, not fatal.
    expect(
      parseNodexch({ xch: { usd: 1 }, cats: { nope: { xch: 1 } } })?.cats,
    ).toEqual({});
  });

  it('keep the last prices when everything fails', async () => {
    const good = answers({ 'nodexch.space': nodexch });
    await fetchPrices(false, good.deps);
    const bad = answers({});
    const prices = await fetchPrices(false, bad.deps);
    expect(prices.xchUsd).toBe(10);
    expect(prices.source).toBe('nodexch');
  });

  it('are zeros when nothing was ever known', async () => {
    const prices = await fetchPrices(false, answers({}).deps);
    expect(prices.xchUsd).toBe(0);
    expect(prices.cats).toEqual({});
  });
});

describe('prices on testnet', () => {
  beforeEach(() => resetPrices());

  it('are zero dollars, with dexie-testnet tickers for the ask prices', async () => {
    const { deps, calls } = answers({ 'api-testnet.dexie.space': dexie });
    const prices = await fetchPrices(true, deps);
    expect(prices.source).toBe('testnet');
    expect(prices.xchUsd).toBe(0);
    expect(prices.cats[CAT]?.askPrice).toBe(0.3);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toContain('api-testnet.dexie.space');
  });
});
