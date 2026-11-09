// SPDX-License-Identifier: AGPL-3.0-or-later
// "Start an auction" on an NFT's page (Sage's `Nft.tsx` carries this one element). Nothing
// without the auctions flag.

import { Button } from '@/components/ui/button';
import { auctionsEnabled } from '@/lib/features';
import { Trans } from '@lingui/react/macro';
import { Gavel } from 'lucide-react';
import { useNavigate } from 'react-router-dom';

export function StartAuctionButton({
  launcherId,
}: {
  launcherId: string | undefined;
}) {
  const navigate = useNavigate();
  if (!auctionsEnabled || !launcherId) return null;
  return (
    <Button
      variant='outline'
      className='w-full mt-1'
      onClick={() => navigate(`/auctions/start/${launcherId}`)}
    >
      <Gavel className='h-4 w-4 mr-2' aria-hidden='true' />
      <Trans>Start an auction</Trans>
    </Button>
  );
}
