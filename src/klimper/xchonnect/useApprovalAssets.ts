// SPDX-License-Identifier: AGPL-3.0-or-later
// What the wallet itself knows about the assets of a request: the balance it holds, the
// ticker its token list shows, and for XCH the dollar price Klimper already fetched.
//
// None of this comes from the website, but a CAT's ticker is not proof either: whoever made
// the CAT registered it, and a CAT can be called "XCH". So the screen keeps naming a CAT by
// its asset id (`readPrompt`) and shows the ticker only beside it, softly; a CAT gets no icon
// and no price here. XCH is priced from the last fetch (`src/klimper/price.ts`), so a request
// costs no network call; on testnet there is no price.

import { commands } from '@/bindings';
import BigNumber from 'bignumber.js';
import { useEffect, useState } from 'react';
import { lastPrices } from '../price';
import { AssetView } from './xchonnectPrompt';

export interface AssetFacts {
  /** The wallet's name for a CAT; never shown in place of its asset id. */
  readonly ticker: string | null;
  /** What the wallet holds now, in mojos. */
  readonly balance: BigNumber | null;
}

/** XCH, a CAT by its id, or nothing to look up (a CAT whose id did not parse). */
const key = (asset: AssetView) => (asset.isXch ? 'xch' : asset.assetId);

export function useApprovalAssets(
  assets: readonly AssetView[],
): ReadonlyMap<string, AssetFacts> {
  const [facts, setFacts] = useState<ReadonlyMap<string, AssetFacts>>(
    () => new Map(),
  );
  const ids = assets
    .map(key)
    .filter((id) => id !== null)
    .join(',');

  useEffect(() => {
    let live = true;
    const wanted = ids === '' ? [] : ids.split(',');
    void Promise.all(
      wanted.map((id) =>
        commands
          .getToken({ asset_id: id === 'xch' ? null : id })
          .then(({ token }) => [
            id,
            {
              ticker: id === 'xch' ? null : (token?.ticker ?? null),
              balance: token ? BigNumber(token.balance) : null,
            },
          ])
          .catch(() => [id, { ticker: null, balance: null }]),
      ),
    ).then((entries) => {
      if (live) setFacts(new Map(entries as [string, AssetFacts][]));
    });
    return () => {
      live = false;
    };
  }, [ids]);

  return facts;
}

export function factsOf(
  facts: ReadonlyMap<string, AssetFacts>,
  asset: AssetView,
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
