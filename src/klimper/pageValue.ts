// SPDX-License-Identifier: AGPL-3.0-or-later
// What the open page is worth, in US dollars (clapandpay TASK-111, decision-009).
//
// The Klimper sound's shake answers with the weight of what the user is looking at: a token
// page rattles with that token's value, the wallet page with the total. Pages publish the
// value they already compute for display (one `useEffect` each); the sound module reads it.
// Nothing else does. Deliberately outside src/klimper/sound/ so a page's import pulls in
// these few lines and not the sound chunk.

import { useEffect } from 'react';

let value: number | null = null;
const listeners = new Set<(usd: number | null) => void>();

/** The dollar value of the open page, or null when the page shows none. */
export function pageValue(): number | null {
  return value;
}

export function subscribePageValue(listener: (usd: number | null) => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function publish(usd: number | null) {
  const next = usd !== null && Number.isFinite(usd) && usd >= 0 ? usd : null;
  if (next === value) return;
  value = next;
  for (const listener of listeners) listener(value);
}

/**
 * A page says what it is worth while it is mounted, and takes it back when it leaves.
 * `usd` may be `NaN` or `undefined` while prices load: that counts as nothing yet.
 */
export function usePageValue(usd: number | null | undefined) {
  useEffect(() => {
    publish(usd ?? null);
    return () => publish(null);
  }, [usd]);
}

/** Tests only. */
export function resetPageValue() {
  value = null;
  listeners.clear();
}
