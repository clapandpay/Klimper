// SPDX-License-Identifier: AGPL-3.0-or-later
// What the card can spend, for the wallet's total on the Assets page (TASK-158). Read from
// the card plugin while the page is open and never kept. It never signs in by itself:
// signing in is a signature and asks for Face ID, so it only happens on the card screen.

import { cardEnabled } from '@/lib/features';
import { useCallback, useEffect, useState } from 'react';
import { card, isCardError } from './card';

export const CARD_BALANCE_POLL_MS = 30000;

export type CardBalance =
  /** No card in this build, or this wallet cannot have one: nothing is shown. */
  | { state: 'none' }
  | { state: 'signedOut' }
  /** Signed in, but the card issuer has no account yet (onboarding not finished). */
  | { state: 'setup' }
  | {
      state: 'ready';
      currency: string;
      spendable: number;
      /** Top-ups on the way, not spendable yet. */
      pending: number;
      last4: string | null;
      frozen: boolean;
    };

const NONE: CardBalance = { state: 'none' };

function amount(text: string) {
  const value = Number(text);
  return Number.isFinite(value) ? value : 0;
}

export async function readCardBalance(): Promise<CardBalance> {
  const status = await card.status();
  if (!status.configured || !status.available) return NONE;
  if (!status.signedIn) return { state: 'signedOut' };
  try {
    const account = await card.account();
    const usable = account.cards.find((c) => c.status !== 'canceled');
    return {
      state: 'ready',
      currency: account.currency,
      spendable: amount(account.spendable),
      pending: amount(account.pending),
      last4: usable?.last4 ?? null,
      frozen: usable?.status === 'blocked',
    };
  } catch (e) {
    if (isCardError(e) && e.kind === 'signedOut') return { state: 'signedOut' };
    if (isCardError(e) && e.kind === 'api') return { state: 'setup' };
    throw e;
  }
}

export function useCardBalance(): CardBalance {
  const [balance, setBalance] = useState<CardBalance>(NONE);

  const load = useCallback(async () => {
    try {
      setBalance(await readCardBalance());
    } catch {
      // Offline or the plugin is busy: keep what is shown until the next read.
    }
  }, []);

  useEffect(() => {
    if (!cardEnabled) return;
    void load();
    const timer = setInterval(() => void load(), CARD_BALANCE_POLL_MS);
    const onVisible = () => {
      if (document.visibilityState === 'visible') void load();
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, [load]);

  return balance;
}

/** The card's money in USD, for adding it to the wallet's total; null when no rate is known. */
export function cardInUsd(
  value: number,
  currency: string,
  eurUsd: number | null,
): number | null {
  if (currency === 'USD') return value;
  if (currency === 'EUR' && eurUsd !== null) return value * eurUsd;
  return null;
}
