// SPDX-License-Identifier: AGPL-3.0-or-later
import { Button } from '@/components/ui/button';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { KeyRound } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { passkey, PASSKEY_CHANGED, PasskeyStatus } from './passkey';

/**
 * Rendered next to the app, it covers it while a passkey protects the keys and they are locked. The keys lock again
 * whenever the app leaves the screen, so an unlocked wallet never sits in the background.
 */
export function PasskeyGate() {
  const [status, setStatus] = useState<PasskeyStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const prompted = useRef(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await passkey.status());
    } catch (err) {
      setError(String(err));
      setStatus({ supported: false, enrolled: false, locked: false });
    }
  }, []);

  const unlock = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      await passkey.unlock();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
      await refresh();
    }
  }, [refresh]);

  useEffect(() => {
    void refresh();

    const onVisibility = async () => {
      if (document.visibilityState === 'hidden') {
        await passkey.lock().catch(() => undefined);
        prompted.current = false;
      }
      await refresh();
    };

    document.addEventListener('visibilitychange', onVisibility);
    window.addEventListener(PASSKEY_CHANGED, refresh);
    return () => {
      document.removeEventListener('visibilitychange', onVisibility);
      window.removeEventListener(PASSKEY_CHANGED, refresh);
    };
  }, [refresh]);

  const locked = status === null || (status.enrolled && status.locked);

  // Ask for Face ID once each time the locked app comes to the front.
  useEffect(() => {
    if (
      status?.enrolled &&
      status.locked &&
      !prompted.current &&
      document.visibilityState === 'visible'
    ) {
      prompted.current = true;
      void unlock();
    }
  }, [status, unlock]);

  return (
    locked && (
      <div
        className='fixed inset-0 z-[1000] flex flex-col items-center justify-center gap-6 bg-background p-6 text-center'
        aria-modal='true'
        role='dialog'
        aria-label={t`Klimper is locked`}
      >
        {status !== null && (
          <>
            <KeyRound className='h-12 w-12 text-muted-foreground' />
            <div className='space-y-2'>
              <h1 className='text-2xl font-semibold'>
                <Trans>Klimper is locked</Trans>
              </h1>
              <p className='text-muted-foreground'>
                <Trans>Your keys open with your passkey.</Trans>
              </p>
            </div>
            <Button size='lg' onClick={unlock} disabled={busy}>
              <Trans>Unlock with passkey</Trans>
            </Button>
            {error && (
              <p className='max-w-sm text-sm text-destructive'>{error}</p>
            )}
          </>
        )}
      </div>
    )
  );
}
