// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// An install that had the removed Colorful theme gets the default theme again.

import { expect, it, vi } from 'vitest';

vi.mock('../klimper.css', () => ({}));

it('sends a stored colorful theme back to the default', async () => {
  // In memory: Node's own localStorage global shadows jsdom's and is unusable here.
  const memory = new Map([
    ['theme', 'colorful'],
    ['theme-o-rama-last-used-non-core-theme', 'colorful'],
    ['klimper-theme-seeded-v2', 'light'],
  ]);
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => memory.get(key) ?? null,
    setItem: (key: string, value: string) => void memory.set(key, value),
    removeItem: (key: string) => void memory.delete(key),
  });

  await import('./klimperTheme');

  expect(['light', 'dark']).toContain(memory.get('theme'));
  expect(memory.has('theme-o-rama-last-used-non-core-theme')).toBe(false);
});
