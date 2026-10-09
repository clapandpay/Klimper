// SPDX-License-Identifier: AGPL-3.0-or-later
// Before a combine or split spends coins locked in an open offer, ask (TASK-124). Spending an
// offer's coin is how an offer is cancelled (Sage's own cancel does the same), so the wallet
// does it without harm to the coins: the offer turns cancelled once the spend confirms and its
// other coins are free again. What the user must know is that the offer stops working.
//
// Asked when the form is submitted, right before the spend is built, with the coins read fresh:
// an offer made after the selection is seen too. Nothing is spent without a yes; an error
// reading the coins spends nothing either.

import { commands } from '@/bindings';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { useErrors } from '@/hooks/useErrors';
import { Plural, Trans } from '@lingui/react/macro';
import { LockIcon } from 'lucide-react';
import { useCallback, useState } from 'react';

export type CoinAction = 'combine' | 'split';

interface Asking {
  action: CoinAction;
  offers: number;
  go: () => void;
}

export function useOfferCoinsWarning() {
  const { addError } = useErrors();
  const [asking, setAsking] = useState<Asking | null>(null);
  /** Run `go` now, or after the user agreed to cancel the offers holding these coins. */
  const check = useCallback(
    (action: CoinAction, coinIds: string[], go: () => void) => {
      commands
        .getCoinsByIds({ coin_ids: coinIds })
        .then(({ coins }) => {
          const offers = new Set(
            coins.flatMap((c) => (c.offer_id === null ? [] : [c.offer_id])),
          );
          if (offers.size === 0) go();
          else setAsking({ action, offers: offers.size, go });
        })
        .catch(addError);
    },
    [addError],
  );

  const dialog = (
    <OfferCoinsDialog
      asking={asking}
      onAnswer={(yes) => {
        const go = asking?.go;
        setAsking(null);
        if (yes) go?.();
      }}
    />
  );

  return { check, dialog };
}

function OfferCoinsDialog({
  asking,
  onAnswer,
}: {
  asking: Asking | null;
  onAnswer: (yes: boolean) => void;
}) {
  const offers = asking?.offers ?? 0;
  const combine = asking?.action === 'combine';
  return (
    <Dialog
      open={asking !== null}
      onOpenChange={(open) => !open && onAnswer(false)}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle className='flex items-center gap-2'>
            <LockIcon className='h-4 w-4' aria-hidden='true' />
            {combine ? (
              <Trans>Combine coins locked in an offer?</Trans>
            ) : (
              <Trans>Split coins locked in an offer?</Trans>
            )}
          </DialogTitle>
          <DialogDescription>
            <Plural
              value={offers}
              one='This cancels an open offer that holds some of the selected coins.'
              other='This cancels # open offers that hold some of the selected coins.'
            />{' '}
            <Trans>
              Until this transaction confirms, someone could still take an
              offer; then this transaction fails and nothing is lost. Your coins
              stay yours either way.
            </Trans>
          </DialogDescription>
        </DialogHeader>
        <DialogFooter className='gap-2'>
          <Button variant='outline' onClick={() => onAnswer(false)}>
            <Trans>Keep the offers</Trans>
          </Button>
          <Button variant='destructive' onClick={() => onAnswer(true)}>
            {combine ? (
              <Trans>Cancel offers and combine</Trans>
            ) : (
              <Trans>Cancel offers and split</Trans>
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
