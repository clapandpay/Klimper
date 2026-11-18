// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The home card: the balance, with or without prices, and Send, Receive and Scan one tap away.

import { cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';

const ADDRESS = 'txch1' + 'q'.repeat(58);

vi.mock('@/state', () => ({
  useWalletState: () => ({
    sync: {
      receive_address: ADDRESS,
      selectable_balance: '106000000000000',
      unit: { ticker: 'TXCH', precision: 12 },
    },
  }),
}));
const prices = vi.hoisted(() => ({ xchFiat: 0, currency: 'EUR' }));
vi.mock('./price', () => ({
  lastPrices: () => ({ xchFiat: prices.xchFiat, currency: prices.currency }),
}));
const wallet = vi.hoisted(() => ({ isTransactionDisabled: false }));
vi.mock('@/contexts/WalletContext', () => ({ useWallet: () => wallet }));
vi.mock('./privacy', () => ({ useMaskAmounts: () => false }));
vi.mock('./Scanner', () => ({ SCANNER_ROUTE: '/scanner' }));
vi.mock('@/components/NumberFormat', () => ({
  NumberFormat: ({ value }: { value: unknown }) => <>{String(value)}</>,
}));
vi.mock('@/components/QRCodeDialog', () => ({
  QRCodeDialog: ({
    isOpen,
    qr_code_contents,
  }: {
    isOpen: boolean;
    qr_code_contents: string;
  }) => (isOpen ? <div data-testid='qr'>{qr_code_contents}</div> : null),
}));
const navigate = vi.fn();
vi.mock('react-router-dom', async (original) => ({
  ...(await original<typeof import('react-router-dom')>()),
  useNavigate: () => navigate,
}));

const { renderScreen } = await import('./testHarness');
const { WalletTotal } = await import('./WalletTotal');

afterEach(() => {
  cleanup();
  navigate.mockClear();
  prices.xchFiat = 0;
  prices.currency = 'EUR';
  wallet.isTransactionDisabled = false;
});

it('shows the XCH balance when there are no prices', () => {
  renderScreen(<WalletTotal value={0} currency='USD' />);
  expect(screen.getByText('Spendable balance')).toBeDefined();
  expect(screen.getByText('106')).toBeDefined();
  expect(screen.getByText('TXCH')).toBeDefined();
});

it('shows the total in the display currency when there are prices', () => {
  prices.xchFiat = 20;
  renderScreen(<WalletTotal value={2120} currency='EUR' />);
  expect(screen.getByText('Total balance')).toBeDefined();
  expect(screen.getByText('2120')).toBeDefined();
  // 2120 EUR at 20 EUR per XCH.
  expect(screen.getByText(/≈\s*106\s*TXCH/)).toBeDefined();
});

it('sends XCH and shows the receive address', () => {
  renderScreen(<WalletTotal value={0} currency={null} />);
  fireEvent.click(screen.getByRole('button', { name: /Send/ }));
  expect(navigate).toHaveBeenCalledWith('/wallet/send/xch');
  expect(screen.queryByTestId('qr')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /Receive/ }));
  expect(screen.getByTestId('qr').textContent).toBe(ADDRESS);
});

it('falls back to XCH while the prices are in another currency', () => {
  prices.xchFiat = 20;
  prices.currency = 'USD';
  renderScreen(<WalletTotal value={0} currency='EUR' />);
  expect(screen.getByText('Spendable balance')).toBeDefined();
});

it('cannot send from a watch-only or cold wallet', () => {
  wallet.isTransactionDisabled = true;
  renderScreen(<WalletTotal value={0} currency={null} />);
  const send = screen.getByRole('button', { name: /Send/ });
  expect((send as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(send);
  expect(navigate).not.toHaveBeenCalled();
});
