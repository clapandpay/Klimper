// SPDX-License-Identifier: AGPL-3.0-or-later
// Making a partial offer (TASK-164): the assets and amounts come from Sage's Make offer screen
// (its offer state), exactly one token for one token; this screen adds the least one fill
// takes, the fee each fill pays and the expiry, and makes it.

import Container from '@/components/Container';
import { CopyBox } from '@/components/CopyBox';
import Header from '@/components/Header';
import { ReadOnlyButton } from '@/components/ReadOnlyButton';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Label } from '@/components/ui/label';
import { FeeAmountInput, TokenAmountInput } from '@/components/ui/masked-input';
import { useBiometric } from '@/hooks/useBiometric';
import { useErrors } from '@/hooks/useErrors';
import useOfferStateWithDefault from '@/hooks/useOfferStateWithDefault';
import { toDecimal, toMojos } from '@/lib/utils';
import { OfferState } from '@/state';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import BigNumber from 'bignumber.js';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { isPartialError, partialOffers } from './api';
import { PartialOfferQr } from './PartialOfferQr';
import { partialPair } from './pair';
import { amountText, useSideAsset } from './useSideAsset';

function expirySeconds(state: OfferState): number | null {
  if (state.expiration === null) return null;
  const days = parseInt(state.expiration.days) || 0;
  const hours = parseInt(state.expiration.hours) || 0;
  const minutes = parseInt(state.expiration.minutes) || 0;
  const total = days * 86_400 + hours * 3_600 + minutes * 60;
  return total > 0 ? Math.ceil(Date.now() / 1000) + total : null;
}

export default function MakePartialOffer() {
  const [state, setState] = useOfferStateWithDefault();
  const navigate = useNavigate();
  const pair = partialPair(state);

  if (pair === null) {
    return (
      <>
        <Header title={t`Partial offer`} />
        <Container className='max-w-2xl space-y-4'>
          <p className='text-sm text-muted-foreground'>
            <Trans>
              A partial offer gives exactly one token for one other token. Pick
              one of each, with amounts, on the offer screen.
            </Trans>
          </p>
          <Button onClick={() => navigate('/offers/make')}>
            <Trans>Back to the offer</Trans>
          </Button>
        </Container>
      </>
    );
  }
  return (
    <MakeForm
      give={pair.give}
      want={pair.want}
      expiresAt={expirySeconds(state)}
      done={() => {
        setState(null);
        navigate('/offers', { replace: true });
      }}
    />
  );
}

