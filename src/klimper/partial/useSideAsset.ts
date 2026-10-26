// SPDX-License-Identifier: AGPL-3.0-or-later
// How to show one side of a partial offer: its ticker and its precision, from the wallet's
// own token list (a CAT the wallet has never seen shows a short asset id and 3 decimals,
// the CAT standard).

import { commands } from '@/bindings';
import { toDecimal } from '@/lib/utils';
import { useWalletState } from '@/state';
import { useEffect, useState } from 'react';

export interface SideAsset {
  ticker: string;
  name: string | null;
  precision: number;
  iconUrl: string | null;
}

const CAT_PRECISION = 3;

export function useSideAsset(assetId: string | null): SideAsset {
  const unit = useWalletState().sync.unit;
  const xch: SideAsset = {
    ticker: unit.ticker,
    name: 'Chia',
    precision: unit.precision,
    iconUrl: null,
  };
  const unknown = (id: string): SideAsset => ({
    ticker: `${id.slice(0, 6)}…`,
    name: null,
    precision: CAT_PRECISION,
    iconUrl: null,
  });
  const [asset, setAsset] = useState<SideAsset | null>(null);

  useEffect(() => {
    if (assetId === null) return;
    let live = true;
    commands
      .getToken({ asset_id: assetId })
      .then(({ token }) => {
        if (!live || token === null) return;
        setAsset({
          ticker: token.ticker ?? token.name ?? `${assetId.slice(0, 6)}…`,
          name: token.name,
          precision: token.precision,
          iconUrl: token.icon_url,
        });
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [assetId]);

  if (assetId === null) return xch;
  return asset ?? unknown(assetId);
}

/** `mojos` of `asset`, as the user reads it: "12.5 SBX". */
export function amountText(mojos: string | bigint, asset: SideAsset): string {
  return `${toDecimal(mojos.toString(), asset.precision)} ${asset.ticker}`;
}
