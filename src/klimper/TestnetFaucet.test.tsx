// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The testnet faucet button: testnet only, copies the address, opens the fixed faucet URL.

import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';

const ADDRESS = 'txch1' + 'q'.repeat(58);

const net = vi.hoisted(() => ({ network: 'testnet' as string | null }));
vi.mock('@/hooks/useNetwork', () => ({
  useNetwork: () => ({ network: net.network }),
}));
const wallet = vi.hoisted(() => ({ receive_address: '' }));
vi.mock('@/state', () => ({
  useWalletState: () => ({ sync: { receive_address: wallet.receive_address } }),
}));
const writeText = vi.hoisted(() => vi.fn((_text: string) => Promise.resolve()));
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ writeText }));
const openUrl = vi.hoisted(() => vi.fn((_url: string) => Promise.resolve()));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl }));
const toast = vi.hoisted(() => ({ info: vi.fn(), error: vi.fn() }));
vi.mock('react-toastify', () => ({ toast }));

const { renderScreen } = await import('./testHarness');
const { TestnetFaucet, TESTNET_FAUCET_URL, isTestnetAddress } =
  await import('./TestnetFaucet');

afterEach(() => {
  cleanup();
  writeText.mockClear();
  openUrl.mockClear();
  toast.info.mockClear();
  toast.error.mockClear();
  net.network = 'testnet';
  wallet.receive_address = ADDRESS;
});
wallet.receive_address = ADDRESS;

it('copies the address and opens the official faucet', async () => {
  renderScreen(<TestnetFaucet />);
  fireEvent.click(screen.getByRole('button', { name: /Get test XCH/ }));
  await waitFor(() =>
    expect(openUrl).toHaveBeenCalledWith('https://testnet11-faucet.chia.net/'),
  );
  expect(writeText).toHaveBeenCalledWith(ADDRESS);
  expect(TESTNET_FAUCET_URL).toBe('https://testnet11-faucet.chia.net/');
});

it.each(['mainnet', 'unknown', null])('shows nothing on %s', (network) => {
  net.network = network;
  renderScreen(<TestnetFaucet />);
  expect(screen.queryByRole('button', { name: /Get test XCH/ })).toBeNull();
});

it('shows nothing without a testnet address', () => {
  wallet.receive_address = 'xch1' + 'q'.repeat(58);
  renderScreen(<TestnetFaucet />);
  expect(screen.queryByRole('button', { name: /Get test XCH/ })).toBeNull();
});

it('accepts only bech32 testnet addresses', () => {
  expect(isTestnetAddress(ADDRESS)).toBe(true);
  expect(isTestnetAddress('Unknown')).toBe(false);
  expect(isTestnetAddress('txch1bio')).toBe(false);
  expect(isTestnetAddress(null)).toBe(false);
});

it('still opens the faucet when copying fails, and says so', async () => {
  writeText.mockRejectedValueOnce(new Error('denied'));
  renderScreen(<TestnetFaucet />);
  fireEvent.click(screen.getByRole('button', { name: /Get test XCH/ }));
  await waitFor(() => expect(openUrl).toHaveBeenCalled());
  expect(toast.info).toHaveBeenCalledWith(
    'Copy your receive address and paste it into the faucet.',
  );
});

it('shows an error when the faucet cannot be opened', async () => {
  openUrl.mockRejectedValueOnce(new Error('no browser'));
  renderScreen(<TestnetFaucet />);
  fireEvent.click(screen.getByRole('button', { name: /Get test XCH/ }));
  await waitFor(() =>
    expect(toast.error).toHaveBeenCalledWith('The faucet could not be opened.'),
  );
});
