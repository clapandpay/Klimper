// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// Combining or splitting coins locked in an offer cancels the offer: nothing is spent before
// the user said yes or when the coins cannot be read, coins outside offers go straight on,
// and every submit asks anew.

import { i18n } from '@lingui/core';
import { I18nProvider } from '@lingui/react';
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CoinAction, useOfferCoinsWarning } from './OfferCoinsWarning';

const offers = vi.hoisted(() => new Map<string, string>());
const failing = vi.hoisted(() => ({ value: false }));
vi.mock('@/bindings', () => ({
  commands: {
    getCoinsByIds: ({ coin_ids }: { coin_ids: string[] }) =>
      failing.value
        ? Promise.reject(new Error('no wallet'))
        : Promise.resolve({
            coins: coin_ids.map((coin_id) => ({
              coin_id,
              offer_id: offers.get(coin_id) ?? null,
            })),
          }),
  },
}));
const errors = vi.hoisted(() => ({ addError: vi.fn() }));
vi.mock('@/hooks/useErrors', () => ({ useErrors: () => errors }));
vi.mock('theme-o-rama', () => ({
  useTheme: () => ({ currentTheme: { mostLike: 'light' } }),
}));

i18n.loadAndActivate({ locale: 'en-US', messages: {} });

let api: ReturnType<typeof useOfferCoinsWarning>;
function Harness() {
  api = useOfferCoinsWarning();
  return api.dialog;
}

function run(action: CoinAction, ids: string[]) {
  const go = vi.fn();
  api.check(action, ids, go);
  return go;
}

beforeEach(() => {
  offers.clear();
  failing.value = false;
  render(
    <I18nProvider i18n={i18n}>
      <Harness />
    </I18nProvider>,
  );
});
afterEach(cleanup);

describe('the offer coins warning', () => {
  it('goes straight on with coins outside offers', async () => {
    const go = run('combine', ['a', 'b']);
    await waitFor(() => expect(go).toHaveBeenCalledOnce());
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('asks before using a coin in an offer, and does nothing on no', async () => {
    offers.set('b', 'offer1');
    const go = run('combine', ['a', 'b']);
    await screen.findByText('Combine coins locked in an offer?');
    expect(go).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText('Keep the offers'));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(go).not.toHaveBeenCalled();
  });

  it('goes on after yes, and asks again on the next submit', async () => {
    offers.set('b', 'offer1');
    const first = run('split', ['b']);
    await screen.findByText('Split coins locked in an offer?');
    screen.getByText(/cancels an open offer/);
    fireEvent.click(screen.getByText('Cancel offers and split'));
    await waitFor(() => expect(first).toHaveBeenCalledOnce());

    const second = run('split', ['b']);
    await screen.findByText('Split coins locked in an offer?');
    expect(second).not.toHaveBeenCalled();
  });

  it('counts the offers, not the coins', async () => {
    offers.set('a', 'offer1');
    offers.set('b', 'offer1');
    offers.set('c', 'offer2');
    run('combine', ['a', 'b', 'c']);
    await screen.findByText(/cancels 2 open offers/);
  });

  it('spends nothing when the coins cannot be read', async () => {
    failing.value = true;
    const go = run('combine', ['a']);
    await waitFor(() => expect(errors.addError).toHaveBeenCalled());
    expect(go).not.toHaveBeenCalled();
  });
});
