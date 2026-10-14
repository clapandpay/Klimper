// SPDX-License-Identifier: AGPL-3.0-or-later
// The fee of the requests Klimper builds and sends itself for a website (`chia_send`, an
// offer take, the option methods) is the wallet's own: the default fee the user set in the
// wallet's settings, never the `fee` a website asks for (xchonnect spec 11.1.1, 11.2). The
// plugin reads it from here, whenever it changes.
import { useDefaultFee } from '@/hooks/useDefaultFee';
import { xchonnectEnabled } from '@/lib/features';
import BigNumber from 'bignumber.js';
import { useEffect } from 'react';
import { xchonnect } from './xchonnect';
import { XCH_PRECISION } from './xchonnectPrompt';

/** The default fee in mojos, a whole number; `null` when it is not a fee. */
export function feeMojos(fee: string): string | null {
  const value = BigNumber(fee || '0');
  if (!value.isFinite() || value.isNegative()) return null;
  return value
    .shiftedBy(XCH_PRECISION)
    .integerValue(BigNumber.ROUND_DOWN)
    .toFixed();
}

export function useOwnFeeSync() {
  const { fee } = useDefaultFee();
  useEffect(() => {
    if (!xchonnectEnabled) return;
    const mojos = feeMojos(fee);
    if (mojos === null) return;
    void xchonnect.setOwnFee(mojos).catch(() => undefined);
  }, [fee]);
}
