// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// Bringing in the Clap and Pay wallet (handoff version 2, TASK-129): Klimper opens the page and
// waits; the answer comes back as a clapandpay.com/klimper/return link that the phone hands to
// this app, and its 24 words are imported with Sage's mnemonic import. (The cryptography of the
// handoff is tested with the page's side in green-envelope/packages/handoff.)

import {
  act,
  cleanup,
  fireEvent,
  screen,
  waitFor,
} from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const WORDS =
  'history indoor salmon ripple side tent tourist decline venture turn diary sad awkward minute barrel much dumb asset milk mandate coach ritual reflect any';
const RETURN = 'https://clapandpay.com/klimper/return#cp2.answer';

const importKey = vi.fn().mockResolvedValue({ fingerprint: 4242 });
const payload = {
  v: 2,
  network: 'testnet11',
  address: 'txch1vwcxaqjcr0n6022h0yrw3wp6vghns6cdd0jnl7g9ehhn27r8kf3qzhsmh3',
  words: WORDS,
};
const handoff = vi.hoisted(() => {
  class HandoffError extends Error {
    constructor(public readonly code: string) {
      super(code);
    }
  }
  return { HandoffError, open: vi.fn(), made: 0 };
});

vi.mock('@/lib/clapandpayHandoff', () => ({
  HandoffError: handoff.HandoffError,
  createHandoffRequest: () => {
    handoff.made += 1;
    return Promise.resolve({
      url: `https://envelope.test/connect#v=2&k=key${handoff.made}&n=nonce`,
      open: (link: string) => handoff.open(link),
    });
  },
}));
vi.mock('@/components/SafeAreaView', () => ({
  default: ({ children }: { children: unknown }) => children,
}));
vi.mock('@/components/Header', () => ({ default: () => null }));
vi.mock('@/components/Container', () => ({
  default: ({ children }: { children: unknown }) => children,
}));
const wallet = vi.hoisted(() => ({ setWallet: () => undefined }));
vi.mock('@/contexts/WalletContext', () => ({ useWallet: () => wallet }));
// Stable across renders, like the real hook: a new function each time would re-run the effect
// that makes the request, forever.
const errors = vi.hoisted(() => ({ addError: () => undefined }));
vi.mock('@/hooks/useErrors', () => ({ useErrors: () => errors }));
const biometric = vi.hoisted(() => ({
  available: true,
  enabled: false,
  enableIfAvailable: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('@/hooks/useBiometric', () => ({ useBiometric: () => biometric }));
vi.mock('@/lib/features', () => ({
  clapAndPayOrigin: 'https://envelope.test',
}));
const openUrl = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl }));
vi.mock('react-toastify', () => ({ toast: { info: vi.fn() } }));
vi.mock('@/state', () => ({ clearState: vi.fn(), fetchState: vi.fn() }));
const sage = vi.hoisted(() => ({
  login: vi.fn().mockResolvedValue(null),
  getKey: vi.fn().mockResolvedValue({ key: null }),
  keys: [] as {
    fingerprint: number;
    has_secrets: boolean;
    mnemonic: string | null;
  }[],
}));
vi.mock('@/bindings', () => ({
  commands: {
    importKey: (...args: unknown[]) => importKey(...args),
    getNetwork: () => Promise.resolve({ network: { name: 'testnet11' } }),
    getKey: (req: unknown) => sage.getKey(req),
    login: (req: unknown) => sage.login(req),
    getKeys: () => Promise.resolve({ keys: sage.keys }),
    getSecretKey: ({ fingerprint }: { fingerprint: number }) => {
      const key = sage.keys.find((k) => k.fingerprint === fingerprint);
      return Promise.resolve({
        secrets: key?.mnemonic
          ? { mnemonic: key.mnemonic, secret_key: 'sk' }
          : null,
      });
    },
  },
}));
const navigate = vi.hoisted(() => vi.fn());
vi.mock('react-router-dom', async (original) => ({
  ...(await original<typeof import('react-router-dom')>()),
  useNavigate: () => navigate,
}));

const { renderScreen } = await import('./testHarness');
const { default: ClapAndPayConnect } = await import('./ClapAndPayConnect');
const { offerClapAndPayReturn } = await import('./clapandpayReturn');