function MakeForm({
  give,
  want,
  expiresAt,
  done,
}: {
  give: OfferState['offered']['tokens'][number];
  want: OfferState['requested']['tokens'][number];
  expiresAt: number | null;
  done: () => void;
}) {
  const navigate = useNavigate();
  const { addError } = useErrors();
  const { promptIfEnabled } = useBiometric();
  const offered = useSideAsset(give.asset_id);
  const requested = useSideAsset(want.asset_id);
  const xch = useSideAsset(null);

  // Sage's offer state holds amounts as the user typed them (12 decimals for XCH, 3 for a
  // CAT, as Sage's own make does).
  const offeredMojos = toMojos(String(give.amount), give.asset_id ? 3 : 12);
  const requestedMojos = toMojos(String(want.amount), want.asset_id ? 3 : 12);
  const [minFill, setMinFill] = useState(() =>
    toDecimal(
      BigNumber(requestedMojos)
        .dividedToIntegerBy(20)
        .integerValue()
        .plus(BigNumber(requestedMojos).lt(20) ? 1 : 0)
        .toFixed(),
      requested.precision,
    ),
  );
  const [feePerFill, setFeePerFill] = useState('');
  const [making, setMaking] = useState(false);
  const [made, setMade] = useState<string | null>(null);

  const giveText = amountText(offeredMojos, offered);
  const wantText = amountText(requestedMojos, requested);
  const expiryText =
    expiresAt === null ? '' : new Date(expiresAt * 1000).toLocaleString();
  const minMojos = BigNumber(toMojos(minFill || '0', requested.precision))
    .integerValue(BigNumber.ROUND_DOWN)
    .toFixed();
  const minValid =
    BigNumber(minMojos).gt(0) && BigNumber(minMojos).lte(requestedMojos);

  const make = async () => {
    if (!(await promptIfEnabled())) return;
    setMaking(true);
    try {
      const fee = BigNumber(toMojos(feePerFill || '0', xch.precision))
        .integerValue(BigNumber.ROUND_DOWN)
        .toFixed();
      const result = await partialOffers.make({
        offered_asset_id: give.asset_id,
        offered_amount: offeredMojos,
        requested_asset_id: want.asset_id,
        requested_amount: requestedMojos,
        min_fill_requested: minMojos,
        required_fee: fee === '0' ? null : fee,
        expires_at: expiresAt,
      });
      setMade(result.offer);
    } catch (error) {
      addError({
        kind: 'invalid',
        reason: isPartialError(error) ? error.message : String(error),
      });
    } finally {
      setMaking(false);
    }
  };

  return (
    <>
      <Header title={t`New partial offer`} />
      <Container className='max-w-2xl space-y-4'>
        <Card>
          <CardHeader className='pb-2'>
            <CardTitle className='text-md font-medium'>
              <Trans>Takers can fill it in parts</Trans>
            </CardTitle>
          </CardHeader>
          <CardContent className='space-y-4 text-sm'>
            <p>
              <Trans>
                You offer {giveText} for {wantText}. Anyone can take part of it
                at this price until it is all taken, it expires, or you cancel
                it.
              </Trans>
            </p>

            <div className='space-y-1.5'>
              <Label htmlFor='partial-min'>
                <Trans>Least one fill pays</Trans>
              </Label>
              <TokenAmountInput
                id='partial-min'
                value={minFill}
                precision={requested.precision}
                ticker={requested.ticker}
                hideMaxButton
                className='pr-16'
                onValueChange={(values) => setMinFill(values.value)}
              />
              {!minValid && (
                <p className='text-destructive'>
                  <Trans>More than zero, and at most {wantText}.</Trans>
                </p>
              )}
            </div>

            <div className='space-y-1.5'>
              <Label htmlFor='partial-fee-per-fill'>
                <Trans>Fee each fill must pay (optional)</Trans>
              </Label>
              <FeeAmountInput
                id='partial-fee-per-fill'
                className='pr-12'
                onValueChange={(values) => setFeePerFill(values.value)}
              />
              <p className='text-xs text-muted-foreground'>
                <Trans>
                  A network fee the taker pays on every fill, so fills confirm
                  quickly.
                </Trans>
              </p>
            </div>

            <p className='text-xs text-muted-foreground'>
              {expiresAt === null ? (
                <Trans>It does not expire.</Trans>
              ) : (
                <Trans>It expires {expiryText}.</Trans>
              )}{' '}
              <Trans>
                Making it sends nothing: the first taker sends your coins along.
                If you spend those coins before then, the offer is cancelled.
              </Trans>
            </p>
          </CardContent>
        </Card>

        {made === null ? (
          <div className='flex gap-2'>
            <Button variant='outline' onClick={() => navigate('/offers/make')}>
              <Trans>Back</Trans>
            </Button>
            <ReadOnlyButton
              requiresSigning
              disabled={!minValid || making}
              onClick={make}
            >
              <Trans>Create partial offer</Trans>
            </ReadOnlyButton>
          </div>
        ) : (
          <Card>
            <CardHeader className='pb-2'>
              <CardTitle className='text-md font-medium'>
                <Trans>Your partial offer</Trans>
              </CardTitle>
            </CardHeader>
            <CardContent className='space-y-3 text-sm'>
              <p className='text-muted-foreground'>
                <Trans>
                  Scan this code with Klimper, or share the text. It is listed
                  under Offers, where you can follow its fills and cancel it.
                </Trans>
              </p>
              <PartialOfferQr offer={made} />
              <CopyBox title={t`Partial offer`} value={made} />
              <Button onClick={done}>
                <Trans>Done</Trans>
              </Button>
            </CardContent>
          </Card>
        )}
      </Container>
    </>
  );
}
