// SPDX-License-Identifier: AGPL-3.0-or-later
// A partial offer, read from its text and followed on the chain to its newest coin: its terms,
// and taking an amount of it (TASK-164). Reached from every place an offer can be opened
// (scan, paste, NFC, a link): `routes.tsx` sends a `partial1…` there instead of Sage's page.

import Container from '@/components/Container';
import { CopyBox } from '@/components/CopyBox';
import Header from '@/components/Header';
import { Loading } from '@/components/Loading';
import { ReadOnlyButton } from '@/components/ReadOnlyButton';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Label } from '@/components/ui/label';
import { FeeAmountInput, TokenAmountInput } from '@/components/ui/masked-input';
import { useWallet } from '@/contexts/WalletContext';
import { useBiometric } from '@/hooks/useBiometric';
import { useErrors } from '@/hooks/useErrors';
import { toDecimal, toMojos } from '@/lib/utils';
import { useWalletState } from '@/state';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import BigNumber from 'bignumber.js';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { toast } from 'react-toastify';
import {
  isPartialError,
  viewErrorText,
  partialOffers,
  PartialOfferView,
  TakeResult,
} from './api';
import { clampPayment, price, quote, trimPayment } from './math';
import { PartialOfferQrButton } from './PartialOfferQr';
import { PartialOfferTerms } from './PartialOfferTerms';
import { amountText, useSideAsset } from './useSideAsset';

/** Mojos from what the user typed, or null if it is not an amount. */
function mojos(text: string, precision: number): bigint | null {
  if (text.trim() === '') return null;
  const value = BigNumber(toMojos(text, precision)).integerValue(
    BigNumber.ROUND_DOWN,
  );
  return value.isFinite() && value.gte(0) ? BigInt(value.toFixed()) : null;
}

export default function PartialOfferPage() {
  const { offer: param } = useParams();
  const navigate = useNavigate();
  const { addError } = useErrors();
  const [view, setView] = useState<PartialOfferView | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(
    async (text: string) => {
      setLoading(true);
      try {
        setView(await partialOffers.view(text));
      } catch (error) {
        addError({ kind: 'invalid', reason: viewErrorText(error) });
        navigate('/offers');
      } finally {
        setLoading(false);
      }
    },
    [addError, navigate],
  );

  useEffect(() => {
    // React Router has already decoded the parameter.
    if (param) load(param.trim());
  }, [param, load]);

  return (
    <>
      <Header title={t`Partial offer`} />
      <Container className='max-w-2xl space-y-4'>
        {loading || view === null ? (
          <Loading className='my-8' text={t`Following the offer…`} />
        ) : (
          <>
            <PartialOfferTerms view={view} />
            {(view.status === 'open' || view.status === 'partly_filled') && (
              <TakePanel view={view} reload={() => load(view.offer)} />
            )}
            <Card>
              <CardHeader className='pb-2'>
                <CardTitle className='text-md font-medium'>
                  <Trans>Share</Trans>
                </CardTitle>
              </CardHeader>
              <CardContent className='text-sm text-muted-foreground'>
                <p className='mb-2'>
                  <Trans>
                    The newest text of this offer. After each fill there is a
                    new one; this wallet finds it from any older one.
                  </Trans>
                </p>
                <CopyBox title={t`Partial offer`} value={view.offer} />
                <div className='mt-2'>
                  <PartialOfferQrButton offer={view.offer} />
                </div>
              </CardContent>
            </Card>
          </>
        )}
      </Container>
    </>
  );
}