afterEach(() => {
  cleanup();
  importKey.mockReset();
  importKey.mockResolvedValue({ fingerprint: 4242 });
  sage.login.mockClear();
  sage.getKey.mockClear();
  sage.keys = [];
  navigate.mockClear();
  biometric.enableIfAvailable.mockClear();
  handoff.open.mockReset();
  openUrl.mockClear();
});

async function openThePage() {
  renderScreen(<ClapAndPayConnect />);
  const button = await screen.findByRole('button', {
    name: /Open envelope\.test/,
  });
  await waitFor(() => expect(button).toHaveProperty('disabled', false));
  fireEvent.click(button);
  await waitFor(() => expect(openUrl).toHaveBeenCalledTimes(1));
  return openUrl.mock.calls[0]?.[0] as string;
}

describe('Clap and Pay connect, version 2', () => {
  it('opens the page with the request and imports the 24 words that come back', async () => {
    handoff.open.mockResolvedValue(payload);
    const url = await openThePage();
    expect(url).toMatch(/^https:\/\/envelope\.test\/connect#v=2&k=/);
    // Nothing to paste, no codes to compare.
    expect(screen.queryByRole('button', { name: /paste/i })).toBeNull();

    act(() => offerClapAndPayReturn(RETURN));
    await waitFor(() => expect(importKey).toHaveBeenCalledTimes(1));
    expect(handoff.open).toHaveBeenCalledWith(RETURN);
    expect(importKey.mock.calls[0]?.[0]).toMatchObject({
      name: 'Clap and Pay',
      key: WORDS,
    });
  });

  it('imports nothing from an answer it cannot open, and starts again with a new key', async () => {
    handoff.open.mockRejectedValue(new handoff.HandoffError('expired'));
    await openThePage();
    const made = handoff.made;

    act(() => offerClapAndPayReturn(RETURN));
    expect(await screen.findByText(/longer than two minutes/)).toBeTruthy();
    expect(importKey).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Start again' }));
    await waitFor(() => expect(handoff.made).toBe(made + 1));
  });

  it('offers Face ID after the import when it is off (TASK-126 audit)', async () => {
    handoff.open.mockResolvedValue(payload);
    await openThePage();
    act(() => offerClapAndPayReturn(RETURN));
    const turnOn = await screen.findByRole('button', {
      name: 'Turn on Face ID',
    });
    fireEvent.click(turnOn);
    await waitFor(() =>
      expect(biometric.enableIfAvailable).toHaveBeenCalledTimes(1),
    );
  });
});

describe('the same wallet a second time (TASK-130)', () => {
  it('opens the account it already is instead of adding a duplicate', async () => {
    importKey.mockRejectedValue({
      kind: 'internal',
      reason: 'Keychain error: Key already exists',
    });
    sage.keys = [
      { fingerprint: 111, has_secrets: true, mnemonic: 'other words' },
      { fingerprint: 222, has_secrets: false, mnemonic: null },
      { fingerprint: 333, has_secrets: true, mnemonic: WORDS },
    ];
    handoff.open.mockResolvedValue(payload);
    await openThePage();
    act(() => offerClapAndPayReturn(RETURN));

    await waitFor(() =>
      expect(sage.login).toHaveBeenCalledWith({ fingerprint: 333 }),
    );
    expect(importKey).toHaveBeenCalledTimes(1);
    expect(sage.getKey).toHaveBeenCalledWith({ fingerprint: 333 });
  });

  it('keeps the import when Face ID is declined', async () => {
    handoff.open.mockResolvedValue(payload);
    await openThePage();
    act(() => offerClapAndPayReturn(RETURN));
    fireEvent.click(await screen.findByRole('button', { name: 'Not now' }));
    expect(navigate).toHaveBeenCalledWith('/wallet');
    expect(importKey).toHaveBeenCalledTimes(1);
    expect(biometric.enableIfAvailable).not.toHaveBeenCalled();
  });
});

describe('a return link with nobody waiting', () => {
  it('is dropped with a word to the person, and opens nothing', async () => {
    const { toast } = await import('react-toastify');
    offerClapAndPayReturn(RETURN);
    expect(toast.info).toHaveBeenCalledTimes(1);
    expect(handoff.open).not.toHaveBeenCalled();
  });
});
