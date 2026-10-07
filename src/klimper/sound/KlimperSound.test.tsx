// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The sound as it runs: the wallet store changes, the right ding rings once, and the native
// side is armed with the wallet's tier (clapandpay TASK-111.03).

import { act, cleanup, render } from '@testing-library/react';
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

const ding = vi.hoisted(() => vi.fn(async () => undefined));
const shake = vi.hoisted(() => vi.fn(async () => undefined));
const phone = vi.hoisted(() => ({ value: false }));
vi.mock('./player', () => ({
  playDing: ding,
  setShake: shake,
  onPhone: () => phone.value,
}));

const setting = vi.hoisted(() => ({ value: null as boolean | null }));
vi.mock('usehooks-ts', () => ({
  useLocalStorage: (_key: string, initial: boolean) => [
    setting.value ?? initial,
    () => undefined,
  ],
}));

import { COALESCE_MS, forgetPrice, KlimperSound } from './KlimperSound';
import { resetPageValue, usePageValue } from '@/klimper/pageValue';

/** A page that shows a dollar value, as Token.tsx and TokenList.tsx do. */
function Page({ usd }: { usd: number | null }) {
  usePageValue(usd);
  return null;
}

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
    ding.mockClear();
    shake.mockClear();
    price.mockReset().mockResolvedValue({ usd: 10 });
    key.mockReset().mockResolvedValue({ key: { fingerprint: 1 } });
    forgetPrice();
    resetPageValue();
    setting.value = null;
    phone.value = false;
    wallet().setState({ sync: sync('0', true, 'Unknown') });
  });
  afterEach(() => {
    cleanup(); // unmount, so a test's subscriptions do not outlive it
    vi.useRealTimers();
  });

  it('rings the ding of what arrived, once for coins that land together', async () => {
    render(<KlimperSound />);
    await act(async () => {
      wallet().setState({ sync: sync('1000000000000') }); // 1 XCH: the baseline
    });
    await settle();
    expect(ding).not.toHaveBeenCalled();

    await act(async () => {
      wallet().setState({ sync: sync('1500000000000', false) }); // a coin, not yet synced
      wallet().setState({ sync: sync('1500000000000') }); // +0.5 XCH = $5
      await vi.advanceTimersByTimeAsync(300);
      wallet().setState({ sync: sync('4000000000000') }); // +2.5 XCH = $25, together $30
    });
    await settle();
    expect(ding).toHaveBeenCalledExactlyOnceWith('handful');
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
    expect(ding).not.toHaveBeenCalled();
  });

  it('does not ring when the fingerprint changed under it', async () => {
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
    expect(ding).not.toHaveBeenCalled();
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
    expect(ding).toHaveBeenCalledExactlyOnceWith('coin');
  });

  it('on the phone, arms the shake with the wallet tier and keeps it current', async () => {
    phone.value = true;
    const { unmount } = render(<KlimperSound />);
    await settle();
    // Armed at once with the lightest tier, before any balance is known.
    expect(shake).toHaveBeenCalledWith(true, 'coin');
    await act(async () => {
      wallet().setState({ sync: sync('30000000000000') }); // 30 XCH = $300
    });
    await settle();
    expect(shake).toHaveBeenLastCalledWith(true, 'pouch');
    // The same balance again, or a change within the tier: nothing is re-sent.
    const calls = shake.mock.calls.length;
    await act(async () => {
      wallet().setState({ sync: sync('30000000000000') });
      wallet().setState({ sync: sync('31000000000000') });
    });
    await settle();
    expect(shake).toHaveBeenCalledTimes(calls);
    // Down to a handful: re-armed with the new tier.
    await act(async () => {
      wallet().setState({ sync: sync('5000000000000') }); // $50
    });
    await settle();
    expect(shake).toHaveBeenLastCalledWith(true, 'handful');
    unmount();
    expect(shake).toHaveBeenLastCalledWith(false, 'handful');
  });

  it('on the phone, the open page sets the weight; without one, the wallet does', async () => {
    phone.value = true;
    const { rerender } = render(
      <>
        <KlimperSound />
        <Page usd={null} />
      </>,
    );
    await act(async () => {
      wallet().setState({ sync: sync('30000000000000') }); // $300 in the wallet
    });
    await settle();
    expect(shake).toHaveBeenLastCalledWith(true, 'pouch');
    // A token page worth $5: the shake is a few coins, whatever the wallet holds.
    rerender(
      <>
        <KlimperSound />
        <Page usd={5} />
      </>,
    );
    await settle();
    expect(shake).toHaveBeenLastCalledWith(true, 'few');
    // Prices still loading on the page: nothing yet, so the wallet again.
    rerender(
      <>
        <KlimperSound />
        <Page usd={Number.NaN} />
      </>,
    );
    await settle();
    expect(shake).toHaveBeenLastCalledWith(true, 'pouch');
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
    expect(ding).not.toHaveBeenCalled();
    expect(shake).not.toHaveBeenCalled();
  });
});
