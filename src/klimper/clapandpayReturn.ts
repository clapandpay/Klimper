// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 the Klimper authors
//
// Where the answer of "Use my Clap and Pay money" arrives (clapandpay TASK-129). The page answers
// with a clapandpay.com/klimper/return link that the phone hands to this app (the applinks in the
// entitlements); the deep-link listener passes it here, and the screen that made the request
// is waiting for it. Only that screen holds the one-time key, in memory: an answer that comes
// with no screen waiting cannot be opened by anyone, and is dropped with a word to the person.

import { t } from '@lingui/core/macro';
import { toast } from 'react-toastify';

type Waiter = (link: string) => void;

let waiting: Waiter | null = null;

/** The screen waits for the return link; the returned function stops waiting. */
export function awaitClapAndPayReturn(waiter: Waiter): () => void {
  waiting = waiter;
  return () => {
    if (waiting === waiter) waiting = null;
  };
}

/** A return link the system handed to the app. */
export function offerClapAndPayReturn(link: string): void {
  if (waiting) {
    waiting(link);
    return;
  }
  toast.info(
    t`That answer was for a request Klimper no longer has. Open "Use my Clap and Pay money" and start again.`,
  );
}
