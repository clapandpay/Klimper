// SPDX-License-Identifier: AGPL-3.0-or-later
// Links the OS hands the app reach the handoff, once each, and nothing is logged
// (clapandpay TASK-072.01).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/features', () => ({
  xchonnectEnabled: true,
  clapAndPayConnectEnabled: true,
}));
const offerClapAndPayReturn = vi.hoisted(() => vi.fn());
vi.mock('../clapandpayReturn', () => ({ offerClapAndPayReturn }));
vi.mock('@tauri-apps/plugin-deep-link', () => ({
  onOpenUrl: () => Promise.reject(new Error('no ipc')),
  getCurrent: () => Promise.reject(new Error('no ipc')),
}));

import { DeepLinkApi, listenForXchonnectLinks } from './xchonnectDeepLink';
import {
  resetXchonnectLinkHandoff,
  setXchonnectNavigator,
  takeXchonnectLink,
  XCHONNECT_ROUTES,
} from './xchonnectLinkHandoff';

const PAIRING_URI =
  'xchonnect:v1?r=https%3A%2F%2Frelay.example%2F&s=VEhFLVNFQ1JFVA&d=pengui.xyz';
const REQUEST_LINK = 'https://clapandpay.com/req#mbx=abc';

/** A plugin whose event we can fire by hand. */
function fakePlugin(launch: string[] | null) {
  let handler: ((urls: string[]) => void) | null = null;
  const unlisten = vi.fn();
  const api: DeepLinkApi = {
    onOpenUrl: vi.fn(async (h) => {
      handler = h;
      return unlisten;
    }),
    getCurrent: vi.fn(async () => launch),
  };
  return { api, unlisten, fire: (urls: string[]) => handler?.(urls) };
}

describe('listenForXchonnectLinks', () => {
  let routes: string[];
  const log = vi.spyOn(console, 'log');
  const warn = vi.spyOn(console, 'warn');
  const error = vi.spyOn(console, 'error');

  beforeEach(() => {
    resetXchonnectLinkHandoff();
    routes = [];
    setXchonnectNavigator((route) => routes.push(route));
    log.mockClear();
    warn.mockClear();
    error.mockClear();
  });

  afterEach(() => {
    expect(log).not.toHaveBeenCalled();
    expect(warn).not.toHaveBeenCalled();
    expect(error).not.toHaveBeenCalled();
  });

  it('hands the link the app was started with to the connect screen', async () => {
    const plugin = fakePlugin([PAIRING_URI]);
    await listenForXchonnectLinks(plugin.api);

    expect(routes).toEqual([XCHONNECT_ROUTES.connect]);
    expect(takeXchonnectLink()).toEqual({ kind: 'pair', uri: PAIRING_URI });
    // Listening starts before the launch URL is read, so nothing falls in between.
    expect(plugin.api.onOpenUrl).toHaveBeenCalledBefore(
      plugin.api.getCurrent as ReturnType<typeof vi.fn>,
    );
  });

  it('hands a link that arrives while the app runs to the right screen', async () => {
    const plugin = fakePlugin(null);
    await listenForXchonnectLinks(plugin.api);
    expect(routes).toEqual([]);

    plugin.fire([REQUEST_LINK]);
    expect(routes).toEqual([XCHONNECT_ROUTES.request]);
    expect(takeXchonnectLink()).toEqual({
      kind: 'request',
      mailboxHint: 'abc',
    });
  });

  it('delivers a link that comes through both paths at start-up once, and later taps again', async () => {
    const plugin = fakePlugin([PAIRING_URI]);
    const started = listenForXchonnectLinks(plugin.api);
    // The OS event for the launch URL lands right after the listener is registered.
    await Promise.resolve();
    plugin.fire([PAIRING_URI]);
    await started;
    expect(routes).toEqual([XCHONNECT_ROUTES.connect]);

    // Start-up is over: the same link, tapped again by the user, is a new request.
    plugin.fire([PAIRING_URI]);
    expect(routes).toEqual([
      XCHONNECT_ROUTES.connect,
      XCHONNECT_ROUTES.connect,
    ]);
  });

  it('is a no-op outside a Tauri window', async () => {
    const stop = await listenForXchonnectLinks();
    expect(routes).toEqual([]);
    expect(() => stop()).not.toThrow();
  });

  it('returns the unsubscribe of the plugin', async () => {
    const plugin = fakePlugin(null);
    const stop = await listenForXchonnectLinks(plugin.api);
    stop();
    expect(plugin.unlisten).toHaveBeenCalledOnce();
  });

  it('hands the Clap and Pay return link to the handoff, never to Xchonnect (TASK-129)', async () => {
    const plugin = fakePlugin(null);
    await listenForXchonnectLinks(plugin.api);
    const link = 'https://clapandpay.com/klimper/return#cp2.answer';
    plugin.fire([link]);
    expect(offerClapAndPayReturn).toHaveBeenCalledWith(link);
    expect(routes).toEqual([]);
    expect(takeXchonnectLink()).toBeNull();
  });
});
