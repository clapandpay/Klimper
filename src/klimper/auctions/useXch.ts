// SPDX-License-Identifier: AGPL-3.0-or-later
// XCH amounts as the wallet shows them: mojos to the network's unit and back.

import { toDecimal, toMojos } from '@/lib/utils';
import { useWalletState } from '@/state';
import BigNumber from 'bignumber.js';
import { useCallback } from 'react';

export function useXch() {
  const unit = useWalletState().sync.unit;
  const text = (mojos: bigint | string) =>
    `${toDecimal(mojos.toString(), unit.precision)} ${unit.ticker}`;
  /** Mojos from what the user typed, or null if it is not an amount. */
  const mojos = (typed: string): bigint | null => {
    if (typed.trim() === '') return null;
    const value = BigNumber(toMojos(typed, unit.precision)).integerValue(
      BigNumber.ROUND_DOWN,
    );
    return value.isFinite() && value.gte(0) ? BigInt(value.toFixed()) : null;
  };
  const decimal = useCallback(
    (mojos: bigint | string) => toDecimal(mojos.toString(), unit.precision),
    [unit.precision],
  );
  return { unit, text, mojos, decimal };
}
