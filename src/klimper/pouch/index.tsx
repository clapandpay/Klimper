// SPDX-License-Identifier: AGPL-3.0-or-later
// The one door from Sage's coin card to the coin pouch (TASK-124). Behind the Klimper coins
// flag: off, this renders nothing and the build carries none of the pouch; on, the pouch is
// its own chunk, loaded the first time a token page opens.

import { klimperCoinsEnabled } from '@/lib/features';
import { lazy, Suspense } from 'react';
import type { CoinPouchProps } from './CoinPouch';

const CoinPouch = klimperCoinsEnabled
  ? lazy(() => import('./CoinPouch'))
  : null;

export function KlimperCoinPouch(props: CoinPouchProps) {
  if (!CoinPouch) return null;
  return (
    <Suspense fallback={null}>
      <CoinPouch {...props} />
    </Suspense>
  );
}
