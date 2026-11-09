// SPDX-License-Identifier: AGPL-3.0-or-later
// The auctions this wallet started or bid on, followed on the chain, and a field to open any
// auction by its id (`auction:0x…`, as its QR code and share text carry it).

import Container from '@/components/Container';
import Header from '@/components/Header';
import { Loading } from '@/components/Loading';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { useErrors } from '@/hooks/useErrors';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { auctionErrorText, auctions, MyAuction, parseAuctionId } from './api';
import { statusText } from './AuctionPage';
import { useXch } from './useXch';

export default function AuctionsPage() {
  const navigate = useNavigate();
  const { addError } = useErrors();
  const xch = useXch();
  const [mine, setMine] = useState<MyAuction[] | null>(null);
  const [typed, setTyped] = useState('');
  const id = parseAuctionId(typed);

  useEffect(() => {
    auctions
      .list()
      .then(setMine)
      .catch((error) => {
        addError({ kind: 'invalid', reason: auctionErrorText(error) });
        setMine([]);
      });
  }, [addError]);

  return (
    <>
      <Header title={t`Auctions`} />
      <Container className='max-w-2xl'>
        <div className='flex flex-col gap-4'>
          <Card>
            <CardContent className='flex gap-2 pt-6'>
              <Input
                aria-label={t`Auction id`}
                placeholder={t`Paste an auction id (auction:0x…)`}
                value={typed}
                onChange={(event) => setTyped(event.target.value)}
              />
              <Button
                disabled={id === null}
                onClick={() => id && navigate(`/auctions/view/${id}`)}
              >
                <Trans>Open</Trans>
              </Button>
            </CardContent>
          </Card>
          <Card>
            <CardHeader className='pb-2'>
              <CardTitle className='text-md font-medium'>
                <Trans>Your auctions and bids</Trans>
              </CardTitle>
            </CardHeader>
            <CardContent className='space-y-2 text-sm'>
              {mine === null ? (
                <Loading />
              ) : mine.length === 0 ? (
                <p className='text-muted-foreground'>
                  <Trans>
                    None yet. Start one from an NFT’s page, or open one from its
                    id or QR code.
                  </Trans>
                </p>
              ) : (
                mine.map((auction) => (
                  <button
                    type='button'
                    key={auction.id}
                    className='flex w-full flex-col items-start rounded-md border p-3 text-left hover:bg-accent'
                    onClick={() => navigate(`/auctions/view/${auction.id}`)}
                  >
                    <span className='font-medium'>
                      {auction.role === 'started' ? (
                        <Trans>You sell</Trans>
                      ) : (
                        <Trans>You bid</Trans>
                      )}
                      {auction.view
                        ? ` · ${statusText(auction.view.status)}`
                        : ''}
                    </span>
                    {auction.view?.winning_bid && (
                      <WinningBid
                        amount={xch.text(auction.view.winning_bid.amount)}
                        mine={auction.view.winning_bid.is_mine}
                      />
                    )}
                    {auction.error && (
                      <span className='text-destructive'>{auction.error}</span>
                    )}
                    <span className='font-mono text-xs text-muted-foreground'>
                      {auction.view?.nft_address ?? auction.id}
                    </span>
                  </button>
                ))
              )}
            </CardContent>
          </Card>
        </div>
      </Container>
    </>
  );
}

function WinningBid({ amount, mine }: { amount: string; mine: boolean }) {
  return (
    <span className='text-muted-foreground'>
      {mine ? (
        <Trans>Highest bid: {amount} (yours)</Trans>
      ) : (
        <Trans>Highest bid: {amount}</Trans>
      )}
    </span>
  );
}
