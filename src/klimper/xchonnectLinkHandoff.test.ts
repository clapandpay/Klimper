// SPDX-License-Identifier: AGPL-3.0-or-later
// A link is routed by its kind, and its parameters never travel in the URL
// (clapandpay TASK-072).

import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/features', () => ({ xchonnectEnabled: true }));

import {
  offerXchonnectLink,
  resetXchonnectLinkHandoff,
  setXchonnectNavigator,
  subscribeXchonnectLink,
  takeXchonnectLink,
  XCHONNECT_ROUTES,
} from './xchonnectLinkHandoff';

const PARAMS = 'r=https%3A%2F%2Frelay.example%2F&s=VEhFLVNFQ1JFVA&d=pengui.xyz';

describe('offerXchonnectLink', () => {
  let routes: string[];

  beforeEach(() => {
    resetXchonnectLinkHandoff();
    routes = [];
    setXchonnectNavigator((route) => routes.push(route));
  });

  it('routes a pairing link to the connect screen and keeps it out of the URL', () => {
    const url = `https://clapandpay.com/pair#${PARAMS}`;
    const link = offerXchonnectLink(url);

    expect(link).toEqual({ kind: 'pair', uri: url });
    expect(routes).toEqual([XCHONNECT_ROUTES.connect]);
    // The route carries no parameters: the app is a hash router, so anything in the
    // route would be a fragment, and for /pair that fragment is the pairing secret.
    expect(routes[0]).not.toContain('THE-SECRET');
    expect(routes[0]).not.toContain('#');
  });

  it('routes a request link to the approval screen', () => {
    offerXchonnectLink('https://clapandpay.com/req#mbx=bWFpbA');
    expect(routes).toEqual([XCHONNECT_ROUTES.request]);
  });

  it('routes a malformed link to the screen that shows the error', () => {
    const link = offerXchonnectLink('https://evil.example/pair#s=THE-SECRET');
    expect(link.kind).toBe('notALink');
    expect(routes).toEqual([XCHONNECT_ROUTES.connect]);
  });

  it('hands the link over exactly once', () => {
    const url = `xchonnect:v1?${PARAMS}`;
    offerXchonnectLink(url);
    expect(takeXchonnectLink()).toEqual({ kind: 'pair', uri: url });
    // A reload must not replay a pairing code that was already used.
    expect(takeXchonnectLink()).toBeNull();
  });

  it('tells a screen that is already open', () => {
    const seen = vi.fn();
    subscribeXchonnectLink(seen);
    offerXchonnectLink(`xchonnect:v1?${PARAMS}`);
    expect(seen).toHaveBeenCalledWith({
      kind: 'pair',
      uri: `xchonnect:v1?${PARAMS}`,
    });
  });
});
