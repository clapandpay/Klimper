// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The sound as it runs: the wallet store changes, and the right tier plays, once
// (clapandpay TASK-111.03).

import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const sync = (balance: string, synced = true, address = 'xch1abc') => ({
  receive_address: address,
  burn_address: 'x',
  selectable_balance: balance,
  unit: { ticker: 'XCH', precision: 12 },
  total_coins: 3,
  synced_coins: synced ? 3 : 1,
  unhardened_derivation_index: 0,
  hardened_derivation_index: 0,
  checked_files: 0,
  total_files: 0,
  database_size: 0,
});

const store = vi.hoisted(() => ({ current: null as unknown }));
vi.mock('@/state', async () => {
  const { create } = await import('zustand');
  const useWalletState = create(() => ({
    sync: {
      receive_address: 'Unknown',
      selectable_balance: '0',
      total_coins: 0,
      synced_coins: 0,
    },
  }));
  store.current = useWalletState;
  return { useWalletState };
});

const price = vi.hoisted(() => vi.fn());
const key = vi.hoisted(() => vi.fn());
vi.mock('@/bindings', () => ({
  commands: { getXchUsdPrice: price, getKey: key },
}));

const played = vi.hoisted(() => vi.fn());
const shake = vi.hoisted(() => vi.fn());
const phone = vi.hoisted(() => ({ value: false }));
vi.mock('./player', () => ({
  playJingle: played,
  setShakeEnabled: shake,
  onPhone: () => phone.value,
}));

const listeners = vi.hoisted(
  () => new Set<(event: { payload: unknown }) => void>(),
);
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (_: string, cb: (event: { payload: unknown }) => void) => {
    listeners.add(cb);
    return () => listeners.delete(cb);
  },
}));

vi.mock('usehooks-ts', () => ({
  useLocalStorage: (_key: string, initial: boolean) => [
    setting.value ?? initial,
    () => undefined,
  ],
}));
const setting = vi.hoisted(() => ({ value: null as boolean | null }));

import { COALESCE_MS, forgetPrice, KlimperSound } from './KlimperSound';

interface Store {
  setState: (s: { sync: ReturnType<typeof sync> }) => void;
}
const wallet = () => store.current as Store;

const settle = async () => {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(COALESCE_MS + 10);
  });
};

describe('the Klimper sound, running', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    played.mockReset();
    shake.mockReset();
    price.mockReset().mockResolvedValue({ usd: 10 });
    forgetPrice();
    key.mockReset().mockResolvedValue({ key: { fingerprint: 1 } });
    listeners.clear();
    setting.value = null;
    phone.value = false;
    wallet().setState({ sync: sync('0', true, 'Unknown') });
  });
  afterEach(() => vi.useRealTimers());

  it('plays the tier of what arrived, once for coins that land together', async () => {
    render(<KlimperSound />);
    await act(async () => {
      wallet().setState({ sync: sync('1000000000000') }); // 1 XCH: the baseline
    });
    await settle();
    expect(played).not.toHaveBeenCalled();

    await act(async () => {
      wallet().setState({ sync: sync('1500000000000') }); // +0.5 XCH = $5
      await vi.advanceTimersByTimeAsync(300);
      wallet().setState({ sync: sync('4000000000000') }); // +2.5 XCH = $25, together $30
    });
    await settle();
    expect(played).toHaveBeenCalledExactlyOnceWith('handful', false);
  });

  it('is quiet for a spend, a catch-up, and a switch to a richer wallet', async () => {
    render(<KlimperSound />);
    await act(async () => {
      wallet().setState({ sync: sync('1000000000000') });
    });
    await settle();
    await act(async () => {
      wallet().setState({ sync: sync('400000000000') }); // spent
      wallet().setState({ sync: sync('900000000000', false) }); // catching up
      wallet().setState({ sync: sync('0', true, 'Unknown') }); // logged out
      wallet().setState({ sync: sync('99000000000000', true, 'xch1rich') }); // other wallet
    });
    await settle();
    expect(played).not.toHaveBeenCalled();
  });

  it('does not play when the fingerprint changed under it', async () => {
    render(<KlimperSound />);
    await act(async () => {
      wallet().setState({ sync: sync('1000000000000') });
    });
    await settle();
    key.mockResolvedValue({ key: { fingerprint: 2 } });
    await act(async () => {
      wallet().setState({ sync: sync('5000000000000') });
    });
    await settle();
    expect(played).not.toHaveBeenCalled();
  });

  it('is a coin without a price', async () => {
    price.mockRejectedValue(new Error('offline'));
    render(<KlimperSound />);
    await act(async () => {
      wallet().setState({ sync: sync('1000000000000') });
    });
    await settle();
    await act(async () => {
      wallet().setState({ sync: sync('900000000000000') });
    });
    await settle();
    expect(played).toHaveBeenCalledExactlyOnceWith('coin', false);
  });

  it('on the phone, a shake plays the whole wallet with haptics; off stops the watch', async () => {
    phone.value = true;
    const { unmount } = render(<KlimperSound />);
    await act(async () => {
      wallet().setState({ sync: sync('30000000000000') }); // 30 XCH = $300
    });
    await settle();
    expect(shake).toHaveBeenCalledWith(true);
    await act(async () => {
      for (const cb of listeners) cb({ payload: { intensity: 0.8 } });
      await vi.advanceTimersByTimeAsync(10);
    });
    expect(played).toHaveBeenCalledExactlyOnceWith('pouch', true);
    unmount();
    expect(shake).toHaveBeenLastCalledWith(false);
  });

  it('does nothing at all when the switch is off', async () => {
    setting.value = false;
    phone.value = true;
    render(<KlimperSound />);
    await act(async () => {
      wallet().setState({ sync: sync('1000000000000') });
      wallet().setState({ sync: sync('9000000000000') });
    });
    await settle();
    expect(played).not.toHaveBeenCalled();
    expect(shake).not.toHaveBeenCalled();
    expect(listeners.size).toBe(0);
  });
});
