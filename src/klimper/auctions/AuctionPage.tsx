// SPDX-License-Identifier: AGPL-3.0-or-later
// One NFT auction, read and checked from the chain by its id: what is sold and on what terms,
// where it stands, and bidding in XCH, ending it, or (the seller's, before any bid) cancelling
// it. Reached from a link, the Scan tab (`auction:0x…`), the auctions list and the start form.

import Container from '@/components/Container';
import { CopyBox } from '@/components/CopyBox';
import Header from '@/components/Header';
import { Loading } from '@/components/Loading';
import StyledQRCode from '@/components/StyledQrCode';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Label } from '@/components/ui/label';
import { FeeAmountInput, TokenAmountInput } from '@/components/ui/masked-input';
import { useWallet } from '@/contexts/WalletContext';
import { useBiometric } from '@/hooks/useBiometric';
import { useErrors } from '@/hooks/useErrors';
import { formatTimestamp } from '@/lib/utils';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { useCallback, useEffect, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { toast } from 'react-toastify';
import {
  AuctionStatus,
  AuctionView,
  auctionErrorText,
  auctions,
  auctionShareText,
  parseAuctionId,
} from './api';
import { bidCost, bpsOf, sellerGets } from './math';
import { useXch } from './useXch';

export function statusText(status: AuctionStatus): string {
  switch (status) {
    case 'live':
      return t`Taking bids`;
    case 'ending':
      return t`Bidding closed: waiting to be ended`;
    case 'sold':
      return t`Sold`;
    case 'unsold':
      return t`Ended without a bid`;
    case 'cancelled':
      return t`Cancelled by the seller`;
    default:
      return t`Unknown: no connection to the network`;
  }
}

function percent(bps: number): string {
  return `${(bps / 100).toLocaleString(undefined, { maximumFractionDigits: 2 })}%`;
}

function when(seconds: number): string {
  return formatTimestamp(seconds, 'medium', 'short');
}

type Pending = 'bid' | 'end' | 'cancel' | null;

export default function AuctionPage() {
  const { id: param } = useParams();
  const navigate = useNavigate();
  const { addError } = useErrors();
  const { isReadOnly } = useWallet();
  const { promptIfEnabled } = useBiometric();
  const xch = useXch();
  const id = parseAuctionId(param ?? '');

  const [view, setView] = useState<AuctionView | null>(null);
  const [loading, setLoading] = useState(true);
  const [amount, setAmount] = useState('');
  const [fee, setFee] = useState('');
  const [pending, setPending] = useState<Pending>(null);
  const [sending, setSending] = useState(false);

  const load = useCallback(async () => {
    if (id === null) return;
    setLoading(true);
    try {
      setView(await auctions.view(id));
    } catch (error) {
      addError({ kind: 'invalid', reason: auctionErrorText(error) });
      navigate('/auctions');
    } finally {
      setLoading(false);
    }
  }, [id, addError, navigate]);

  useEffect(() => {
    load();
  }, [load]);

  // The least bid, filled in once, for the user to raise.
  const nextMinimum = view?.next_minimum_bid ?? null;
  const { decimal } = xch;
  useEffect(() => {
    if (nextMinimum !== null) {
      setAmount((current) => (current === '' ? decimal(nextMinimum) : current));
    }
  }, [nextMinimum, decimal]);

  if (id === null) {
    return (
      <>
        <Header title={t`Auction`} back={() => navigate('/auctions')} />
        <Container>
          <p className='text-sm text-muted-foreground'>
            <Trans>That is not an auction id.</Trans>
          </p>
        </Container>
      </>
    );
  }

  const bid = xch.mojos(amount);
  const networkFee = xch.mojos(fee || '0') ?? 0n;
  const cost =
    view && bid !== null
      ? bidCost(bid, view.buyers_premium_bps, view.royalty_bps)
      : null;
  const bidValid =
    view !== null && bid !== null && bid >= BigInt(view.next_minimum_bid);

  // Every figure a message shows, as a plain value (the messages are translated whole).
  const winningText = view?.winning_bid
    ? xch.text(view.winning_bid.amount)
    : '';
  const endsText = view ? when(view.end_time) : '';
  const nextMinimumText = view ? xch.text(view.next_minimum_bid) : '';
  const cannotBidReason = view?.cannot_bid_reason ?? '';
  const costText = cost !== null ? xch.text(cost) : '';
  const royaltyPercent = view ? percent(view.royalty_bps) : '';
  const premiumPercent = view ? percent(view.buyers_premium_bps) : '';
  const commissionPercent = view ? percent(view.commission_bps) : '';
  const minimumText = view ? xch.text(view.minimum_bid) : '';
  const stepText =
    view?.increment.kind === 'flat' ? xch.text(view.increment.amount) : '';
  const stepPercent =
    view?.increment.kind === 'percent' ? percent(view.increment.bps) : '';
  const graceMinutes = view ? Math.round(view.grace_period / 60) : 0;
  const premiumAddress = view?.buyers_premium_address ?? '';
  const commissionAddress = view?.commission_address ?? '';
  const royaltyAddress = view?.royalty_address ?? '';
  const feeText = xch.text(networkFee);
  const sellerText = view?.winning_bid
    ? xch.text(sellerGets(BigInt(view.winning_bid.amount), view.commission_bps))
    : '';
  const royaltyAmount =
    view && bid !== null ? bpsOf(bid, view.royalty_bps) : 0n;
  const royaltyAmountText = xch.text(royaltyAmount);

  const send = async () => {
    if (view === null || pending === null || isReadOnly) return;
    if (!(await promptIfEnabled())) return;
    setSending(true);
    try {
      const feeMojos = networkFee.toString();
      if (pending === 'bid' && bid !== null) {
        await auctions.bid(view.id, bid.toString(), feeMojos);
        toast.success(t`Bid sent. It counts once the network confirms it.`);
      } else if (pending === 'end') {
        await auctions.end(view.id, feeMojos);
        toast.success(t`Sent: the auction ends once the network confirms it.`);
      } else if (pending === 'cancel') {
        await auctions.cancel(view.id, feeMojos);
        toast.success(
          t`Cancel sent: the NFT comes back once the network confirms it.`,
        );
      }
      setPending(null);
      await load();
    } catch (error) {
      addError({ kind: 'invalid', reason: auctionErrorText(error) });
      await load();
    } finally {
      setSending(false);
      setPending(null);
    }
  };

  return (
    <>
      <Header title={t`Auction`} back={() => navigate('/auctions')} />
      <Container className='max-w-2xl'>
        {loading && view === null ? (
          <Loading />
        ) : view === null ? null : (
          <div className='flex flex-col gap-4'>
            <Card>
              <CardHeader className='pb-2'>
                <CardTitle className='text-md font-medium'>
                  {statusText(view.status)}
                </CardTitle>
              </CardHeader>
              <CardContent className='space-y-3 text-sm'>
                <CopyBox title={t`NFT for sale`} value={view.nft_address} />
                <p className='text-muted-foreground'>
                  <Trans>
                    Check the NFT by its id: a name or picture can be copied.
                  </Trans>
                </p>
                {view.winning_bid ? (
                  <p data-testid='auction-winning-bid'>
                    {view.winning_bid.is_mine ? (
                      <Trans>Highest bid: {winningText} (yours)</Trans>
                    ) : (
                      <Trans>Highest bid: {winningText}</Trans>
                    )}
                  </p>
                ) : (
                  <p>
                    <Trans>No bid yet.</Trans>
                  </p>
                )}
                {view.status === 'live' && (
                  <p>
                    <Trans>
                      Ends {endsText}. The next bid is at least{' '}
                      {nextMinimumText}.
                    </Trans>
                  </p>
                )}
                {view.cannot_bid_reason && view.status === 'live' && (
                  <p className='text-destructive'>
                    <Trans>You cannot bid: {cannotBidReason}.</Trans>
                  </p>
                )}
              </CardContent>
            </Card>

            {view.can_bid && !view.is_mine && (
              <Card>
                <CardHeader className='pb-2'>
                  <CardTitle className='text-md font-medium'>
                    <Trans>Bid</Trans>
                  </CardTitle>
                </CardHeader>
                <CardContent className='space-y-3 text-sm'>
                  <div className='space-y-1'>
                    <Label htmlFor='auction-bid'>
                      <Trans>Your bid</Trans>
                    </Label>
                    <TokenAmountInput
                      id='auction-bid'
                      value={amount}
                      onValueChange={(values) => setAmount(values.value)}
                      ticker={xch.unit.ticker}
                      precision={xch.unit.precision}
                    />
                  </div>
                  <div className='space-y-1'>
                    <Label htmlFor='auction-bid-fee'>
                      <Trans>Network fee</Trans>
                    </Label>
                    <FeeAmountInput
                      id='auction-bid-fee'
                      value={fee}
                      onValueChange={(values) => setFee(values.value)}
                    />
                  </div>
                  {cost !== null && bid !== null && (
                    <p data-testid='auction-bid-cost'>
                      <Trans>
                        Locks up {costText}: the bid, the NFT’s royalty (
                        {royaltyPercent}) and the buyer’s premium (
                        {premiumPercent}). If someone bids higher, all of it
                        comes back.
                      </Trans>
                    </p>
                  )}
                  {view.in_grace && (
                    <p className='text-muted-foreground'>
                      <Trans>
                        The end time has passed: a bid now must come within the
                        grace period of the last one.
                      </Trans>
                    </p>
                  )}
                  <Button
                    disabled={!bidValid || isReadOnly || sending}
                    onClick={() => setPending('bid')}
                  >
                    <Trans>Bid</Trans>
                  </Button>
                </CardContent>
              </Card>
            )}

            {(view.can_end || view.can_cancel) && (
              <Card>
                <CardContent className='space-y-3 pt-6 text-sm'>
                  <div className='space-y-1'>
                    <Label htmlFor='auction-fee'>
                      <Trans>Network fee</Trans>
                    </Label>
                    <FeeAmountInput
                      id='auction-fee'
                      value={fee}
                      onValueChange={(values) => setFee(values.value)}
                    />
                  </div>
                  {view.can_end && (
                    <>
                      <p>
                        <Trans>
                          Bidding has closed. Anyone can end the auction: the
                          NFT goes to the highest bidder (or back to the
                          seller), and the bid to the seller.
                        </Trans>
                      </p>
                      <Button
                        disabled={isReadOnly || sending}
                        onClick={() => setPending('end')}
                      >
                        <Trans>End the auction</Trans>
                      </Button>
                    </>
                  )}
                  {view.can_cancel && (
                    <Button
                      variant='outline'
                      disabled={isReadOnly || sending}
                      onClick={() => setPending('cancel')}
                    >
                      <Trans>Cancel and take the NFT back</Trans>
                    </Button>
                  )}
                </CardContent>
              </Card>
            )}

            <Card>
              <CardHeader className='pb-2'>
                <CardTitle className='text-md font-medium'>
                  <Trans>Terms</Trans>
                </CardTitle>
              </CardHeader>
              <CardContent className='space-y-2 text-sm'>
                <p>
                  <Trans>Minimum bid: {minimumText}</Trans>
                </p>
                <p>
                  {view.increment.kind === 'flat' ? (
                    <Trans>Each bid at least {stepText} more</Trans>
                  ) : (
                    <Trans>Each bid at least {stepPercent} more</Trans>
                  )}
                </p>
                <p>
                  <Trans>Ends: {endsText}</Trans>
                </p>
                {view.grace_period > 0 && (
                  <p>
                    <Trans>
                      Grace period: a bid within {graceMinutes} minutes of the
                      last one still counts after the end.
                    </Trans>
                  </p>
                )}
                <p>
                  <Trans>
                    NFT royalty: {royaltyPercent}, paid by the winner
                  </Trans>
                </p>
                {view.buyers_premium_bps > 0 && (
                  <p>
                    <Trans>
                      Buyer’s premium: {premiumPercent}, paid by the winner to{' '}
                      {premiumAddress}
                    </Trans>
                  </p>
                )}
                {view.commission_bps > 0 && (
                  <p>
                    <Trans>
                      Commission: {commissionPercent} of the winning bid, to{' '}
                      {commissionAddress}
                    </Trans>
                  </p>
                )}
                <CopyBox title={t`Seller`} value={view.seller_address} />
              </CardContent>
            </Card>

            <Card>
              <CardHeader className='pb-2'>
                <CardTitle className='text-md font-medium'>
                  <Trans>Share</Trans>
                </CardTitle>
              </CardHeader>
              <CardContent className='flex flex-col items-center gap-3'>
                <div className='mx-auto aspect-square w-full max-w-[280px] rounded-md bg-white p-2'>
                  <StyledQRCode
                    data={auctionShareText(view.id)}
                    width={1024}
                    height={1024}
                    margin={16}
                    dotsOptions={{ type: 'square', color: '#000000' }}
                    backgroundOptions={{ color: '#ffffff' }}
                  />
                </div>
                <CopyBox
                  title={t`Auction id`}
                  value={auctionShareText(view.id)}
                  truncateMiddle
                />
              </CardContent>
            </Card>
          </div>
        )}
      </Container>

      <Dialog
        open={pending !== null}
        onOpenChange={(open) => !open && setPending(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {pending === 'bid' ? (
                <Trans>Place this bid?</Trans>
              ) : pending === 'end' ? (
                <Trans>End the auction?</Trans>
              ) : (
                <Trans>Cancel the auction?</Trans>
              )}
            </DialogTitle>
            <DialogDescription>
              {pending === 'bid' && cost !== null && view ? (
                <Trans>
                  {costText} leaves your wallet now, plus a network fee of{' '}
                  {feeText}. If you win, you receive the NFT; if someone bids
                  higher, the {costText} comes back.
                </Trans>
              ) : pending === 'end' && view ? (
                view.winning_bid ? (
                  <Trans>
                    The NFT goes to the highest bidder and {sellerText} to the
                    seller. Network fee: {feeText}.
                  </Trans>
                ) : (
                  <Trans>
                    Nobody bid: the NFT goes back to the seller. Network fee:{' '}
                    {feeText}.
                  </Trans>
                )
              ) : (
                <Trans>
                  The NFT comes back to your wallet. Network fee: {feeText}.
                </Trans>
              )}
            </DialogDescription>
          </DialogHeader>
          {pending === 'bid' && view && bid !== null && (
            <p className='text-xs text-muted-foreground'>
              {bpsOf(bid, view.royalty_bps) > 0n && (
                <Trans>
                  The royalty, {royaltyAmountText}, goes to {royaltyAddress}.
                </Trans>
              )}
            </p>
          )}
          <DialogFooter>
            <Button variant='outline' onClick={() => setPending(null)}>
              <Trans>Back</Trans>
            </Button>
            <Button disabled={sending} onClick={send}>
              <Trans>Confirm</Trans>
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
