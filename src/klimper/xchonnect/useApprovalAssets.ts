// SPDX-License-Identifier: AGPL-3.0-or-later
// What the wallet itself knows about the assets of a request: the balance it holds, the
// ticker its token list shows, and for XCH the dollar price Klimper already fetched.
//
// None of this comes from the website, but a CAT's ticker is not proof either: whoever made
// the CAT registered it, and a CAT can be called "XCH". So the screen keeps a CAT's asset id
// on screen wherever it names it (`readPrompt`), and the wallet's ticker, name and icon are
// shown with it, never instead of it; a ticker that reads as the network's coin is not used
// at all, and neither is its icon (`ticker.ts`, `walletTicker`).
//
// Audit of TASK-139:
// * No dollars for a CAT (M1): a CAT's last price on a market is whatever its maker last
//   traded it at with themselves, so "≈ $1,200" on a worthless CAT is one wash trade away.
//   XCH is priced from the last fetch (`src/klimper/price.ts`); on testnet there is no price.
// * Icons (L3): XCH is drawn by the screen itself; a CAT's icon is used only for a CAT the
//   wallet holds and shows, whose icon its token list loads anyway. Loading the icon of a
//   CAT the user has never held would tell its host the user's IP address and what is about
//   to be traded, before the user decided anything.
// * Twins (M2): the wallet's other CATs are read once, so a CAT whose ticker looks like
//   another one's (same skeleton) can be pointed out.

import { commands } from '@/bindings';
import BigNumber from 'bignumber.js';
import { useEffect, useState } from 'react';
import { lastPrices } from '../price';
import { tickerSkeleton } from './ticker';

export interface AssetFacts {
  /** The wallet's ticker for a CAT; never shown in place of its asset id. */
  readonly ticker: string | null;
  /** The wallet's name for a CAT, from the same record. */
  readonly name?: string | null;
  /**
   * The icon the wallet shows for this CAT elsewhere: only for a CAT it holds and shows, and
   * never for XCH (the screen draws XCH's coin itself).
   */
  readonly iconUrl?: string | null;
  /** What the wallet holds now, in mojos. */
  readonly balance: BigNumber | null;
  /**
   * The ids of the wallet's other CATs whose ticker looks the same as this one's (the same
   * skeleton): two tokens the user cannot tell apart by name.
   */
  readonly twins?: readonly string[];
}

/** What identifies an asset for a lookup: XCH, or a CAT by its id. */
export interface AssetRef {
  readonly isXch: boolean;
  /** An option contract (TASK-140): the wallet's token records do not describe one. */
  readonly isOption?: boolean;
  readonly assetId: string | null;
}

/** XCH, a CAT by its id, or nothing to look up (an option, or a CAT whose id did not parse). */
const key = (asset: AssetRef) =>
  asset.isXch ? 'xch' : asset.isOption ? null : asset.assetId;

/** An icon URL the screen may load: https only, so a record cannot point at a local file. */
function iconOf(value: string | null | undefined): string | null {
  return typeof value === 'string' && /^https:\/\/[^\s"'<>]+$/.test(value)
    ? value
    : null;
}

/** The wallet's record of one asset, or null; never throws. */
async function tokenOf(id: string) {
  try {
    return (await commands.getToken({ asset_id: id === 'xch' ? null : id }))
      .token;
  } catch {
    return null;
  }
}

/** Every CAT the wallet knows, by id and ticker; empty when it cannot be read. */
async function walletCats(): Promise<[string, string][]> {
  try {
    const { cats } = await commands.getCats({});
    return cats.flatMap((cat) =>
      cat.asset_id && cat.ticker
        ? [[cat.asset_id.toLowerCase(), cat.ticker] as [string, string]]
        : [],
    );
  } catch {
    return [];
  }
}

export function useApprovalAssets(
  assets: readonly AssetRef[],
): ReadonlyMap<string, AssetFacts> {
  const [facts, setFacts] = useState<ReadonlyMap<string, AssetFacts>>(
    () => new Map(),
  );
  const ids = [...new Set(assets.map(key).filter((id) => id !== null))].join(
    ',',
  );

  useEffect(() => {
    let live = true;
    const wanted = ids === '' ? [] : ids.split(',');
    void Promise.all([
      Promise.all(wanted.map(async (id) => [id, await tokenOf(id)] as const)),
      wanted.some((id) => id !== 'xch') ? walletCats() : Promise.resolve([]),
    ]).then(([records, known]) => {
      if (!live) return;
      // Tickers by skeleton, from the wallet's CATs and the request's own.
      const tickers = new Map<string, string>(known);
      for (const [id, token] of records) {
        if (id !== 'xch' && token?.ticker)
          tickers.set(id.toLowerCase(), token.ticker);
      }
      const bySkeleton = new Map<string, string[]>();
      for (const [id, ticker] of tickers) {
        const skeleton = tickerSkeleton(ticker);
        if (skeleton === '') continue;
        bySkeleton.set(skeleton, [...(bySkeleton.get(skeleton) ?? []), id]);
      }
      const entries = records.map(([id, token]): [string, AssetFacts] => {
        if (id === 'xch') {
          return [
            id,
            {
              ticker: null,
              name: null,
              iconUrl: null,
              balance: token ? BigNumber(token.balance) : null,
            },
          ];
        }
        const balance = token ? BigNumber(token.balance) : null;
        const shown =
          token !== null && token.visible && balance?.isGreaterThan(0);
        const skeleton = token?.ticker ? tickerSkeleton(token.ticker) : '';
        return [
          id,
          {
            ticker: token?.ticker ?? null,
            name: token?.name ?? null,
            iconUrl: shown ? iconOf(token.icon_url) : null,
            balance,
            twins: (bySkeleton.get(skeleton) ?? []).filter(
              (other) => other !== id.toLowerCase(),
            ),
          },
        ];
      });
      setFacts(new Map(entries));
    });
    return () => {
      live = false;
    };
  }, [ids]);

  return facts;
}

export function factsOf(
  facts: ReadonlyMap<string, AssetFacts>,
  asset: AssetRef,
): AssetFacts | undefined {
  const id = key(asset);
  return id === null ? undefined : facts.get(id);
}

/** Dollars for an amount of XCH in mojos, or null when there is no price to go by. */
export function xchUsd(mojos: BigNumber): number | null {
  const prices = lastPrices();
  if (!prices || prices.xchUsd <= 0) return null;
  return mojos.dividedBy(1e12).toNumber() * prices.xchUsd;
}

const dollars = new Intl.NumberFormat(undefined, {
  style: 'currency',
  currency: 'USD',
  maximumFractionDigits: 2,
});

/** `$12.34`, or `<$0.01` for dust. */
export function formatUsd(usd: number): string {
  const abs = Math.abs(usd);
  if (abs > 0 && abs < 0.01) return '<$0.01';
  return dollars.format(abs);
}
