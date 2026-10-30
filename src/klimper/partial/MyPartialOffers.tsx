// SPDX-License-Identifier: AGPL-3.0-or-later
// Sage's Offers screen shows the partial offers this wallet made as a group of their own
// (TASK-164), under the same filter as its offers. Each is followed on the chain when shown:
// how often and how far it is filled, what is left, and cancelling it (claw back). A cancel
// reads "Cancel pending" until the chain confirms it, and the list refreshes itself meanwhile.

import { events } from '@/bindings';
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
import { Loader2 } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { toast } from 'react-toastify';
import {
  isPartialError,
  MyPartialOffer,
  PartialStatus,
  partialOffers,
} from './api';
import { matchesOfferFilter } from './filter';
import { percent } from './math';
import { FilledBar, statusLabel } from './PartialOfferTerms';
import { PartialOfferQrButton } from './PartialOfferQr';
import { amountText, useSideAsset } from './useSideAsset';

/** How often the list looks again while a cancel waits for the chain. */
const PENDING_POLL_MS = 10_000;
/** After this long a pending cancel may have been dropped: offer to send it again. */
const RETRY_AFTER_S = 10 * 60;

export interface MyPartialOffersState {
  /** Null until loaded, and always null with partial offers off. */
  offers: MyPartialOffer[] | null;
  reload: () => void;
}

/** The wallet's partial offers, reloaded when the wallet's coins change and, while a cancel
 * is pending, every few seconds. Says so when a pending cancel is confirmed or fails. */
export function useMyPartialOffers(): MyPartialOffersState {
  const [offers, setOffers] = useState<MyPartialOffer[] | null>(null);
  const { addError } = useErrors();
  const seen = useRef(new Map<string, PartialStatus>());

  const load = useCallback(() => {
    if (!partialOffersEnabled) return;
    partialOffers
      .list()
      .then((next) => {
        for (const offer of next) {
          const before = seen.current.get(offer.id);
          const now = offer.view.status;
          if (before === 'cancelling' && now === 'cancelled') {
            toast.success(
              t`Partial offer cancelled. The rest is back in your wallet.`,
            );
          } else if (
            before === 'cancelling' &&
            (now === 'open' || now === 'partly_filled')
          ) {
            toast.warning(
              t`The cancel did not go through: someone filled the offer first. Cancel again to take back the rest.`,
            );
          }
          seen.current.set(offer.id, now);
        }
        setOffers(next);
      })
      .catch((error) =>
        addError({
          kind: 'invalid',
          reason: isPartialError(error) ? error.message : String(error),
        }),
      );
  }, [addError]);

  useEffect(() => {
    if (!partialOffersEnabled) return;
    load();
    const unlisten = events.syncEvent.listen((data) => {
      if (data.payload.type === 'coin_state') load();
    });
    return () => {
      unlisten.then((u) => u());
    };
  }, [load]);

  const pending = offers?.some((offer) => offer.view.status === 'cancelling');
  useEffect(() => {
    if (!pending) return;
    const timer = setInterval(load, PENDING_POLL_MS);
    return () => clearInterval(timer);
  }, [pending, load]);

  return { offers, reload: load };
}

/** The partial offers group on the Offers screen, under Sage's status filter. */
export function MyPartialOffers({
  state,
  statusFilter,
}: {
  state: MyPartialOffersState;
  statusFilter: string;
}) {
  const { offers, reload } = state;
  if (!partialOffersEnabled || offers === null || offers.length === 0) {
    return null;
  }
  const shown = offers.filter((offer) =>
    matchesOfferFilter(offer.view.status, statusFilter),
  );

  return (
    <section className='flex flex-col gap-2'>
      <h3 className='text-sm font-medium text-muted-foreground'>
        <Trans>Partial offers</Trans> ({shown.length})
      </h3>
      {shown.length === 0 ? (
        <p className='text-sm text-muted-foreground'>
          <Trans>No partial offers match this filter.</Trans>
        </p>
      ) : (
        shown.map((offer) => (
          <MyPartialOfferRow key={offer.id} offer={offer} reload={reload} />
        ))
      )}
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
  const pending = view.status === 'cancelling' ? offer.cancel : null;
  // A cancel still pending after a while may have been dropped by the network.
  const stuck =
    pending !== null && Date.now() / 1000 - pending.sent_at > RETRY_AFTER_S;

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
      toast.info(
        t`Cancel sent. It shows as cancelled once the network confirms it.`,
      );
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
          <span
            className={
              pending
                ? 'flex items-center gap-1 text-xs font-medium text-amber-600 dark:text-amber-400'
                : 'text-xs text-muted-foreground'
            }
          >
            {pending && (
              <Loader2 className='h-3 w-3 animate-spin' aria-hidden='true' />
            )}
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
      {pending && (
        <div className='rounded-md bg-muted p-2 text-xs text-muted-foreground'>
          <Trans>
            Waiting for the network to confirm the cancel. {leftText} comes back
            to you then.
          </Trans>
          <div className='mt-1 flex items-center gap-1 break-all font-mono'>
            {pending.transaction_id.slice(0, 18)}…
            <CopyButton value={pending.transaction_id} />
          </div>
        </div>
      )}
      <div className='flex flex-wrap gap-2'>
        <CopyButton value={view.offer} />
        {live && <PartialOfferQrButton offer={view.offer} />}
        {(live || stuck) && (
          <ReadOnlyButton
            requiresSigning
            size='sm'
            variant='outline'
            onClick={() => setCancelling(true)}
          >
            {stuck ? (
              <Trans>Cancel again</Trans>
            ) : (
              <Trans>Cancel (claw back)</Trans>
            )}
          </ReadOnlyButton>
        )}
        {!live && !pending && (
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
