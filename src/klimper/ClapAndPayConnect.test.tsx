// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// Bringing in the Clap and Pay wallet: nothing is imported before the person has seen the
// answer code and said the page shows the same. (The cryptography of the handoff is tested
// with the page's side in green-envelope/packages/handoff.)

import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const importKey = vi.fn().mockResolvedValue(undefined);
const payload = {
  v: 1,
  network: 'testnet11',
  address: 'txch1anxwneg6hlkmv2uff2mnqh0qhcp5g24hkkfh5j6m9w4t7vjejfts5au9zn',
  masterSk: '3f2d5b3d12cda13881eefced740a88d33e0d7acd279c99d53bb6797a9066b2be',
};

vi.mock('@/lib/clapandpayHandoff', () => ({
  HandoffError: class extends Error {},
  createHandoffRequest: () =>
    Promise.resolve({
      url: 'https://envelope.test/connect#v=1&k=key',
      checkCode: '111111',
      open: () => Promise.resolve({ payload, answerCode: '481516' }),
    }),
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
vi.mock('@/lib/features', () => ({
  clapAndPayOrigin: 'https://envelope.test',
}));
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({
  readText: () => Promise.resolve('cp1.answer'),
}));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));
vi.mock('@/state', () => ({ clearState: vi.fn(), fetchState: vi.fn() }));
vi.mock('@/bindings', () => ({
  commands: {
    importKey: (...args: unknown[]) => importKey(...args),
    getNetwork: () => Promise.resolve({ network: { name: 'testnet11' } }),
    getKey: () => Promise.resolve({ key: null }),
  },
}));

const { renderScreen } = await import('./testHarness');
const { default: ClapAndPayConnect } = await import('./ClapAndPayConnect');

afterEach(() => {
  cleanup();
  importKey.mockClear();
});

async function paste() {
  renderScreen(<ClapAndPayConnect />);
  const button = await screen.findByRole('button', {
    name: /Paste what the page gave you/,
  });
  await waitFor(() => expect(button).toHaveProperty('disabled', false));
  fireEvent.click(button);
  return screen.findByTestId('answer-code');
}

describe('Clap and Pay connect', () => {
  it('shows the answer code and imports only after the person confirms it', async () => {
    expect((await paste()).textContent).toBe('481 516');
    expect(importKey).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: 'Same number' }));
    await waitFor(() => expect(importKey).toHaveBeenCalledTimes(1));
    expect(importKey.mock.calls[0]?.[0]).toMatchObject({
      key: payload.masterSk,
    });
  });

  it('imports nothing when the person says the number differs', async () => {
    await paste();
    fireEvent.click(screen.getByRole('button', { name: 'Different' }));
    expect(await screen.findByText(/did not come from/)).toBeTruthy();
    expect(importKey).not.toHaveBeenCalled();
  });
});
