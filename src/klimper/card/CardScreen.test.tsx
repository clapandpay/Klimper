// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The card screen before the account (milestone gnosis-card-mvp): what each state shows, and
// that signing in asks for biometrics first.

import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { CardStatus } from './card';

vi.mock('@/components/Layout', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('@/components/Header', () => ({
  default: ({ title }: { title: string }) => <h1>{title}</h1>,
}));
const prompt = vi.hoisted(() => vi.fn());
vi.mock('@/hooks/useBiometric', () => ({
  useBiometric: () => ({ promptIfEnabled: prompt }),
}));

const api = vi.hoisted(() => ({
  status: vi.fn(),
  signIn: vi.fn(),
  signOut: vi.fn(),
}));
vi.mock('./card', async (original) => ({
  ...(await original<typeof import('./card')>()),
  card: api,
}));
vi.mock('./CardHome', () => ({
  CardHome: ({ onSignOut }: { onSignOut: () => void }) => (
    <button type='button' data-testid='card-home' onClick={onSignOut}>
      home
    </button>
  ),
}));

const { renderScreen } = await import('../testHarness');
const { default: CardScreen } = await import('./CardScreen');

const ADDRESS = '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266';

function status(over: Partial<CardStatus> = {}): CardStatus {
  return {
    configured: true,
    environment: 'sandbox',
    available: true,
    unavailable: null,
    address: ADDRESS,
    signedIn: false,
    ...over,
  };
}

beforeEach(() => {
  vi.resetAllMocks();
  prompt.mockResolvedValue(true);
  api.signIn.mockResolvedValue(undefined);
  api.signOut.mockResolvedValue(undefined);
});
afterEach(cleanup);

it('says so when the build has no card provider, and signs nothing', async () => {
  api.status.mockResolvedValue(status({ configured: false }));
  renderScreen(<CardScreen />);
  expect(await screen.findByTestId('card-not-configured')).toBeTruthy();
  expect(screen.queryByTestId('card-sign-in-button')).toBeNull();
});

it('explains a wallet without recovery words', async () => {
  api.status.mockResolvedValue(
    status({ available: false, unavailable: 'noRecoveryWords', address: null }),
  );
  renderScreen(<CardScreen />);
  expect(
    await screen.findByText('This wallet cannot have a card'),
  ).toBeTruthy();
});

it('shows the card address and the sandbox note, and signs in after biometrics', async () => {
  api.status.mockResolvedValueOnce(status());
  api.status.mockResolvedValueOnce(status({ signedIn: true }));
  renderScreen(<CardScreen />);
  expect((await screen.findByTestId('card-address')).textContent).toBe(ADDRESS);
  expect(screen.getByTestId('card-sandbox')).toBeTruthy();

  fireEvent.click(screen.getByTestId('card-sign-in-button'));
  await screen.findByTestId('card-home');
  expect(prompt).toHaveBeenCalledTimes(1);
  expect(api.signIn).toHaveBeenCalledTimes(1);
  expect(prompt.mock.invocationCallOrder[0]).toBeLessThan(
    api.signIn.mock.invocationCallOrder[0],
  );
});

it('does not sign in when biometrics are refused', async () => {
  api.status.mockResolvedValue(status());
  prompt.mockResolvedValue(false);
  renderScreen(<CardScreen />);
  fireEvent.click(await screen.findByTestId('card-sign-in-button'));
  await waitFor(() => expect(prompt).toHaveBeenCalled());
  expect(api.signIn).not.toHaveBeenCalled();
});

it('shows why a sign-in was refused', async () => {
  api.status.mockResolvedValue(status());
  api.signIn.mockRejectedValue({
    kind: 'refusedMessage',
    message: 'the sign-in message is for another website',
    code: null,
  });
  renderScreen(<CardScreen />);
  fireEvent.click(await screen.findByTestId('card-sign-in-button'));
  expect((await screen.findByTestId('card-error')).textContent).toContain(
    'another website',
  );
});

it('signs out from the signed-in part', async () => {
  api.status.mockResolvedValueOnce(status({ signedIn: true }));
  api.status.mockResolvedValue(status());
  renderScreen(<CardScreen />);
  fireEvent.click(await screen.findByTestId('card-home'));
  await screen.findByTestId('card-sign-in-button');
  expect(api.signOut).toHaveBeenCalledTimes(1);
});
