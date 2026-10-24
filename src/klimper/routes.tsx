// SPDX-License-Identifier: AGPL-3.0-or-later
// Klimper's one hook into Sage's router (docs/UPSTREAM.md, "Hook points").
//
// `src/App.tsx` is Sage's file. Klimper used to edit its route block in place — a feature
// flag around six routes, three routes of its own — so every upstream change to those lines
// was a merge conflict. Now Sage's routes are declared exactly as upstream declares them and
// App.tsx wraps Sage's router factory with `klimperRouter`, which passes them through
// `withKlimper` once:
//
//   * a feature switched off in `features.ts` turns its route into a redirect to the
//     wallet, looked up by path in `GATED` below — unreachable, not only hidden;
//   * Klimper's own screens are appended.
//
// A new upstream route needs nothing here; a new gated feature is one line in `GATED`.

import { featureElement } from '@/lib/featureElement';
import {
  nftMintingEnabled,
  nftsEnabled,
  offersEnabled,
  optionsEnabled,
  profilesEnabled,
  swapEnabled,
  themeGalleryEnabled,
  tokenIssuingEnabled,
} from '@/lib/features';
import { ReactElement } from 'react';
import { createRoutesFromElements, RouteObject } from 'react-router-dom';
import { cardRoutes } from './card/CardRoutes';
import { RouteRecovery } from './KlimperErrorBoundary';
import { scannerRoutes } from './Scanner';
import { xchonnectRoutes } from './xchonnect/XchonnectRoutes';

/** Top-level paths of features that can be switched off. */
const GATED: Readonly<Record<string, boolean>> = {
  '/nfts': nftsEnabled,
  '/dids': profilesEnabled,
  '/options': optionsEnabled,
  '/offers': offersEnabled,
  '/swap': swapEnabled,
  '/themes': themeGalleryEnabled,
};

/** Child paths, under their parent's path. */
const GATED_CHILDREN: Readonly<
  Record<string, Readonly<Record<string, boolean>>>
> = {
  '/wallet': { 'issue-token': tokenIssuingEnabled },
  '/nfts': { mint: nftMintingEnabled },
};

function gated(route: RouteObject, enabled: boolean | undefined): RouteObject {
  if (enabled === undefined || route.element === undefined) return route;
  return {
    ...route,
    element: featureElement(enabled, route.element as ReactElement),
  };
}

function gate(route: RouteObject): RouteObject {
  const path = route.path ?? '';
  const children = GATED_CHILDREN[path];
  const next = gated(route, GATED[path]);
  if (children === undefined || next.children === undefined) return next;
  return {
    ...next,
    children: next.children.map((child) =>
      gated(child, children[child.path ?? '']),
    ),
  } as RouteObject;
}

/**
 * `createRoutesFromElements` numbers the routes it makes by position ("0", "0-0", …), and
 * Sage's routes and Klimper's are made by two separate calls, so both lists start at "0". A
 * data router refuses duplicate ids and the app renders nothing. Dropping the ids lets the
 * router number the joined tree once.
 */
function withoutIds(route: RouteObject): RouteObject {
  const next = { ...route, children: route.children?.map(withoutIds) };
  delete next.id;
  if (next.children === undefined) delete next.children;
  return next as RouteObject;
}

/**
 * A screen that throws while rendering shows the recovery screen, with the way back to the
 * wallet, instead of React Router's developer page (TASK-105, `KlimperErrorBoundary.tsx`).
 * Only top-level routes: an error bubbles up to the nearest one.
 */
function recoverable(route: RouteObject): RouteObject {
  if (route.errorElement !== undefined) return route;
  return { ...route, errorElement: <RouteRecovery /> } as RouteObject;
}

/** Sage's routes, with Klimper's feature flags applied and Klimper's screens added. */
export function withKlimper(routes: RouteObject[]): RouteObject[] {
  return [
    ...routes.map(gate),
    ...createRoutesFromElements(
      <>
        {xchonnectRoutes}
        {scannerRoutes}
        {cardRoutes}
      </>,
    ),
  ]
    .map(withoutIds)
    .map(recoverable);
}

/**
 * Wraps Sage's own router factory: App.tsx writes `klimperRouter(createHashRouter)(…)` where
 * upstream writes `createHashRouter(…)`, so Sage's route block below it keeps upstream's
 * exact lines and indentation, and its imports stay untouched.
 */
export function klimperRouter<R>(create: (routes: RouteObject[]) => R) {
  return (routes: RouteObject[]) => create(withKlimper(routes));
}
