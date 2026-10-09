import { Amount, Asset, AssetKind } from '@/bindings';
import { AssetIcon } from '@/components/AssetIcon';
import { CopyButton } from '@/components/CopyButton';
import { NumberFormat } from '@/components/NumberFormat';
import { useNetwork } from '@/hooks/useNetwork';
import { mempoolCoinUrl } from '@/klimper/explorer';
import { formatAddress, fromMojos, getAssetDisplayName } from '@/lib/utils';
import { t } from '@lingui/core/macro';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ReactNode } from 'react';
import { toast } from 'react-toastify';

interface AssetCoinProps {
  asset: Asset;
  amount: Amount;
  coinId: string | null;
  children?: ReactNode;
}

export function AssetCoin({ asset, amount, coinId, children }: AssetCoinProps) {
  const { isTestnet } = useNetwork();
  const url = mempoolCoinUrl(coinId, isTestnet);
  const coinIdLabel = coinId ?? '';

  return (
    <div className='rounded-xl border border-border bg-card text-card-foreground shadow p-4'>
      <div
        className='cursor-pointer'
        onClick={() => {
          if (url) openUrl(url);
        }}
        aria-label={t`View coin ${coinIdLabel} on mempoolxch.space`}
        role='button'
        tabIndex={0}
        onKeyDown={(e) => {
          if ((e.key === 'Enter' || e.key === ' ') && url) {
            openUrl(url);
          }
        }}
      >
        <AssetAmount asset={asset} amount={amount} />
        {asset.asset_id && <Id kind={asset.kind} id={asset.asset_id} />}
        {coinId && <Id kind='coin' id={coinId} />}
      </div>
      {children}
    </div>
  );
}

interface AssetAmountProps {
  asset: Asset;
  amount: Amount;
}

function AssetAmount({ asset, amount }: AssetAmountProps) {
  const name = getAssetDisplayName(asset.name, asset.ticker, asset.kind);

  return (
    <div className='flex items-center gap-2'>
      <AssetIcon asset={asset} size='md' />

      <div className='flex flex-col'>
        <div className='text-md text-foreground break-all'>
          {asset.kind === 'token' ? (
            <>
              <NumberFormat
                value={fromMojos(amount, asset.precision)}
                maximumFractionDigits={asset.precision}
              />{' '}
              <span className='break-normal'>{name}</span>
            </>
          ) : (
            <span className='break-normal'>{name}</span>
          )}
        </div>
      </div>
    </div>
  );
}

function Id({ kind, id }: { kind: AssetKind | 'coin'; id: string }) {
  let label = '';
  let toastMessage = '';

  switch (kind) {
    case 'token':
      label = t`Asset ID`;
      toastMessage = t`Asset ID copied to clipboard`;
      break;
    case 'nft':
    case 'did':
      label = t`Launcher ID`;
      toastMessage = t`Launcher ID copied to clipboard`;
      break;
    case 'option':
      label = t`Option ID`;
      toastMessage = t`Option ID copied to clipboard`;
      break;
    case 'coin':
      label = t`Coin ID`;
      toastMessage = t`Coin ID copied to clipboard`;
      break;
    default:
      return null;
  }

  const handleCopyClick = (e: React.MouseEvent) => {
    e.stopPropagation();
  };

  const handleCopy = () => {
    toast.success(toastMessage);
  };

  return (
    <div className='flex items-center gap-2 mt-2 text-sm text-muted-foreground'>
      <span>{label}:</span>
      <span className='font-mono'>{formatAddress(id, 6, 6)}</span>
      <div onClick={handleCopyClick}>
        <CopyButton value={id} className='h-6 w-6 p-0' onCopy={handleCopy} />
      </div>
    </div>
  );
}
