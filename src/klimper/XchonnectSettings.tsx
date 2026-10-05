// SPDX-License-Identifier: AGPL-3.0-or-later
// The way into the Xchonnect screens (clapandpay TASK-072, TASK-084).
//
// Follows `PasskeySettings` exactly: a self-contained block in the settings screen that
// returns `null` when the feature is off, so `Settings.tsx` — shared with upstream Sage —
// carries one element and no condition.
//
// Settings is where this belongs rather than the navigation bar: connecting a website is
// something a user does a handful of times, and the request that matters comes to them.

import { Button } from '@/components/ui/button';
import { Label } from '@/components/ui/label';
import { xchonnectEnabled } from '@/lib/features';
import { Trans } from '@lingui/react/macro';
import { useNavigate } from 'react-router-dom';
import { XCHONNECT_ROUTES } from './xchonnectLinkHandoff';
import { useXchonnectSessions } from './useXchonnectSessions';

/** The flag is checked before any hook runs, so a build without it calls nothing. */
export function XchonnectSettings() {
  return xchonnectEnabled ? <XchonnectSettingsBlock /> : null;
}

function XchonnectSettingsBlock() {
  const navigate = useNavigate();
  const { sessions } = useXchonnectSessions();
  const count = sessions.length;

  return (
    <div className='divide-y rounded-md border bg-card text-card-foreground overflow-hidden'>
      <div className='p-3'>
        <h3 className='text-sm font-medium'>
          <Trans>Connected websites</Trans>
        </h3>
      </div>
      <div className='p-3 space-y-3'>
        <div className='space-y-1'>
          <Label className='text-sm font-medium'>
            {count === 0 ? (
              <Trans>No websites are connected</Trans>
            ) : (
              <Trans>{count} websites can ask you to sign</Trans>
            )}
          </Label>
          <div className='text-sm text-muted-foreground'>
            <Trans>
              A connected website can ask you to sign; you decide every time,
              and you can cut one off here.
            </Trans>
          </div>
        </div>
        <div className='flex flex-wrap gap-2'>
          <Button
            variant='outline'
            size='sm'
            onClick={() => navigate(XCHONNECT_ROUTES.dapps)}
          >
            <Trans>Manage</Trans>
          </Button>
          <Button
            variant='outline'
            size='sm'
            onClick={() => navigate(XCHONNECT_ROUTES.connect)}
          >
            <Trans>Connect a website</Trans>
          </Button>
        </div>
      </div>
    </div>
  );
}
