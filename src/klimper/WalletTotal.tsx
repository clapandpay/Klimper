// SPDX-License-Identifier: AGPL-3.0-or-later
// The wallet's total, on top of the token list: one large figure in the display face, what it
// is in XCH beneath it, and "an estimate" behind an info dot. Masked in privacy mode like every
// amount (NumberFormat). One element in TokenList.tsx.

import { NumberFormat } from '@/components/NumberFormat';
import { Trans } from '@lingui/react/macro';
import { lastPrices } from './price';
import { useMaskAmounts } from './privacy';
import { Hint } from './Hint';

export function WalletTotal({
  value,
  currency,
}: {
  /** In `currency`, Sage's display currency (Settings, General). */
  readonly value: number;
  readonly currency: string;
}) {
  const masked = useMaskAmounts();
  const xchFiat = lastPrices()?.xchFiat ?? 0;
  const inXch = xchFiat > 0 ? value / xchFiat : null;
  return (
    <section className='klimper-total' data-testid='wallet-total'>
      <div className='flex items-center gap-1 text-sm font-medium opacity-80'>
        <Trans>Total balance</Trans>
        <Hint>
          <Trans>
            An estimate from current prices. It does not account for liquidity.
          </Trans>
        </Hint>
      </div>
      <div className='klimper-total-amount'>
        <NumberFormat
          value={value}
          style='currency'
          currency={currency}
          maximumFractionDigits={2}
        />
      </div>
      {inXch !== null && !masked && (
        <div className='text-sm opacity-80 tabular-nums'>
          ≈ <NumberFormat value={inXch} maximumFractionDigits={4} /> XCH
        </div>
      )}
    </section>
  );
}
