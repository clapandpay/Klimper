// SPDX-License-Identifier: AGPL-3.0-or-later
// The one element in Sage's Make offer screen (TASK-164): make this offer a partial offer
// instead. It reads Sage's offer state, so nothing typed there is lost.

import { Button } from '@/components/ui/button';
import useOfferStateWithDefault from '@/hooks/useOfferStateWithDefault';
import { partialOffersEnabled } from '@/lib/features';
import { Trans } from '@lingui/react/macro';
import { Split } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { partialPair } from './MakePartialOffer';

export function PartialFillsOption() {
  const [state] = useOfferStateWithDefault();
  const navigate = useNavigate();
  if (!partialOffersEnabled) return null;
  const ready = partialPair(state) !== null;

  return (
    <div className='mt-4 flex flex-col gap-2 rounded-md border p-3 text-sm sm:flex-row sm:items-center sm:justify-between'>
      <div>
        <div className='font-medium'>
          <Trans>Allow partial fills</Trans>
        </div>
        <div className='text-muted-foreground'>
          {ready ? (
            <Trans>Takers can take any part of it, at the same price.</Trans>
          ) : (
            <Trans>For exactly one token offered for one token.</Trans>
          )}
        </div>
      </div>
      <Button
        variant='outline'
        disabled={!ready}
        onClick={() => navigate('/offers/make-partial')}
      >
        <Split className='mr-2 h-4 w-4' aria-hidden='true' />
        <Trans>Make partial offer</Trans>
      </Button>
    </div>
  );
}
