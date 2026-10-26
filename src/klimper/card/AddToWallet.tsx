// SPDX-License-Identifier: AGPL-3.0-or-later
// Adding the virtual card to Apple Pay or Google Pay (TASK-157).
//
// Today this is the route every Visa card has: the phone's wallet app takes the card's
// details, and the card issuer verifies it. The one-tap "Add to Apple Wallet" button
// (in-app provisioning) needs an entitlement Apple grants only to the card issuer's app or
// an app it sponsors, plus a provisioning API from the issuer; Gnosis Pay offers none yet
// (docs/card.md).
// Apple's "Add to Apple Wallet" badge is reserved for that button, so it is not used here.
//
// Copying the number puts it on the clipboard, which other apps can read. Klimper clears
// the clipboard after a minute, even if this screen is gone by then.

import { Button } from '@/components/ui/button';
import { Trans } from '@lingui/react/macro';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { platform } from '@tauri-apps/plugin-os';
import { Check, Copy } from 'lucide-react';
import { useState } from 'react';
import type { CardDetails } from './card';

export const CLIPBOARD_CLEAR_MS = 60_000;

let clearTimer: ReturnType<typeof setTimeout> | undefined;

/** Copies `text` and clears the clipboard a minute later (module-level: outlives the screen). */
export async function copyForAMinute(text: string) {
  await writeText(text);
  if (clearTimer !== undefined) clearTimeout(clearTimer);
  clearTimer = setTimeout(() => {
    clearTimer = undefined;
    void writeText('').catch(() => undefined);
  }, CLIPBOARD_CLEAR_MS);
}

function currentPlatform(): string {
  try {
    return platform();
  } catch {
    return 'unknown';
  }
}

export function AddToWallet({ details }: { readonly details: CardDetails }) {
  const [copied, setCopied] = useState(false);
  const android = currentPlatform() === 'android';

  const copy = async () => {
    await copyForAMinute(details.number.replace(/\D/g, ''));
    setCopied(true);
  };

  return (
    <div
      className='space-y-2 rounded-lg bg-muted p-3 text-sm'
      data-testid='card-add-to-wallet'
    >
      <div className='font-medium'>
        {android ? (
          <Trans>Add to Google Pay</Trans>
        ) : (
          <Trans>Add to Apple Pay</Trans>
        )}
      </div>
      <ol className='list-decimal space-y-1 pl-5'>
        <li>
          <Trans>Copy the card number below, or keep this screen open.</Trans>
        </li>
        {android ? (
          <li>
            <Trans>
              Open Google Wallet, tap Add to Wallet, then Payment card, and
              enter the details by hand.
            </Trans>
          </li>
        ) : (
          <li>
            <Trans>
              Open the Wallet app, tap +, then Debit or Credit Card, and choose
              to enter the details by hand.
            </Trans>
          </li>
        )}
        <li>
          <Trans>
            Paste the number, then type the expiry, the security code and your
            name as on your ID.
          </Trans>
        </li>
        <li>
          <Trans>
            The card issuer checks the card; it may send you a code by text
            message. Then pay by holding the phone to the card reader.
          </Trans>
        </li>
      </ol>
      <Button
        size='sm'
        variant='outline'
        onClick={copy}
        data-testid='card-copy-number'
      >
        {copied ? (
          <Check className='mr-2 h-4 w-4' aria-hidden='true' />
        ) : (
          <Copy className='mr-2 h-4 w-4' aria-hidden='true' />
        )}
        {copied ? (
          <Trans>Copied; cleared again in a minute</Trans>
        ) : (
          <Trans>Copy the card number</Trans>
        )}
      </Button>
    </div>
  );
}
