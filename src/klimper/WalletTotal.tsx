// SPDX-License-Identifier: AGPL-3.0-or-later
// The wallet's home card, on top of the token list: the total in the display face, what it is in
// XCH beneath it, and the three things people come to do, Send, Receive and Scan, one tap away
// on a phone. Without prices (testnet, lookups off) the headline is the XCH balance instead.
// Masked in privacy mode like every amount (NumberFormat). One element in TokenList.tsx.

import { NumberFormat } from '@/components/NumberFormat';
import { useWallet } from '@/contexts/WalletContext';
import { QRCodeDialog } from '@/components/QRCodeDialog';
import { scanEnabled } from '@/lib/features';
import { fromMojos } from '@/lib/utils';
import { useWalletState } from '@/state';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { ArrowDownLeft, ArrowUpRight, ScanLine } from 'lucide-react';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Hint } from './Hint';
import { lastPrices } from './price';
import { useMaskAmounts } from './privacy';
import { SCANNER_ROUTE } from './Scanner';

export function WalletTotal({
  value,
  currency,
}: {
  /** In `currency`, Sage's display currency (Settings, General). */
  readonly value: number;
  /** Null when price lookups are off. */
  readonly currency: string | null;
}) {
  const masked = useMaskAmounts();
  const navigate = useNavigate();
  const { receive_address, selectable_balance, unit } = useWalletState().sync;
  const [receiving, setReceiving] = useState(false);
  // As Sage's own Send (ReadOnlyButton): no sending from a watch-only or cold wallet.
  const { isTransactionDisabled } = useWallet();

  // Prices in another currency (just after a switch, before a fetch answers) count as none.
  const last = lastPrices();
  const xchFiat = last?.currency === currency ? last.xchFiat : 0;
  const priced = currency !== null && xchFiat > 0;
  const inXch = priced ? value / xchFiat : null;
  const canReceive = receive_address !== '' && receive_address !== 'Unknown';

  return (
    <section className='klimper-total' data-testid='wallet-total'>
      <div className='flex items-center gap-1 text-sm font-medium opacity-85'>
        {priced ? (
          <>
            <Trans>Total balance</Trans>
            <Hint>
              <Trans>
                An estimate from current prices. It does not account for
                liquidity.
              </Trans>
            </Hint>
          </>
        ) : (
          <Trans>Spendable balance</Trans>
        )}
      </div>
      <div className='klimper-total-amount'>
        {priced ? (
          <NumberFormat
            value={value}
            style='currency'
            currency={currency}
            maximumFractionDigits={2}
          />
        ) : (
          <>
            <NumberFormat
              value={fromMojos(selectable_balance, unit.precision)}
              maximumFractionDigits={4}
            />{' '}
            <span className='klimper-total-unit'>{unit.ticker}</span>
          </>
        )}
      </div>
      {inXch !== null && !masked && (
        <div className='text-sm opacity-85 tabular-nums'>
          ≈ <NumberFormat value={inXch} maximumFractionDigits={4} />{' '}
          {unit.ticker}
        </div>
      )}

      <div className='klimper-total-actions'>
        <button
          type='button'
          className='klimper-action klimper-action-send'
          disabled={isTransactionDisabled}
          onClick={() => navigate('/wallet/send/xch')}
        >
          <ArrowUpRight aria-hidden='true' />
          <Trans>Send</Trans>
        </button>
        <button
          type='button'
          className='klimper-action klimper-action-receive'
          disabled={!canReceive}
          onClick={() => setReceiving(true)}
        >
          <ArrowDownLeft aria-hidden='true' />
          <Trans>Receive</Trans>
        </button>
        {scanEnabled && (
          <button
            type='button'
            className='klimper-action klimper-action-scan'
            onClick={() => navigate(SCANNER_ROUTE)}
          >
            <ScanLine aria-hidden='true' />
            <Trans>Scan</Trans>
          </button>
        )}
      </div>

      {canReceive && (
        <QRCodeDialog
          isOpen={receiving}
          onClose={setReceiving}
          asset={null}
          qr_code_contents={receive_address}
          title={t`Receive`}
          description={t`Anyone can pay you with this address. It is safe to share.`}
        />
      )}
    </section>
  );
}
