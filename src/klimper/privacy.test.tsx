// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// Privacy mode: amounts are masked on pages, shown in dialogs, and the coins go quiet.

import { NumberFormat } from '@/components/NumberFormat';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

// An in-memory localStorage: Node's own global shadows jsdom's and is unusable here.
const memory = new Map<string, string>();
const storage = {
  getItem: (key: string) => memory.get(key) ?? null,
  setItem: (key: string, value: string) => void memory.set(key, String(value)),
  removeItem: (key: string) => void memory.delete(key),
  clear: () => memory.clear(),
  key: (index: number) => [...memory.keys()][index] ?? null,
  get length() {
    return memory.size;
  },
};
Object.defineProperty(window, 'localStorage', {
  value: storage,
  configurable: true,
});
Object.defineProperty(globalThis, 'localStorage', {
  value: storage,
  configurable: true,
});

vi.mock('@/lib/features', () => ({ klimperSoundEnabled: true }));
vi.mock('./haptics', () => ({ onPhone: () => true }));

const { MASK, PRIVACY_KEY, RevealAmounts } = await import('./privacy');
const { KlimperSettings } = await import('./KlimperSettings');
const { renderScreen } = await import('./testHarness');

afterEach(() => {
  cleanup();
  localStorage.clear();
});

function amount() {
  return <NumberFormat value={1234.5} style='currency' currency='USD' />;
}

describe('privacy mode', () => {
  it('shows amounts while it is off', () => {
    render(amount());
    expect(document.body.textContent).toContain('1,234.5');
  });

  it('masks amounts on a page, and shows them inside a confirmation', () => {
    localStorage.setItem(PRIVACY_KEY, 'true');
    render(
      <>
        <div data-testid='page'>{amount()}</div>
        <RevealAmounts>
          <div data-testid='dialog'>{amount()}</div>
        </RevealAmounts>
      </>,
    );
    expect(screen.getByTestId('page').textContent).toBe(MASK);
    expect(screen.getByTestId('dialog').textContent).toContain('1,234.5');
  });

  it('turns the coin sound and feel off while it is on', () => {
    renderScreen(<KlimperSettings />);
    const sound = screen.getByTestId('klimper-sound-switch');
    expect(sound.getAttribute('data-state')).toBe('checked');
    fireEvent.click(screen.getByTestId('klimper-privacy-switch'));
    expect(sound.getAttribute('data-state')).toBe('unchecked');
    expect(sound).toHaveProperty('disabled', true);
  });
});
