// SPDX-License-Identifier: AGPL-3.0-or-later
// The signed-in part of the card screen: onboarding until Gnosis Pay says it is complete
// (TASK-150), then the account (TASK-151).
//
// Gnosis Pay has no push to a wallet without a partner backend, so the steps where it is
// working are polled while this screen is open, and read again when Klimper comes back to
// the front (after the identity check in the browser, for example).

import { Button } from '@/components/ui/button';
import { Trans } from '@lingui/react/macro';
import { Loader2, LogOut } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import {
  card,
  cardErrorMessage,
  isCardError,
  Step,
  WAITING_STEPS,
} from './card';
import { CardAccount } from './CardAccount';
import { CardOnboarding } from './CardOnboarding';

export const POLL_MS = 5000;

export function CardHome({
  onSignOut,
  onSignedOut,
}: {
  readonly address: string;
  readonly onSignOut: () => void;
  /** The session ended on its own (expired, revoked): back to the sign-in. */
  readonly onSignedOut: () => void;
}) {
  const [step, setStep] = useState<Step | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setStep(await card.onboarding());
      setError(null);
    } catch (e) {
      if (isCardError(e) && e.kind === 'signedOut') {
        onSignedOut();
        return;
      }
      setError(cardErrorMessage(e));
    }
  }, [onSignedOut]);

  useEffect(() => {
    void load();
    const onVisible = () => {
      if (document.visibilityState === 'visible') void load();
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => document.removeEventListener('visibilitychange', onVisible);
  }, [load]);

  const waiting = step !== null && WAITING_STEPS.has(step.step);
  useEffect(() => {
    if (!waiting) return;
    const timer = setInterval(() => void load(), POLL_MS);
    return () => clearInterval(timer);
  }, [waiting, load]);

  return (
    <div className='space-y-4' data-testid='card-home'>
      {error && (
        <div
          className='rounded-lg border border-destructive p-3 text-sm text-destructive'
          role='alert'
        >
          {error}
        </div>
      )}
      {step === null ? (
        !error && (
          <div className='flex justify-center p-8'>
            <Loader2
              className='h-6 w-6 animate-spin text-muted-foreground'
              aria-hidden='true'
            />
          </div>
        )
      ) : step.step === 'completed' ? (
        <CardAccount onSignedOut={onSignedOut} />
      ) : (
        <CardOnboarding step={step} onChanged={load} />
      )}
      <div className='flex justify-end'>
        <Button
          variant='ghost'
          size='sm'
          onClick={onSignOut}
          data-testid='card-sign-out'
        >
          <LogOut className='mr-2 h-4 w-4' aria-hidden='true' />
          <Trans>Sign out of Gnosis Pay</Trans>
        </Button>
      </div>
    </div>
  );
}