function TakePanel({
  view,
  reload,
}: {
  view: PartialOfferView;
  reload: () => void;
}) {
  const navigate = useNavigate();
  const { addError } = useErrors();
  const { isReadOnly } = useWallet();
  const { promptIfEnabled } = useBiometric();
  const unit = useWalletState().sync.unit;
  const offered = useSideAsset(view.offered.asset_id);
  const requested = useSideAsset(view.requested.asset_id);
  const xch = useSideAsset(null);

  const p = useMemo(() => price(view), [view]);
  const min = BigInt(view.min_fill_requested);
  const max = BigInt(view.remaining_requested);
  const requiredFee = BigInt(view.required_fee ?? '0');

  const [amount, setAmount] = useState(() =>
    toDecimal(min.toString(), requested.precision),
  );
  const [fee, setFee] = useState('');
  const [confirming, setConfirming] = useState(false);
  const [taking, setTaking] = useState(false);
  const [taken, setTaken] = useState<TakeResult | null>(null);

  const typed = mojos(amount, requested.precision);
  const paid = typed === null ? null : trimPayment(p, typed);
  const valid = paid !== null && paid >= min && paid <= max && max >= min;
  const bought = paid === null ? 0n : quote(p, paid);
  const networkFee = mojos(fee || '0', unit.precision) ?? 0n;
  const payingXch = view.requested.asset_id === null;

  const setPaid = (value: bigint) =>
    setAmount(
      toDecimal(clampPayment(value, min, max).toString(), requested.precision),
    );
  const share = (part: bigint) => setPaid((max * part) / 100n);

  const take = async () => {
    if (paid === null || isReadOnly) return;
    if (!(await promptIfEnabled())) return;
    setTaking(true);
    try {
      setTaken(
        await partialOffers.take(
          view.offer,
          paid.toString(),
          networkFee.toString(),
        ),
      );
    } catch (error) {
      if (isPartialError(error) && error.code === 'offer_changed') {
        toast.info(
          t`Someone took from this offer first. Here is what is left now.`,
        );
        reload();
      } else {
        addError({
          kind: 'invalid',
          reason: isPartialError(error) ? error.message : String(error),
        });
      }
    } finally {
      setTaking(false);
      setConfirming(false);
    }
  };

  const minText = amountText(min, requested);
  const maxText = amountText(max, requested);
  const paysText = [
    amountText(paid ?? 0n, requested),
    ...(requiredFee > 0n ? [amountText(requiredFee, xch)] : []),
    ...(networkFee > 0n ? [amountText(networkFee, xch)] : []),
  ].join(' + ');
  const getsText = amountText(bought, offered);

  if (taken !== null) {
    const paidText = amountText(taken.paid, requested);
    const boughtText = amountText(taken.bought, offered);
    return (
      <Card>
        <CardHeader className='pb-2'>
          <CardTitle className='text-md font-medium'>
            <Trans>Sent</Trans>
          </CardTitle>
        </CardHeader>
        <CardContent className='space-y-3 text-sm'>
          <p>
            <Trans>
              You pay {paidText} and receive {boughtText} once the transaction
              is confirmed.
            </Trans>
          </p>
          <CopyBox
            title={t`Transaction`}
            value={taken.transaction_id}
            truncateMiddle
          />
          {taken.next_offer && (
            <>
              <p className='text-muted-foreground'>
                <Trans>What is left is offered on as a new text:</Trans>
              </p>
              <CopyBox title={t`Next offer`} value={taken.next_offer} />
            </>
          )}
          <Button onClick={() => navigate('/wallet')}>
            <Trans>Done</Trans>
          </Button>
        </CardContent>
      </Card>
    );
  }

  if (max < min) {
    return (
      <Card>
        <CardContent className='pt-6 text-sm text-muted-foreground'>
          <Trans>
            Less is left than the least one fill takes, so nobody can fill it
            any more. Only the maker can take the rest back.
          </Trans>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader className='pb-2'>
        <CardTitle className='text-md font-medium'>
          <Trans>Take part of it</Trans>
        </CardTitle>
      </CardHeader>
      <CardContent className='space-y-4 text-sm'>
        <div className='space-y-1.5'>
          <Label htmlFor='partial-amount'>
            <Trans>You pay</Trans>
          </Label>
          <TokenAmountInput
            id='partial-amount'
            value={amount}
            precision={requested.precision}
            ticker={requested.ticker}
            hideMaxButton
            className='pr-16'
            onValueChange={(values) => setAmount(values.value)}
          />
          <input
            type='range'
            className='w-full accent-primary'
            aria-label={t`Amount`}
            min={0}
            max={1000}
            value={
              paid === null || max === min
                ? 0
                : Number(
                    ((clampPayment(paid, min, max) - min) * 1000n) /
                      (max - min),
                  )
            }
            onChange={(event) =>
              setPaid(min + ((max - min) * BigInt(event.target.value)) / 1000n)
            }
          />
          <div className='flex flex-wrap gap-2'>
            <Button size='sm' variant='outline' onClick={() => setPaid(min)}>
              <Trans>Min</Trans>
            </Button>
            <Button size='sm' variant='outline' onClick={() => share(25n)}>
              25 %
            </Button>
            <Button size='sm' variant='outline' onClick={() => share(50n)}>
              50 %
            </Button>
            <Button size='sm' variant='outline' onClick={() => setPaid(max)}>
              <Trans>Max</Trans>
            </Button>
          </div>
          {!valid && (
            <p className='text-destructive'>
              <Trans>
                Between {minText} and {maxText}.
              </Trans>
            </p>
          )}
        </div>

        <div className='space-y-1.5'>
          <Label htmlFor='partial-fee'>
            <Trans>Network fee</Trans>
          </Label>
          <FeeAmountInput
            id='partial-fee'
            className='pr-12'
            onValueChange={(values) => setFee(values.value)}
          />
        </div>

        <dl className='grid grid-cols-[auto,1fr] gap-x-4 gap-y-1 rounded-md bg-muted p-3'>
          <dt>
            <Trans>You pay</Trans>
          </dt>
          <dd className='text-right font-medium'>
            {amountText(paid ?? 0n, requested)}
          </dd>
          {requiredFee > 0n && (
            <>
              <dt>
                <Trans>Fee per fill (the offer asks)</Trans>
              </dt>
              <dd className='text-right'>{amountText(requiredFee, xch)}</dd>
            </>
          )}
          {networkFee > 0n && (
            <>
              <dt>
                <Trans>Network fee</Trans>
              </dt>
              <dd className='text-right'>{amountText(networkFee, xch)}</dd>
            </>
          )}
          {payingXch && (requiredFee > 0n || networkFee > 0n) && (
            <>
              <dt>
                <Trans>XCH in all</Trans>
              </dt>
              <dd className='text-right'>
                {amountText((paid ?? 0n) + requiredFee + networkFee, xch)}
              </dd>
            </>
          )}
          <dt>
            <Trans>You receive</Trans>
          </dt>
          <dd className='text-right font-medium'>
            {amountText(bought, offered)}
          </dd>
        </dl>

        <ReadOnlyButton
          requiresSigning
          disabled={!valid || taking}
          onClick={() => setConfirming(true)}
        >
          <Trans>Take</Trans>
        </ReadOnlyButton>
      </CardContent>

      <Dialog open={confirming} onOpenChange={setConfirming}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              <Trans>Take part of this offer?</Trans>
            </DialogTitle>
            <DialogDescription>
              <Trans>
                You pay {paysText} and receive {getsText}. The offer is checked
                again right before signing.
              </Trans>
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant='outline' onClick={() => setConfirming(false)}>
              <Trans>Back</Trans>
            </Button>
            <Button disabled={taking} onClick={take}>
              <Trans>Take</Trans>
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  );
}
