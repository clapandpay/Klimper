// SPDX-License-Identifier: AGPL-3.0-or-later
// The card's number, expiry and CVV (TASK-153), to pay online and to add the card to
// Apple Pay or Google Pay (TASK-157).
//
// Shown only after Face ID or Touch ID (when the wallet requires it, as for any signature),
// fetched fresh each time, hidden again after a minute or when the screen closes. Nothing is
// kept: the details live in this component's state and nowhere else.

import { Button } from '@/components/ui/button';
import { useBiometric } from '@/hooks/useBiometric';
import { Trans } from '@lingui/react/macro';
import { Eye, EyeOff, Loader2 } from 'lucide-react';
import { ReactNode, useEffect, useState } from 'react';
import { card, cardErrorMessage, CardDetails as Details } from './card';

export const DETAILS_VISIBLE_MS = 60_000;

/** `4000000000004242` → `4000 0000 0000 4242`. */
export function groupNumber(number: string) {
  return number.replace(/\D/g, '').replace(/(.{4})(?=.)/g, '$1 ');
}

export function CardDetails({
  cardId,
  children,
}: {
  readonly cardId: string;
  /** Shown under the details while they are visible (the Apple Pay guide). */
  readonly children?: (details: Details) => ReactNode;
}) {
  const { promptIfEnabled } = useBiometric();
  const [details, setDetails] = useState<Details | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (details === null) return;
    const timer = setTimeout(() => setDetails(null), DETAILS_VISIBLE_MS);
    return () => clearTimeout(timer);
  }, [details]);

  // Hidden when Klimper goes to the background, so the app switcher's snapshot shows none.
  useEffect(() => {
    const hide = () => {
      if (document.visibilityState === 'hidden') setDetails(null);
    };
    document.addEventListener('visibilitychange', hide);
    return () => document.removeEventListener('visibilitychange', hide);
  }, []);

  const show = async () => {
    setBusy(true);
    setError(null);
    try {
      if (!(await promptIfEnabled())) return;
      setDetails(await card.details(cardId));
    } catch (e) {
      setError(cardErrorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  if (details === null) {
    return (
      <div className='space-y-2'>
        {error && (
          <div className='text-sm text-destructive' role='alert'>
            {error}
          </div>
        )}
        <Button
          size='sm'
          variant='outline'
          disabled={busy}
          onClick={show}
          data-testid='card-show-details'
        >
          {busy ? (
            <Loader2 className='mr-2 h-4 w-4 animate-spin' aria-hidden='true' />
          ) : (
            <Eye className='mr-2 h-4 w-4' aria-hidden='true' />
          )}
          <Trans>Show card details</Trans>
        </Button>
      </div>
    );
  }

  return (
    <div className='space-y-3' data-testid='card-details'>
      <div className='rounded-lg border p-3'>
        <div className='text-xs text-muted-foreground'>
          <Trans>Card number</Trans>
        </div>
        <div
          className='font-mono text-lg tracking-wider'
          data-testid='card-number'
        >
          {groupNumber(details.number)}
        </div>
        <div className='mt-2 flex gap-6'>
          <div>
            <div className='text-xs text-muted-foreground'>
              <Trans>Expires</Trans>
            </div>
            <div className='font-mono' data-testid='card-expires'>
              {details.expires}
            </div>
          </div>
          <div>
            <div className='text-xs text-muted-foreground'>
              <Trans>Security code</Trans>
            </div>
            <div className='font-mono' data-testid='card-cvv'>
              {details.cvv}
            </div>
          </div>
        </div>
      </div>
      {children?.(details)}
      <Button
        size='sm'
        variant='ghost'
        onClick={() => setDetails(null)}
        data-testid='card-hide-details'
      >
        <EyeOff className='mr-2 h-4 w-4' aria-hidden='true' />
        <Trans>Hide</Trans>
      </Button>
    </div>
  );
}
