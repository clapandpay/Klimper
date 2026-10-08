// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The lock screen's way out when the passkey is lost (TASK-118): nothing is erased until the
// word is typed.

import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const erase = vi.fn().mockResolvedValue(null);
vi.mock('./passkey', () => ({
  passkey: {
    status: () =>
      Promise.resolve({ supported: true, enrolled: true, locked: true }),
    unlock: () => Promise.reject(new Error('cancelled')),
    lock: () => Promise.resolve(null),
    erase: () => erase(),
  },
  PASSKEY_CHANGED: 'klimper-passkey-changed',
}));

const { renderScreen } = await import('./testHarness');
const { PasskeyGate } = await import('./PasskeyGate');

afterEach(() => {
  cleanup();
  erase.mockClear();
});

describe('a lost passkey', () => {
  it('erases the keys only after the word is typed', async () => {
    renderScreen(<PasskeyGate />);
    fireEvent.click(
      await screen.findByRole('button', { name: 'Lost your passkey?' }),
    );
    const button = screen.getByRole('button', { name: 'Erase the keys' });
    expect(button).toHaveProperty('disabled', true);

    fireEvent.change(screen.getByRole('textbox'), {
      target: { value: 'eras' },
    });
    expect(button).toHaveProperty('disabled', true);

    fireEvent.change(screen.getByRole('textbox'), {
      target: { value: 'Erase' },
    });
    fireEvent.click(button);
    await waitFor(() => expect(erase).toHaveBeenCalledTimes(1));
  });

  it('can be cancelled without erasing anything', async () => {
    renderScreen(<PasskeyGate />);
    fireEvent.click(
      await screen.findByRole('button', { name: 'Lost your passkey?' }),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByTestId('erase-keys')).toBeNull();
    expect(erase).not.toHaveBeenCalled();
  });
});
