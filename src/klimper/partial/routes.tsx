// SPDX-License-Identifier: AGPL-3.0-or-later
// Partial offers in Sage's `/offers` routes (TASK-164), from Klimper's router hook
// (`src/klimper/routes.tsx`), so Sage's route block and offer pages stay untouched:
//
//   * `/offers/view/:offer` shows a `partial1…` on the partial offer page and anything else on
//     Sage's own page. Every way an offer is opened (paste, NFC, QR, a link, the Scan tab)
//     ends there, so this one switch covers them all;
//   * `/offers/make-partial` is added.
//
// With the flag off, both are as Sage has them and `make-partial` redirects to the wallet.

import { featureElement } from '@/lib/featureElement';
import { partialOffersEnabled } from '@/lib/features';
import { lazy, ReactElement, Suspense } from 'react';
import { RouteObject, useParams } from 'react-router-dom';
import { isPartialOffer } from './isPartialOffer';

// Loaded when first opened: the router stays light, and a build without the flag never
// loads them.
const MakePartialOffer = lazy(() => import('./MakePartialOffer'));
const PartialOfferPage = lazy(() => import('./PartialOfferPage'));

function OfferOrPartial({ sage }: { sage: ReactElement }) {
  const { offer } = useParams();
  const text = offer ? decodeURIComponent(offer) : '';
  return isPartialOffer(text) ? (
    <Suspense fallback={null}>
      <PartialOfferPage />
    </Suspense>
  ) : (
    sage
  );
}

export function withPartialOffers(route: RouteObject): RouteObject {
  if (!partialOffersEnabled || route.path !== '/offers' || !route.children) {
    return route;
  }
  return {
    ...route,
    children: [
      ...route.children.map((child) =>
        child.path === 'view/:offer' && child.element
          ? {
              ...child,
              element: <OfferOrPartial sage={child.element as ReactElement} />,
            }
          : child,
      ),
      {
        path: 'make-partial',
        element: featureElement(
          partialOffersEnabled,
          <Suspense fallback={null}>
            <MakePartialOffer />
          </Suspense>,
        ),
      },
    ],
  } as RouteObject;
}
