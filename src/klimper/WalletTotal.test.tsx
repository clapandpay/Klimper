// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The wallet's total with the card (TASK-158): wallet and card together on top, each on its
// own line beneath, the card's top-ups on the way, and nothing about a card when there is none.

import { cleanup, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CardBalance } from './card/useCardBalance';

const privacy = vi.hoisted(() => ({ on: false }));
vi.mock('./privacy', () => ({
  useMaskAmounts: () => privacy.on,
  Masked: () => <span>•••••</span>,
}));
const prices = vi.hoisted(() => ({
  current: { xchUsd: 10, eurUsd: 1.1 as number | null },
}));
vi.mock('./price', () => ({ lastPrices: () => prices.current }));
const api = vi.hoisted(() => ({ status: vi.fn(), account: vi.fn() }));
vi.mock('./card/card', async (original) => ({
  ...(await original<typeof import('./card/card')>()),
  card: api,
}));

const { renderScreen } = await import('./testHarness');
const { WalletTotalView } = await import('./WalletTotal');
const { readCardBalance, cardInUsd } = await import('./card/useCardBalance');

const ready: CardBalance = {
  state: 'ready',
  currency: 'EUR',
  spendable: 100,
  pending: 0,
  last4: '4242',
  frozen: false,
};

function show(card: CardBalance, usd = 50, hasTokens = true) {
  return renderScreen(
    <WalletTotalView usd={usd} hasTokens={hasTokens} card={card} />,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  privacy.on = false;
  prices.current = { xchUsd: 10, eurUsd: 1.1 };
});
afterEach(cleanup);

describe('the wallet total', () => {
  it('is the wallet alone when there is no card', () => {
    show({ state: 'none' });
    expect(screen.getByTestId('wallet-total-amount').textContent).toBe(
      '$50.00',
    );
    expect(screen.queryByTestId('wallet-total-card')).toBeNull();
    expect(screen.queryByTestId('wallet-total-wallet')).toBeNull();
  });

  it('adds what the card can spend, at the euro rate, and splits it beneath', () => {
    show(ready);
    expect(screen.getByTestId('wallet-total-amount').textContent).toBe(
      '$160.00',
    );
    expect(screen.getByTestId('wallet-total-wallet').textContent).toContain(
      '$50.00',
    );
    const line = screen.getByTestId('wallet-total-card').textContent ?? '';
    expect(line).toContain('••4242');
    expect(line).toContain('€100.00');
    expect(screen.getByText(/16 XCH/)).toBeTruthy();
    expect(screen.getByTestId('wallet-total-top-up').getAttribute('href')).toBe(
      '/card',
    );
  });

  it('keeps the total to the wallet when no euro rate is known', () => {
    prices.current = { xchUsd: 10, eurUsd: null };
    show(ready);
    expect(screen.getByTestId('wallet-total-amount').textContent).toBe(
      '$50.00',
    );
    expect(screen.getByTestId('wallet-total-card').textContent).toContain(
      '€100.00',
    );
  });

  it('shows a top-up on the way, and hides it with every amount in privacy mode', () => {
    show({ ...ready, pending: 20 });
    expect(
      screen.getByTestId('wallet-total-card-pending').textContent,
    ).toContain('€20.00');
    cleanup();
    privacy.on = true;
    show({ ...ready, pending: 20 });
    expect(screen.queryByTestId('wallet-total-card-pending')).toBeNull();
    expect(screen.getByTestId('wallet-total-amount').textContent).toBe('•••••');
    expect(screen.getByTestId('wallet-total-card').textContent).not.toContain(
      '100',
    );
  });

  it('links to the card screen while signed out or not set up, without amounts', () => {
    show({ state: 'signedOut' });
    const line = screen.getByTestId('wallet-total-card');
    expect(line.getAttribute('href')).toBe('/card');
    expect(line.textContent).toContain('Sign in');
    expect(screen.getByTestId('wallet-total-amount').textContent).toBe(
      '$50.00',
    );
    cleanup();
    show({ state: 'setup' });
    expect(screen.getByTestId('wallet-total-card').textContent).toContain(
      'Finish setting up',
    );
  });

  it('shows for a ready card even with no tokens, and not otherwise', () => {
    show(ready, 0, false);
    expect(screen.getByTestId('wallet-total-amount').textContent).toBe(
      '$110.00',
    );
    cleanup();
    show({ state: 'signedOut' }, 0, false);
    expect(screen.queryByTestId('wallet-total')).toBeNull();
  });

  it('marks a frozen card', () => {
    show({ ...ready, frozen: true });
    expect(screen.getByTestId('wallet-total-card').textContent).toContain(
      'frozen',
    );
  });
});

describe('reading the card balance', () => {
  const status = {
    configured: true,
    environment: 'sandbox',
    available: true,
    unavailable: null,
    address: '0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266',
    signedIn: true,
  };

  it('never signs in by itself', async () => {
    api.status.mockResolvedValue({ ...status, signedIn: false });
    expect(await readCardBalance()).toEqual({ state: 'signedOut' });
    expect(api.account).not.toHaveBeenCalled();
  });

  it('is nothing in a build without a card', async () => {
    api.status.mockResolvedValue({ ...status, configured: false });
    expect(await readCardBalance()).toEqual({ state: 'none' });
  });

  it('reads the account once signed in', async () => {
    api.status.mockResolvedValue(status);
    api.account.mockResolvedValue({
      accountAddress: null,
      ownerAddress: null,
      status: 'active',
      currency: 'EUR',
      spendable: '12.50',
      pending: '5',
      cards: [
        {
          id: 'c',
          status: 'blocked',
          kind: 'virtual',
          last4: '4242',
          brand: null,
          expires: null,
        },
      ],
    });
    expect(await readCardBalance()).toEqual({
      state: 'ready',
      currency: 'EUR',
      spendable: 12.5,
      pending: 5,
      last4: '4242',
      frozen: true,
    });
  });

  it('is "set up" while Gnosis Pay has no account, and signed out when the session ended', async () => {
    api.status.mockResolvedValue(status);
    api.account.mockRejectedValueOnce({
      kind: 'api',
      message: 'not found',
      code: null,
    });
    expect(await readCardBalance()).toEqual({ state: 'setup' });
    api.account.mockRejectedValueOnce({
      kind: 'signedOut',
      message: 'signed out',
      code: null,
    });
    expect(await readCardBalance()).toEqual({ state: 'signedOut' });
  });

  it('converts only what it has a rate for', () => {
    expect(cardInUsd(10, 'USD', null)).toBe(10);
    expect(cardInUsd(10, 'EUR', 1.1)).toBeCloseTo(11);
    expect(cardInUsd(10, 'EUR', null)).toBeNull();
    expect(cardInUsd(10, 'GBP', 1.1)).toBeNull();
  });
});
