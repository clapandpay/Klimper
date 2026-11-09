// SPDX-License-Identifier: AGPL-3.0-or-later
// Start an auction of one of the wallet's NFTs, with bids in XCH: the least bid, how much each
// bid must beat the last, how long it runs. Klimper takes no commission and no buyer's premium;
// the NFT's own royalty is paid by the winner. Reached from the NFT's page.

import Container from '@/components/Container';
import Header from '@/components/Header';
import { Button } from '@/components/ui/button';
import { Card, CardContent } from '@/components/ui/card';
import { Label } from '@/components/ui/label';
import { FeeAmountInput, TokenAmountInput } from '@/components/ui/masked-input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { useWallet } from '@/contexts/WalletContext';
import { useBiometric } from '@/hooks/useBiometric';
import { useErrors } from '@/hooks/useErrors';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { toast } from 'react-toastify';
import { auctionErrorText, auctions } from './api';
import { useXch } from './useXch';

const HOUR = 60 * 60;
const DURATIONS: { seconds: number; label: () => string }[] = [
  { seconds: HOUR, label: () => t`1 hour` },
  { seconds: 6 * HOUR, label: () => t`6 hours` },
  { seconds: 24 * HOUR, label: () => t`1 day` },
  { seconds: 3 * 24 * HOUR, label: () => t`3 days` },
  { seconds: 7 * 24 * HOUR, label: () => t`7 days` },
];
const GRACES: { seconds: number; label: () => string }[] = [
  { seconds: 0, label: () => t`None: bidding stops at the end` },
  { seconds: 5 * 60, label: () => t`5 minutes after the last bid` },
  { seconds: 15 * 60, label: () => t`15 minutes after the last bid` },
];

export default function StartAuction() {
  const { nftId } = useParams();
  const navigate = useNavigate();
  const { addError } = useErrors();
  const { isReadOnly } = useWallet();
  const { promptIfEnabled } = useBiometric();
  const xch = useXch();
  const ticker = xch.unit.ticker;

  const [minimum, setMinimum] = useState('');
  const [step, setStep] = useState('');
  const [duration, setDuration] = useState(String(24 * HOUR));
  const [grace, setGrace] = useState('0');
  const [fee, setFee] = useState('');
  const [sending, setSending] = useState(false);

  const minimumMojos = xch.mojos(minimum);
  const stepMojos = xch.mojos(step);
  const valid =
    nftId !== undefined &&
    minimumMojos !== null &&
    minimumMojos > 0n &&
    stepMojos !== null &&
    stepMojos > 0n;

  const start = async () => {
    if (!valid || isReadOnly || nftId === undefined) return;
    if (!(await promptIfEnabled())) return;
    setSending(true);
    try {
      const sent = await auctions.start({
        nft_id: nftId,
        minimum_bid: (minimumMojos as bigint).toString(),
        increment: { kind: 'flat', amount: (stepMojos as bigint).toString() },
        end_time: Math.floor(Date.now() / 1000) + Number(duration),
        grace_period: Number(grace),
        fee: (xch.mojos(fee || '0') ?? 0n).toString(),
      });
      toast.success(
        t`Auction sent. It takes bids once the network confirms it.`,
      );
      navigate(`/auctions/view/${sent.id}`);
    } catch (error) {
      addError({ kind: 'invalid', reason: auctionErrorText(error) });
    } finally {
      setSending(false);
    }
  };

  return (
    <>
      <Header title={t`Start an auction`} />
      <Container className='max-w-xl'>
        <Card>
          <CardContent className='space-y-4 pt-6 text-sm'>
            <p className='text-muted-foreground'>
              <Trans>
                The NFT is locked in the auction until it ends. Bids are in{' '}
                {ticker}. The highest bid when it ends buys the NFT; with no
                bid, the NFT comes back to you. Until the first bid you can
                cancel it.
              </Trans>
            </p>
            <div className='space-y-1'>
              <Label htmlFor='auction-minimum'>
                <Trans>Least bid</Trans>
              </Label>
              <TokenAmountInput
                id='auction-minimum'
                value={minimum}
                onValueChange={(values) => setMinimum(values.value)}
                ticker={xch.unit.ticker}
                precision={xch.unit.precision}
              />
            </div>
            <div className='space-y-1'>
              <Label htmlFor='auction-step'>
                <Trans>Each bid at least this much higher</Trans>
              </Label>
              <TokenAmountInput
                id='auction-step'
                value={step}
                onValueChange={(values) => setStep(values.value)}
                ticker={xch.unit.ticker}
                precision={xch.unit.precision}
              />
            </div>
            <div className='space-y-1'>
              <Label>
                <Trans>Runs for</Trans>
              </Label>
              <Select value={duration} onValueChange={setDuration}>
                <SelectTrigger aria-label={t`Runs for`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {DURATIONS.map((option) => (
                    <SelectItem
                      key={option.seconds}
                      value={String(option.seconds)}
                    >
                      {option.label()}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className='space-y-1'>
              <Label>
                <Trans>Late bids</Trans>
              </Label>
              <Select value={grace} onValueChange={setGrace}>
                <SelectTrigger aria-label={t`Late bids`}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {GRACES.map((option) => (
                    <SelectItem
                      key={option.seconds}
                      value={String(option.seconds)}
                    >
                      {option.label()}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className='space-y-1'>
              <Label htmlFor='auction-start-fee'>
                <Trans>Network fee</Trans>
              </Label>
              <FeeAmountInput
                id='auction-start-fee'
                value={fee}
                onValueChange={(values) => setFee(values.value)}
              />
            </div>
            <Button disabled={!valid || isReadOnly || sending} onClick={start}>
              <Trans>Start the auction</Trans>
            </Button>
          </CardContent>
        </Card>
      </Container>
    </>
  );
}
