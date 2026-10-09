// SPDX-License-Identifier: AGPL-3.0-or-later
// The wallet's handles, under its total: "@anna" is something to give out instead of an
// address, so each chip copies its name. Nothing shows without a handle, on testnet (the
// registry is mainnet only) or with handles turned off.

import { useNetwork } from '@/hooks/useNetwork';
import { handlesEnabled } from '@/lib/features';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { AtSign, Check, Copy } from 'lucide-react';
import { useEffect, useState } from 'react';
import { loadOwnedHandles, OwnedHandle } from './myHandles';

export function MyHandles() {
  const { network } = useNetwork();
  const on = handlesEnabled && network === 'mainnet';
  const [handles, setHandles] = useState<OwnedHandle[]>([]);

  useEffect(() => {
    if (!on) {
      setHandles([]);
      return;
    }
    let live = true;
    loadOwnedHandles()
      .then((list) => live && setHandles(list))
      .catch(() => live && setHandles([]));
    return () => {
      live = false;
    };
  }, [on]);

  if (handles.length === 0) return null;
  return (
    <section className='mb-4' aria-labelledby='my-handles-title'>
      <h2
        id='my-handles-title'
        className='mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground'
      >
        <Trans>Your handles</Trans>
      </h2>
      <ul className='flex flex-wrap gap-2' data-testid='my-handles'>
        {handles.map((h) => (
          <li key={h.nftId}>
            <HandleChip handle={h} />
          </li>
        ))}
      </ul>
    </section>
  );
}

function HandleChip({ handle }: { readonly handle: OwnedHandle }) {
  const [copied, setCopied] = useState(false);
  const name = `@${handle.handle}`;
  const expired = handle.status === 'expired';
  const until = handle.expiration
    ? new Date(handle.expiration * 1000).toLocaleDateString(undefined, {
        dateStyle: 'medium',
      })
    : null;
  const date = until ?? '';
  const detail = expired
    ? t`Expired ${date}`
    : until
      ? t`Until ${until}`
      : t`Not checked`;

  return (
    <button
      type='button'
      className={`inline-flex min-h-11 items-center gap-2 rounded-full border px-3.5 py-1.5 text-left transition-colors hover:bg-muted ${
        expired ? 'border-dashed opacity-70' : 'border-border bg-card'
      }`}
      onClick={() =>
        void writeText(name)
          .then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1600);
          })
          .catch(() => undefined)
      }
      aria-label={copied ? t`Copied ${name}` : t`Copy ${name}`}
      data-testid='my-handle'
    >
      <AtSign className='h-4 w-4 text-primary' aria-hidden='true' />
      <span className='flex flex-col leading-tight'>
        <span className='font-semibold'>{handle.handle}</span>
        <span className='text-[0.6875rem] text-muted-foreground'>{detail}</span>
      </span>
      {copied ? (
        <Check
          className='h-3.5 w-3.5 text-muted-foreground'
          aria-hidden='true'
        />
      ) : (
        <Copy
          className='h-3.5 w-3.5 text-muted-foreground'
          aria-hidden='true'
        />
      )}
    </button>
  );
}
