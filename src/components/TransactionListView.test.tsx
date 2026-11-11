// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// Klimper: the transaction list shows each transaction's date once, on its first row, and
// every row of a confirmed transaction opens it.

import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import type { TransactionRecord } from '../bindings';

vi.mock('@/hooks/useNetwork', () => ({
  useNetwork: () => ({
    network: 'mainnet',
    isTestnet: false,
    isUnknown: false,
  }),
}));
vi.mock('./AddressLabels', () => ({ AddressLabels: () => null }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));
const navigate = vi.fn();
vi.mock('react-router-dom', async (original) => ({
  ...(await original<typeof import('react-router-dom')>()),
  useNavigate: () => navigate,
}));

const { renderScreen } = await import('../klimper/testHarness');
const { TransactionListView } = await import('./TransactionListView');

afterEach(() => {
  cleanup();
  navigate.mockClear();
});

const xch = {
  asset_id: null,
  name: 'Chia',
  ticker: 'XCH',
  precision: 12,
  kind: 'token',
  icon_url: null,
  description: null,
  is_sensitive_content: false,
  is_visible: true,
  revocation_address: null,
  hidden_puzzle_hash: null,
} as unknown as TransactionRecord['spent'][number]['asset'];

function coin(id: string, amount: number) {
  return {
    coin_id: id.repeat(64).slice(0, 64),
    amount,
    address: null,
    address_kind: 'own',
    asset: xch,
  } as unknown as TransactionRecord['spent'][number];
}

const transactions: TransactionRecord[] = [
  {
    height: 200,
    timestamp: null,
    spent: [coin('a', 5)],
    created: [coin('b', 2), coin('c', 1)],
  },
  { height: 100, timestamp: null, spent: [], created: [coin('d', 7)] },
];

it('shows each transaction once and opens it from any of its rows', async () => {
  renderScreen(
    <TransactionListView transactions={transactions} summarized={false} />,
  );

  await waitFor(() => expect(screen.getAllByRole('row').length).toBe(5));
  // One link per transaction, on its first row only.
  expect(screen.getAllByRole('link', { name: '200' })).toHaveLength(1);
  expect(screen.getAllByRole('link', { name: '100' })).toHaveLength(1);

  // The third data row is the first transaction's last coin.
  const rows = screen.getAllByRole('row').slice(1);
  fireEvent.click(rows[2].querySelector('td:nth-child(3)') ?? rows[2]);
  expect(navigate).toHaveBeenCalledWith('/transactions/200');
  fireEvent.click(rows[3].querySelector('td:nth-child(3)') ?? rows[3]);
  expect(navigate).toHaveBeenLastCalledWith('/transactions/100');
});
