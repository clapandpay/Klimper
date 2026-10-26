// SPDX-License-Identifier: AGPL-3.0-or-later
// A partial offer's terms: what it gives for what, how much is left, the price both ways, the
// least one fill takes, the fee each fill pays, the expiry, the maker and the coin.

import { CopyBox } from '@/components/CopyBox';
import { Badge } from '@/components/ui/badge';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { toDecimal } from '@/lib/utils';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import BigNumber from 'bignumber.js';
import { TriangleAlert } from 'lucide-react';
import { useEffect, useState } from 'react';
import { PartialOfferView, PartialStatus } from './api';
import { percent } from './math';
import { amountText, SideAsset, useSideAsset } from './useSideAsset';

export function statusLabel(status: PartialStatus): string {
  switch (status) {
    case 'open':
      return t`Open`;
    case 'partly_filled':
      return t`Partly filled`;
    case 'filled':
      return t`Filled`;
    case 'cancelled':
      return t`Cancelled`;
    case 'expired':
      return t`Expired`;
    case 'unknown':
    default:
      return t`Status unknown (offline)`;
  }
}

/** "1 SBX = 0.25 XCH": `per` of one asset in the other, both ways. */
function priceText(
  from: SideAsset,
  fromMojos: string,
  to: SideAsset,
  toMojos: string,
): string {
  const one = BigNumber(toDecimal(toMojos, to.precision)).dividedBy(
    toDecimal(fromMojos, from.precision),
  );
  return `1 ${from.ticker} = ${one.decimalPlaces(Math.max(to.precision, 4), BigNumber.ROUND_DOWN).toString()} ${to.ticker}`;
}

function useNow(everyMs: number) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(timer);
  }, [everyMs]);
  return now;
}

function countdown(seconds: number): string {
  if (seconds <= 0) return t`expired`;
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  if (days > 0) return t`in ${days} d ${hours} h`;
  if (hours > 0) return t`in ${hours} h ${minutes} min`;
  const atLeastOne = Math.max(minutes, 1);
  return t`in ${atLeastOne} min`;
}

export function FilledBar({ view }: { view: PartialOfferView }) {
  const initial = BigInt(view.initial_offered);
  const left = BigInt(view.remaining_offered);
  const filled = percent(initial - left, initial);
  return (
    <div
      className='h-2 w-full overflow-hidden rounded-full bg-muted'
      role='progressbar'
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={filled}
      aria-label={t`Filled`}
    >
      <div className='h-full bg-primary' style={{ width: `${filled}%` }} />
    </div>
  );
}

export function PartialOfferTerms({ view }: { view: PartialOfferView }) {
  const offered = useSideAsset(view.offered.asset_id);
  const requested = useSideAsset(view.requested.asset_id);
  const xch = useSideAsset(null);
  const now = useNow(30_000);
  const initial = BigInt(view.initial_offered);
  const left = BigInt(view.remaining_offered);
  const filled = percent(initial - left, initial);
  const initialOffered = amountText(view.initial_offered, offered);
  const initialRequested = amountText(view.initial_requested, requested);
  const fillsSeen = view.fills.length;

  return (
    <Card>
      <CardHeader className='flex flex-row items-center justify-between gap-2 space-y-0 pb-2'>
        <CardTitle className='text-md font-medium'>
          <Trans>Partial offer</Trans>
        </CardTitle>
        <div className='flex gap-1'>
          <Badge variant='secondary'>{statusLabel(view.status)}</Badge>
          {view.is_mine && (
            <Badge variant='outline'>
              <Trans>Yours</Trans>
            </Badge>
          )}
        </div>
      </CardHeader>
      <CardContent className='space-y-4 text-sm'>
        <div className='grid grid-cols-2 gap-3'>
          <div>
            <div className='text-muted-foreground'>
              <Trans>Offered</Trans>
            </div>
            <div className='font-medium break-words'>
              {amountText(view.remaining_offered, offered)}
            </div>
            <div className='text-xs text-muted-foreground'>
              <Trans>of {initialOffered}</Trans>
            </div>
          </div>
          <div>
            <div className='text-muted-foreground'>
              <Trans>Requested</Trans>
            </div>
            <div className='font-medium break-words'>
              {amountText(view.remaining_requested, requested)}
            </div>
            <div className='text-xs text-muted-foreground'>
              <Trans>of {initialRequested}</Trans>
            </div>
          </div>
        </div>

        <div className='space-y-1'>
          <FilledBar view={view} />
          <div className='text-xs text-muted-foreground'>
            <Trans>{filled}% filled</Trans>
            {view.fills.length > 0 && (
              <>
                {' · '}
                <Trans>{fillsSeen} fills seen</Trans>
              </>
            )}
          </div>
        </div>

        <dl className='grid grid-cols-[auto,1fr] gap-x-4 gap-y-1'>
          <dt className='text-muted-foreground'>
            <Trans>Price</Trans>
          </dt>
          <dd className='break-words'>
            {priceText(
              offered,
              view.initial_offered,
              requested,
              view.initial_requested,
            )}
            <br />
            {priceText(
              requested,
              view.initial_requested,
              offered,
              view.initial_offered,
            )}
          </dd>
          <dt className='text-muted-foreground'>
            <Trans>Least per fill</Trans>
          </dt>
          <dd className='break-words'>
            {amountText(view.min_fill_requested, requested)} →{' '}
            {amountText(view.min_fill_offered, offered)}
          </dd>
          <dt className='text-muted-foreground'>
            <Trans>Fee per fill</Trans>
          </dt>
          <dd>
            {view.required_fee ? (
              amountText(view.required_fee, xch)
            ) : (
              <Trans>None</Trans>
            )}
          </dd>
          <dt className='text-muted-foreground'>
            <Trans>Expires</Trans>
          </dt>
          <dd>
            {view.expires_at === null ? (
              <Trans>Never</Trans>
            ) : (
              <>
                {new Date(view.expires_at * 1000).toLocaleString()} (
                {countdown(view.expires_at - Math.floor(now / 1000))})
              </>
            )}
          </dd>
        </dl>

        {(view.offered.revocable || view.requested.revocable) && (
          <div className='flex gap-2 rounded-md border border-yellow-500/50 p-2 text-xs'>
            <TriangleAlert className='h-4 w-4 shrink-0 text-yellow-600' />
            <Trans>
              One of these tokens is revocable: its issuer can take coins of it
              back, also after the trade.
            </Trans>
          </div>
        )}

        <CopyBox title={t`Maker`} value={view.maker_address} truncateMiddle />
        <CopyBox title={t`Coin`} value={view.coin_id} truncateMiddle />
      </CardContent>
    </Card>
  );
}
