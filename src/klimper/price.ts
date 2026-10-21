// SPDX-License-Identifier: AGPL-3.0-or-later
// Where Klimper's prices come from (clapandpay TASK-113, owner's decision of 2026-10-07).
//
// One module fetches prices; nothing else does. Mainnet asks nodexch.com for everything
// (XCH in USD and every CAT's price; TASK-112 defines `GET /api/prices`), so phones do not
// call third parties directly, and falls back to what Sage did before (CoinGecko for XCH,
// dexie's tickers for CATs) while that endpoint is not answering. Testnet coins are worth
// nothing: TXCH is 0 USD, and dexie-testnet's tickers give CAT prices in TXCH, so every
// dollar value is 0 (the Klimper sound is silent there by its 0-dollar rule).
//
// `PriceContext` calls `fetchPrices` once a minute; `lastPrices` lets code outside React (the
// sound) read what it fetched without a request of its own.

import { dexieApiUrl } from '@/lib/urls';

/** A CAT's price in XCH, as Sage's `PriceContext` keeps it. */
export interface CatPrice {
  lastPrice: number | null;
  askPrice: number | null;
}

export interface Prices {
  xchUsd: number;
  cats: Record<string, CatPrice>;
  source: 'nodexch' | 'fallback' | 'testnet';
  /** Unix milliseconds of the fetch. */
  at: number;
}

export const NODEXCH_PRICES_URL = 'https://nodexch.com/api/prices';
// nodexch's first domain (nodexch TASK-184): asked second while it still answers and the new one may not.
export const NODEXCH_OLD_PRICES_URL = 'https://nodexch.space/api/prices';
const COINGECKO_URL =
  'https://api.coingecko.com/api/v3/simple/price?ids=chia&vs_currencies=usd';
const PRICE_TIMEOUT_MS = 5000;

interface PriceDeps {
  fetch: typeof fetch;
  now: () => number;
}

let last: Prices | null = null;
let warned = false;

/** The prices of the last successful fetch, or null before the first one. */
export function lastPrices(): Prices | null {
  return last;
}

/** Tests only. */
export function resetPrices() {
  last = null;
  warned = false;
}

function positive(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value > 0
    ? value
    : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

async function getJson(
  deps: PriceDeps,
  url: string,
  init?: RequestInit,
): Promise<unknown> {
  const response = await deps.fetch(url, {
    ...init,
    headers: { accept: 'application/json', ...(init?.headers ?? {}) },
    signal: AbortSignal.timeout(PRICE_TIMEOUT_MS),
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return response.json();
}

/**
 * nodexch's `/api/prices` (TASK-112): `{at, xch: {usd, eur}, cats: {<asset_id>: {xch, usd,
 * ask?}}}`. Anything malformed is a failure, and the caller falls back.
 */
export function parseNodexch(
  body: unknown,
): Omit<Prices, 'at' | 'source'> | null {
  if (!isRecord(body) || !isRecord(body.xch) || !isRecord(body.cats)) {
    return null;
  }
  const xchUsd = positive(body.xch.usd);
  if (xchUsd === null) return null;
  const cats: Record<string, CatPrice> = {};
  for (const [assetId, entry] of Object.entries(body.cats)) {
    if (!isRecord(entry) || !/^[0-9a-f]{64}$/i.test(assetId)) continue;
    cats[assetId.toLowerCase()] = {
      lastPrice: positive(entry.xch),
      askPrice: positive(entry.ask),
    };
  }
  return { xchUsd, cats };
}

/** dexie's `v3/prices/tickers`, as Sage read it. */
function parseDexie(body: unknown): Record<string, CatPrice> {
  const cats: Record<string, CatPrice> = {};
  if (!isRecord(body) || !Array.isArray(body.tickers)) return cats;
  for (const ticker of body.tickers) {
    if (!isRecord(ticker) || typeof ticker.base_currency !== 'string') continue;
    cats[ticker.base_currency.toLowerCase()] = {
      lastPrice: positive(Number(ticker.last_price)),
      askPrice: positive(Number(ticker.ask)),
    };
  }
  return cats;
}

function warnOnce(what: string, error: unknown) {
  if (warned) return;
  warned = true;
  console.warn(`klimper prices: ${what}:`, error);
}

/**
 * Fetch the prices for the network. Never throws: a network that answers nothing gives the
 * last known prices, or zeros when there are none.
 */
export async function fetchPrices(
  isTestnet: boolean,
  deps: PriceDeps = { fetch: (...a) => fetch(...a), now: Date.now },
): Promise<Prices> {
  let prices: Prices | null = null;
  if (isTestnet) {
    // Nothing on testnet is worth a dollar. CAT tickers are kept for the ask prices Sage shows.
    let cats: Record<string, CatPrice> = {};
    try {
      cats = parseDexie(
        await getJson(deps, dexieApiUrl('v3/prices/tickers', true)),
      );
    } catch (error) {
      warnOnce('dexie testnet', error);
    }
    prices = { xchUsd: 0, cats, source: 'testnet', at: deps.now() };
  } else {
    prices = (await fromNodexch(deps)) ?? (await fallback(deps));
  }
  if (prices !== null) last = prices;
  return (
    prices ??
    last ?? { xchUsd: 0, cats: {}, source: 'fallback', at: deps.now() }
  );
}

async function fromNodexch(deps: PriceDeps): Promise<Prices | null> {
  let failure: unknown;
  for (const url of [NODEXCH_PRICES_URL, NODEXCH_OLD_PRICES_URL]) {
    try {
      const parsed = parseNodexch(await getJson(deps, url));
      if (parsed !== null) {
        return { ...parsed, source: 'nodexch', at: deps.now() };
      }
      failure = new Error('malformed answer');
    } catch (error) {
      failure = error;
    }
  }
  warnOnce('nodexch not answering, using CoinGecko and dexie', failure);
  return null;
}

async function fallback(deps: PriceDeps): Promise<Prices | null> {
  const [xch, cats] = await Promise.allSettled([
    getJson(deps, COINGECKO_URL),
    getJson(deps, dexieApiUrl('v3/prices/tickers', false)),
  ]);
  const xchUsd =
    xch.status === 'fulfilled' &&
    isRecord(xch.value) &&
    isRecord(xch.value.chia)
      ? positive(xch.value.chia.usd)
      : null;
  if (xchUsd === null && cats.status === 'rejected') return null;
  return {
    // No XCH price: keep the last one we had rather than showing zeros for a blip.
    xchUsd: xchUsd ?? last?.xchUsd ?? 0,
    cats:
      cats.status === 'fulfilled' ? parseDexie(cats.value) : (last?.cats ?? {}),
    source: 'fallback',
    at: deps.now(),
  };
}
