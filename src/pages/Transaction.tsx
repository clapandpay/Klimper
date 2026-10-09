import { commands, events, TransactionRecord } from '@/bindings';
import { AssetCoin } from '@/components/AssetCoin';
import Container from '@/components/Container';
import Header from '@/components/Header';
import { LabeledItem } from '@/components/LabeledItem';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { useNetwork } from '@/hooks/useNetwork';
import { CoinAddress } from '@/klimper/CoinAddress';
import { mempoolBlockUrl } from '@/klimper/explorer';
import { formatTimestamp } from '@/lib/utils';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import {
  ArrowDown,
  ArrowLeftRight,
  ArrowUp,
  Calendar,
  CheckCircle,
  ExternalLink,
} from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useParams } from 'react-router-dom';

export default function Transaction() {
  const { height } = useParams();

  const [transaction, setTransaction] = useState<TransactionRecord | null>(
    null,
  );

  const updateTransaction = useCallback(() => {
    commands
      .getTransaction({
        height: Number(height),
      })
      .then((data) => {
        if (data.transaction) {
          setTransaction(data.transaction);
        } else {
          setTransaction(null);
        }
      });
  }, [height]);

  useEffect(() => {
    updateTransaction();

    const unlisten = events.syncEvent.listen((data) => {
      switch (data.payload.type) {
        case 'coin_state':
        case 'cat_info':
        case 'did_info':
        case 'nft_data':
        case 'puzzle_batch_synced':
          updateTransaction();
      }
    });

    return () => {
      unlisten.then((u) => u());
    };
  }, [updateTransaction]);

  const heightLabel = height ?? '';
  // Look the spends up on mempoolxch.space (Klimper).
  const { network, isTestnet } = useNetwork();
  const explorerUrl =
    network === 'mainnet' || network === 'testnet'
      ? mempoolBlockUrl(height, isTestnet)
      : null;

  return (
    <>
      <Header title={t`Transaction #${heightLabel}`} />
      <Container>
        <Card className='mb-6'>
          <CardHeader className='pb-2'>
            <CardTitle className='flex items-center gap-2'>
              <ArrowLeftRight className='h-5 w-5' aria-hidden='true' />
              <Trans>Transaction Details</Trans>
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className='flex flex-wrap items-center gap-x-4 gap-y-2 mb-4'>
              <div className='px-3 py-1 rounded-full text-sm font-medium bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-300'>
                <div className='flex items-center gap-1'>
                  <CheckCircle className='h-4 w-4' aria-hidden='true' />
                  <Trans>Confirmed</Trans>
                </div>
              </div>
              {explorerUrl && (
                <Button
                  variant='outline'
                  size='sm'
                  onClick={() =>
                    void openUrl(explorerUrl).catch(() => undefined)
                  }
                >
                  {t`View on mempoolxch.space`}
                  <ExternalLink className='h-4 w-4' aria-hidden='true' />
                </Button>
              )}
            </div>

            <div className='grid grid-cols-1 md:grid-cols-3 gap-4'>
              {transaction?.timestamp && (
                <div className='flex items-center gap-2'>
                  <Calendar className='h-4 w-4 text-muted-foreground' />
                  <LabeledItem
                    label={t`Created`}
                    content={formatTimestamp(
                      transaction?.timestamp,
                      'short',
                      'short',
                    )}
                  />
                </div>
              )}

              <div className='flex items-center gap-2'>
                <LabeledItem
                  label={t`Block Height`}
                  content={height?.toString() ?? null}
                />
              </div>
            </div>
          </CardContent>
        </Card>
        <div className='grid grid-cols-1 lg:grid-cols-2 gap-6'>
          {(transaction?.spent.length ?? 0) > 0 && (
            <Card>
              <CardHeader>
                <CardTitle className='flex items-center gap-2'>
                  <ArrowUp className='h-5 w-5' aria-hidden='true' />
                  <Trans>Sent</Trans>
                </CardTitle>
              </CardHeader>
              <CardContent className='space-y-4'>
                {transaction?.spent.map((coin) => (
                  <AssetCoin
                    key={coin.coin_id}
                    asset={coin.asset}
                    amount={coin.amount}
                    coinId={coin.coin_id}
                  >
                    <CoinAddress coin={coin} />
                  </AssetCoin>
                ))}
              </CardContent>
            </Card>
          )}
          {(transaction?.created.length ?? 0) > 0 && (
            <Card>
              <CardHeader>
                <CardTitle className='flex items-center gap-2'>
                  <ArrowDown className='h-5 w-5' aria-hidden='true' />
                  <Trans>Received</Trans>
                </CardTitle>
              </CardHeader>
              <CardContent className='space-y-4'>
                {transaction?.created.map((coin) => (
                  <AssetCoin
                    key={coin.coin_id}
                    asset={coin.asset}
                    amount={coin.amount}
                    coinId={coin.coin_id}
                  >
                    <CoinAddress coin={coin} />
                  </AssetCoin>
                ))}
              </CardContent>
            </Card>
          )}
        </div>
      </Container>
    </>
  );
}
