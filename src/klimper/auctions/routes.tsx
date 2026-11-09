// SPDX-License-Identifier: AGPL-3.0-or-later
// The auction screens' routes, added by Klimper's router hook (`src/klimper/routes.tsx`).
// Without the flag they redirect to the wallet. The pages load lazily.

import { featureElement } from '@/lib/featureElement';
import { auctionsEnabled } from '@/lib/features';
import { lazy, ReactElement, Suspense } from 'react';
import { Route } from 'react-router-dom';

const AuctionsPage = lazy(() => import('./AuctionsPage'));
const AuctionPage = lazy(() => import('./AuctionPage'));
const StartAuction = lazy(() => import('./StartAuction'));

function page(element: ReactElement) {
  return featureElement(
    auctionsEnabled,
    <Suspense fallback={null}>{element}</Suspense>,
  );
}

export const auctionRoutes = (
  <Route path='/auctions'>
    <Route path='' element={page(<AuctionsPage />)} />
    <Route path='view/:id' element={page(<AuctionPage />)} />
    <Route path='start/:nftId' element={page(<StartAuction />)} />
  </Route>
);
