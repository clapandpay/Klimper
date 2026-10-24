// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The card account (TASK-151): balance, the card, payments, freeze without a question and
// unfreeze with one.

import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { Account, CardSummary } from './card';

vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));
vi.mock('@/hooks/useBiometric', () => ({
  useBiometric: () => ({ promptIfEnabled: () => Promise.resolve(true) }),
}));
const privacy = vi.hoisted(() => ({ on: false }));
vi.mock('../privacy', () => ({
  useMaskAmounts: () => privacy.on,
  Masked: () => <span>•••••</span>,
}));

const api = vi.hoisted(() => ({
  account: vi.fn(),
  transactions: vi.fn(),
  setFrozen: vi.fn(),
  orderCard: vi.fn(),
}));
vi.mock('./card', async (original) => ({
  ...(await original<typeof import('./card')>()),
  card: api,
}));

const { renderScreen } = await import('../testHarness');
const { CardAccount } = await import('./CardAccount');

function plastic(status: CardSummary['status']): CardSummary {
  return {
    id: 'card-1',
    status,
    kind: 'virtual',
    last4: '4242',
    brand: 'VISA',
    expires: '10/29',
  };
}

function account(cards: CardSummary[], over: Partial<Account> = {}): Account {
  return {
    safeAddress: '0x5afe5afE5afE5AfE5aFE5aFe5AFe5afe5afE5aFE',
    ownerAddress: '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266',
    status: 'active',
    currency: 'EUR',
    spendable: '123.45',
    pending: '0.00',
    cards,
    ...over,
  };
}

const signedOut = vi.fn();

beforeEach(() => {
  vi.resetAllMocks();
  api.setFrozen.mockResolvedValue(null);
  api.transactions.mockResolvedValue({
    items: [
      {
        id: 't-2',
        kind: 'card',
        createdAt: '2026-10-23T18:04:00Z',
        status: 'settled',
        merchant: 'Bakery Example',
        amount: '4.20',
        currency: 'EUR',
      },
      {
        id: 't-1',
        kind: 'card',
        createdAt: '2026-10-22T08:15:00Z',
        status: 'declined',
        merchant: 'Shop Example',
        amount: '900.00',
        currency: 'EUR',
      },
    ],
    nextCursor: null,
  });
});
afterEach(cleanup);

it('shows the balance, the card and the payments', async () => {
  api.account.mockResolvedValue(
    account([plastic('active')], { pending: '5.00' }),
  );
  renderScreen(<CardAccount onSignedOut={signedOut} />);
  expect((await screen.findByTestId('card-spendable')).textContent).toContain(
    '123.45',
  );
  expect(screen.getByTestId('card-pending').textContent).toContain('5.00');
  expect(screen.getByTestId('card-last4').textContent).toBe('4242');
  expect(screen.getByTestId('card-status').textContent).toContain(
    'Ready to pay',
  );
  expect(screen.getByText('Bakery Example')).toBeTruthy();
  expect(screen.getByText(/declined/)).toBeTruthy();
  expect(screen.getByTestId('card-safe-address').textContent).toMatch(
    /^0x5afe/i,
  );
});

it('hides the amounts in privacy mode', async () => {
  privacy.on = true;
  api.account.mockResolvedValue(
    account([plastic('active')], { pending: '5.00' }),
  );
  renderScreen(<CardAccount onSignedOut={signedOut} />);
  const spendable = await screen.findByTestId('card-spendable');
  expect(spendable.textContent).not.toContain('123.45');
  expect(screen.queryByTestId('card-pending')).toBeNull();
  expect(document.body.textContent).not.toContain('4.20');
  expect(screen.getByText('Bakery Example')).toBeTruthy();
  privacy.on = false;
});

it('freezes at once', async () => {
  api.account.mockResolvedValueOnce(account([plastic('active')]));
  api.account.mockResolvedValue(account([plastic('blocked')]));
  renderScreen(<CardAccount onSignedOut={signedOut} />);
  fireEvent.click(await screen.findByTestId('card-freeze'));
  await screen.findByTestId('card-unfreeze');
  expect(api.setFrozen).toHaveBeenCalledWith('card-1', true);
  expect(screen.getByTestId('card-status').textContent).toContain('Frozen');
});

it('unfreezes only after a confirmation', async () => {
  api.account.mockResolvedValueOnce(account([plastic('blocked')]));
  api.account.mockResolvedValue(account([plastic('active')]));
  renderScreen(<CardAccount onSignedOut={signedOut} />);
  fireEvent.click(await screen.findByTestId('card-unfreeze'));
  expect(api.setFrozen).not.toHaveBeenCalled();
  fireEvent.click(screen.getByTestId('card-unfreeze-confirm'));
  await screen.findByTestId('card-freeze');
  expect(api.setFrozen).toHaveBeenCalledWith('card-1', false);
});

it('offers a card when the account has none', async () => {
  api.account.mockResolvedValue(account([]));
  renderScreen(<CardAccount onSignedOut={signedOut} />);
  expect(await screen.findByTestId('card-order')).toBeTruthy();
});

it('goes back to the sign-in when the session has ended', async () => {
  api.account.mockRejectedValue({
    kind: 'signedOut',
    message: 'x',
    code: null,
  });
  renderScreen(<CardAccount onSignedOut={signedOut} />);
  await waitFor(() => expect(signedOut).toHaveBeenCalled());
});

it('shows an error and keeps nothing stale', async () => {
  api.account.mockRejectedValue({
    kind: 'network',
    message: 'Gnosis Pay could not be reached',
    code: null,
  });
  renderScreen(<CardAccount onSignedOut={signedOut} />);
  expect((await screen.findByRole('alert')).textContent).toContain(
    'could not be reached',
  );
  expect(screen.queryByTestId('card-spendable')).toBeNull();
});
