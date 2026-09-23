// SPDX-License-Identifier: AGPL-3.0-or-later
import { Button } from '@/components/ui/button';
import { Label } from '@/components/ui/label';
import { Trans } from '@lingui/react/macro';
import { useCallback, useEffect, useState } from 'react';
import { notifyPasskeyChanged, passkey, PasskeyStatus } from './passkey';

export function PasskeySettings() {
  const [status, setStatus] = useState<PasskeyStatus | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(
    () => passkey.status().then(setStatus, (err) => setError(String(err))),
    [],
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      setConfirming(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
      await refresh();
      notifyPasskeyChanged();
    }
  };

  if (!status?.supported) return null;

  return (
    <div className='divide-y rounded-md border bg-card text-card-foreground overflow-hidden'>
      <div className='p-3'>
        <h3 className='text-sm font-medium'>
          <Trans>Passkey</Trans>
        </h3>
      </div>
      <div className='p-3 space-y-3'>
        <div className='space-y-1'>
          <Label className='text-sm font-medium'>
            {status.enrolled ? (
              <Trans>Your keys are protected by a passkey</Trans>
            ) : (
              <Trans>Protect your keys with a passkey</Trans>
            )}
          </Label>
          <div className='text-sm text-muted-foreground'>
            <Trans>
              The recovery words and keys are encrypted with a key only your
              passkey can open, with Face ID each time Klimper opens.
            </Trans>
          </div>
        </div>

        {!status.enrolled && confirming && (
          <div className='rounded-md border border-destructive/50 p-3 text-sm'>
            <Trans>
              Write down your recovery words first. The passkey lives in your
              iCloud Keychain; if it is deleted or the account is lost, only the
              recovery words restore this wallet.
            </Trans>
          </div>
        )}

        <div className='flex gap-2'>
          {status.enrolled ? (
            <Button
              variant='outline'
              disabled={busy}
              onClick={() => run(passkey.remove)}
            >
              <Trans>Remove passkey</Trans>
            </Button>
          ) : confirming ? (
            <>
              <Button disabled={busy} onClick={() => run(passkey.enroll)}>
                <Trans>I have my recovery words, create the passkey</Trans>
              </Button>
              <Button variant='ghost' onClick={() => setConfirming(false)}>
                <Trans>Cancel</Trans>
              </Button>
            </>
          ) : (
            <Button onClick={() => setConfirming(true)}>
              <Trans>Set up passkey</Trans>
            </Button>
          )}
        </div>
        {error && <p className='text-sm text-destructive'>{error}</p>}
      </div>
    </div>
  );
}
