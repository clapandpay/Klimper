// SPDX-License-Identifier: AGPL-3.0-or-later
// Handing an incoming link to the screen that acts on it (clapandpay TASK-072, TASK-085).
//
// Klimper uses a hash router, so the app's URL *is* a fragment. A universal link's
// parameters are a fragment too, and for `/pair` they contain the pairing secret. Putting
// them in the app URL would therefore break routing and write the secret into the history
// stack, where a screenshot, a crash report or a later `window.location` read would find
// it.
//
// So the link is kept here, in memory, for exactly one read, and navigation carries
// nothing but the route. `offerXchonnectLink` is the single entry point: a deep-link
// listener, a scanned QR code and a pasted code all go through it.

import { xchonnectEnabled } from '@/lib/features';
import { parseXchonnectLink, XchonnectLink } from './xchonnectLink';

/** Where each kind of link is answered. */
export const XCHONNECT_ROUTES = {
  dapps: '/xchonnect',
  connect: '/xchonnect/connect',
  request: '/xchonnect/request',
  /** One connected website's permissions; the session id is appended. */
  dapp: '/xchonnect/dapp',
} as const;

let pending: XchonnectLink | null = null;

/**
 * Go to a route of the app. Replaceable so a test can watch it; the default assigns the
 * hash, which is how a hash router is navigated from outside React (a deep link arrives
 * on a plugin callback, not inside a component).
 */
let navigateTo: (route: string) => void = (route) => {
  window.location.hash = `#${route}`;
};

export function setXchonnectNavigator(navigate: (route: string) => void) {
  navigateTo = navigate;
}

/** Go to an Xchonnect route from outside the router (the approval overlay). */
export function goToXchonnectRoute(route: string) {
  navigateTo(route);
}

type Listener = (link: XchonnectLink) => void;
const listeners = new Set<Listener>();

/**
 * Tell the app a link arrived, from wherever.
 *
 * Returns what it was parsed as, so a caller that has its own error surface (the paste
 * field) can show the problem instead of navigating. A malformed link is still stored and
 * still routes to the pairing screen, because that screen is where the error belongs —
 * showing an error and doing nothing else is the whole response to a bad link.
 */
export function offerXchonnectLink(raw: string): XchonnectLink {
  const link = parseXchonnectLink(raw);
  if (!xchonnectEnabled) return link;

  pending = link;
  for (const listener of listeners) listener(link);
  navigateTo(
    link.kind === 'request'
      ? XCHONNECT_ROUTES.request
      : XCHONNECT_ROUTES.connect,
  );
  return link;
}

/** Read and clear. A link is acted on once; a reload must not replay a pairing secret. */
export function takeXchonnectLink(): XchonnectLink | null {
  const link = pending;
  pending = null;
  return link;
}

/** For a screen that is already open when a link arrives. */
export function subscribeXchonnectLink(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Tests only: forget everything between cases. */
export function resetXchonnectLinkHandoff() {
  pending = null;
  listeners.clear();
}
