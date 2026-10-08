// SPDX-License-Identifier: AGPL-3.0-or-later
// What the wallet itself knows about the assets of a request: the name and icon it shows
// everywhere else, the balance it holds, and the dollar price Klimper already fetched.
//
// None of this comes from the website. The prompt names a CAT by its asset id only
// (`readPrompt`); the ticker here is the wallet's own record of that id, the one its token
// list shows. The dollar figure is the last price fetch (`src/klimper/price.ts`), so a
// request costs no network call; on testnet, or for an asset without a price, there is none.

import { commands } from '@/bindings';
import BigNumber from 'bignumber.js';
import { useEffect, useState } from 'react';
import { lastPrices } from './price';
import { AssetView } from './xchonnectPrompt';

export interface AssetFacts {
  readonly ticker: string | null;
  readonly icon: string | null;
  /** What the wallet holds now, in mojos. */
  readonly balance: BigNumber | null;
}

const key = (asset: AssetView) => asset.assetId ?? 'xch';

export function useApprovalAssets(
  assets: readonly AssetView[],
): ReadonlyMap<string, AssetFacts> {
  const [facts, setFacts] = useState<ReadonlyMap<string, AssetFacts>>(
    () => new Map(),
  );
  const ids = assets.map(key).join(',');

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
              ticker: token?.ticker ?? null,
              icon: token?.icon_url ?? null,
              balance: token ? BigNumber(token.balance) : null,
            },
          ])
          .catch(() => [id, { ticker: null, icon: null, balance: null }]),
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
  return facts.get(key(asset));
}

/** Dollars for an amount in mojos, or null when there is no price to go by. */
export function usdOf(
  assetId: string | null,
  mojos: BigNumber,
  precision: number,
): number | null {
  const prices = lastPrices();
  if (!prices || prices.xchUsd <= 0) return null;
  const xch = assetId
    ? (prices.cats[assetId.toLowerCase()]?.lastPrice ?? null)
    : 1;
  if (xch === null || xch <= 0) return null;
  const amount = mojos.dividedBy(BigNumber(10).pow(precision)).toNumber();
  return amount * xch * prices.xchUsd;
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
