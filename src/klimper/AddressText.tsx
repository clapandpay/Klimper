// SPDX-License-Identifier: AGPL-3.0-or-later
// An address as people check it: the network prefix quiet, the start and the end in full
// colour (the parts anyone compares), the middle folded away. A tap shows the whole address,
// wrapped, since a phone has no hover; the copy button sits beside it with a finger-sized
// target. One look for every address Klimper shows: transactions and website requests.

import { t } from '@lingui/core/macro';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { Check, Copy } from 'lucide-react';
import { ReactNode, useState } from 'react';

/** Characters kept at each end of the part after the prefix. */
const KEEP = 6;

/** `xch1…` into its prefix (with the 1) and the rest; a hex id has no prefix. */
export function splitAddress(address: string): { hrp: string; body: string } {
  const at = address.lastIndexOf('1');
  if (/^t?xch1/i.test(address) && at > 0 && at < 8) {
    return { hrp: address.slice(0, at + 1), body: address.slice(at + 1) };
  }
  if (/^0x/i.test(address)) {
    return { hrp: '0x', body: address.slice(2) };
  }
  return { hrp: '', body: address };
}

export function AddressText({
  address,
  badge,
  copy = true,
  testId,
}: {
  readonly address: string;
  /** A word next to the address, e.g. "Yours". */
  readonly badge?: ReactNode;
  readonly copy?: boolean;
  readonly testId?: string;
}) {
  const [whole, setWhole] = useState(false);
  const [copied, setCopied] = useState(false);
  const { hrp, body } = splitAddress(address);
  const folds = body.length > KEEP * 2 + 3;

  const onCopy = () => {
    void writeText(address)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1600);
      })
      .catch(() => undefined);
  };

  return (
    <span className='inline-flex max-w-full min-w-0 items-center gap-1.5 align-middle'>
      {badge && (
        <span className='shrink-0 rounded-full bg-muted px-2 py-0.5 text-[0.6875rem] font-semibold text-muted-foreground'>
          {badge}
        </span>
      )}
      <button
        type='button'
        className='min-w-0 rounded-md px-1 py-0.5 text-left font-mono text-xs leading-5 tracking-wide hover:bg-muted focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary'
        onClick={(event) => {
          event.stopPropagation();
          if (folds) setWhole((open) => !open);
        }}
        aria-expanded={folds ? whole : undefined}
        aria-label={address}
        data-testid={testId}
      >
        <span className='text-muted-foreground'>{hrp}</span>
        {!folds || whole ? (
          <span className='break-all text-foreground'>{body}</span>
        ) : (
          <>
            <span className='font-semibold text-foreground'>
              {body.slice(0, KEEP)}
            </span>
            <span className='text-muted-foreground'>…</span>
            <span className='font-semibold text-foreground'>
              {body.slice(-KEEP)}
            </span>
          </>
        )}
      </button>
      {copy && (
        <button
          type='button'
          className='-m-1.5 inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-muted-foreground hover:bg-muted hover:text-foreground'
          onClick={(event) => {
            event.stopPropagation();
            onCopy();
          }}
          aria-label={copied ? t`Copied` : t`Copy address`}
        >
          {copied ? (
            <Check className='h-3.5 w-3.5' aria-hidden='true' />
          ) : (
            <Copy className='h-3.5 w-3.5' aria-hidden='true' />
          )}
        </button>
      )}
    </span>
  );
}
