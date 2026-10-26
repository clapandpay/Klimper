// SPDX-License-Identifier: AGPL-3.0-or-later
// The one element in Sage's Offers screen (TASK-164): the partial offers this wallet made,
// each followed on the chain when shown: how often and how far it is filled, what is left,
// and cancelling it (claw back).

import { CopyButton } from '@/components/CopyButton';
import { ReadOnlyButton } from '@/components/ReadOnlyButton';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Label } from '@/components/ui/label';
import { FeeAmountInput } from '@/components/ui/masked-input';
import { useBiometric } from '@/hooks/useBiometric';
import { useErrors } from '@/hooks/useErrors';
import { partialOffersEnabled } from '@/lib/features';
import { toMojos } from '@/lib/utils';
import { useWalletState } from '@/state';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import BigNumber from 'bignumber.js';
import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { toast } from 'react-toastify';
import { isPartialError, MyPartialOffer, partialOffers } from './api';
import { percent } from './math';
import { FilledBar, statusLabel } from './PartialOfferTerms';
import { amountText, useSideAsset } from './useSideAsset';

export function MyPartialOffers() {
  const [offers, setOffers] = useState<MyPartialOffer[] | null>(null);
  const { addError } = useErrors();

  const load = useCallback(() => {
    partialOffers
      .list()
      .then(setOffers)
      .catch((error) =>
        addError({
          kind: 'invalid',
          reason: isPartialError(error) ? error.message : String(error),
        }),
      );
  }, [addError]);

  useEffect(() => {
    if (partialOffersEnabled) load();
  }, [load]);

  if (!partialOffersEnabled || offers === null || offers.length === 0) {
    return null;
  }

  return (
    <section className='space-y-2'>
      <h2 className='text-sm font-medium text-muted-foreground'>
        <Trans>Partial offers</Trans>
      </h2>
      {offers.map((offer) => (
        <MyPartialOfferRow key={offer.id} offer={offer} reload={load} />
      ))}
    </section>
  );
}

function MyPartialOfferRow({
  offer,
  reload,
}: {
  offer: MyPartialOffer;
  reload: () => void;
}) {
  const { view } = offer;
  const navigate = useNavigate();
  const { addError } = useErrors();
  const { promptIfEnabled } = useBiometric();
  const unit = useWalletState().sync.unit;
  const offered = useSideAsset(view.offered.asset_id);
  const requested = useSideAsset(view.requested.asset_id);
  const [cancelling, setCancelling] = useState(false);
  const [fee, setFee] = useState('');
  const [busy, setBusy] = useState(false);

  const initial = BigInt(view.initial_offered);
  const left = BigInt(view.remaining_offered);
  const filled = percent(initial - left, initial);
  const fills = offer.fills.length;
  const leftText = amountText(left, offered);
  // Unknown (offline) may still be open on the chain: keep Cancel, never offer to forget it.
  const live =
    view.status === 'open' ||
    view.status === 'partly_filled' ||
    view.status === 'expired' ||
    view.status === 'unknown';

  const cancel = async () => {
    if (!(await promptIfEnabled())) return;
    setBusy(true);
    try {
      await partialOffers.cancel(
        offer.id,
        BigNumber(toMojos(fee || '0', unit.precision))
          .integerValue(BigNumber.ROUND_DOWN)
          .toFixed(),
      );
      toast.success(t`Cancelled. The rest comes back once it is confirmed.`);
      setCancelling(false);
      reload();
    } catch (error) {
      addError({
        kind: 'invalid',
        reason: isPartialError(error) ? error.message : String(error),
      });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className='space-y-2 rounded-md border p-3 text-sm'>
      <button
        type='button'
        className='w-full text-left'
        onClick={() =>
          navigate(`/offers/view/${encodeURIComponent(view.offer)}`)
        }
      >
        <div className='flex flex-wrap items-baseline justify-between gap-2'>
          <span className='font-medium break-words'>
            {amountText(view.initial_offered, offered)} →{' '}
            {amountText(view.initial_requested, requested)}
          </span>
          <span className='text-xs text-muted-foreground'>
            {statusLabel(view.status)}
          </span>
        </div>
        <div className='mt-1 text-xs text-muted-foreground'>
          <Trans>
            Filled {fills}× · {filled}% · {leftText} left
          </Trans>
        </div>
      </button>
      <FilledBar view={view} />
      <div className='flex flex-wrap gap-2'>
        <CopyButton value={view.offer} />
        {live && (
          <ReadOnlyButton
            requiresSigning
            size='sm'
            variant='outline'
            onClick={() => setCancelling(true)}
          >
            <Trans>Cancel (claw back)</Trans>
          </ReadOnlyButton>
        )}
        {!live && (
          <Button
            size='sm'
            variant='ghost'
            onClick={() =>
              partialOffers.forget(offer.id).then(reload).catch(addError)
            }
          >
            <Trans>Remove from list</Trans>
          </Button>
        )}
      </div>

      <Dialog open={cancelling} onOpenChange={setCancelling}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              <Trans>Cancel this partial offer?</Trans>
            </DialogTitle>
            <DialogDescription>
              <Trans>
                {leftText} comes back to you. Fills that already happened stay.
                Cancelling is a transaction, so it can pay a network fee.
              </Trans>
            </DialogDescription>
          </DialogHeader>
          <div className='space-y-1.5'>
            <Label htmlFor={`partial-cancel-fee-${offer.id}`}>
              <Trans>Network fee</Trans>
            </Label>
            <FeeAmountInput
              id={`partial-cancel-fee-${offer.id}`}
              className='pr-12'
              onValueChange={(values) => setFee(values.value)}
            />
          </div>
          <DialogFooter>
            <Button variant='outline' onClick={() => setCancelling(false)}>
              <Trans>Back</Trans>
            </Button>
            <Button variant='destructive' disabled={busy} onClick={cancel}>
              <Trans>Cancel offer</Trans>
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
