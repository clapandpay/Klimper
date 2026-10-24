// SPDX-License-Identifier: AGPL-3.0-or-later
// The signed-in part of the card screen.

import { Button } from '@/components/ui/button';
import {
  Card,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { Trans } from '@lingui/react/macro';
import { LogOut } from 'lucide-react';

export function CardHome({
  onSignOut,
}: {
  readonly address: string;
  readonly onSignOut: () => void;
  /** The session ended on its own (expired, revoked): back to the sign-in. */
  readonly onSignedOut: () => void;
}) {
  return (
    <Card data-testid='card-home'>
      <CardHeader>
        <CardTitle className='text-base'>
          <Trans>Signed in to Gnosis Pay</Trans>
        </CardTitle>
        <CardDescription>
          <Trans>The session lasts until Klimper closes.</Trans>
        </CardDescription>
        <Button variant='outline' size='sm' onClick={onSignOut}>
          <LogOut className='mr-2 h-4 w-4' aria-hidden='true' />
          <Trans>Sign out</Trans>
        </Button>
      </CardHeader>
    </Card>
  );
}
