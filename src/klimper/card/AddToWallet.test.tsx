// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// Adding the card to Apple Pay or Google Pay (TASK-157): the right guide per platform, and a
// copied number that leaves the clipboard after a minute.

import { act, cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const writeText = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ writeText }));
const os = vi.hoisted(() => ({ name: 'ios' }));
vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => os.name }));

const { renderScreen } = await import('../testHarness');
const { AddToWallet, CLIPBOARD_CLEAR_MS } = await import('./AddToWallet');

const DETAILS = { number: '4000 0000 0000 4242', cvv: '737', expires: '10/29' };

beforeEach(() => {
  vi.resetAllMocks();
  writeText.mockResolvedValue(undefined);
  os.name = 'ios';
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it('guides through Apple Wallet on an iPhone', () => {
  renderScreen(<AddToWallet details={DETAILS} />);
  expect(screen.getByText('Add to Apple Pay')).toBeTruthy();
  expect(screen.getByText(/Open the Wallet app/)).toBeTruthy();
});

it('guides through Google Wallet on Android', () => {
  os.name = 'android';
  renderScreen(<AddToWallet details={DETAILS} />);
  expect(screen.getByText('Add to Google Pay')).toBeTruthy();
  expect(screen.getByText(/Open Google Wallet/)).toBeTruthy();
});

it('copies the digits and clears the clipboard a minute later, even after leaving', async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  renderScreen(<AddToWallet details={DETAILS} />);
  fireEvent.click(screen.getByTestId('card-copy-number'));
  await screen.findByText('Copied; cleared again in a minute');
  expect(writeText).toHaveBeenCalledWith('4000000000004242');
  cleanup();
  await act(() => vi.advanceTimersByTimeAsync(CLIPBOARD_CLEAR_MS));
  expect(writeText).toHaveBeenLastCalledWith('');
});
