// SPDX-License-Identifier: AGPL-3.0-or-later
// Sending the user back to the dApp after a same-device request (clapandpay TASK-085,
// xchonnect spec 8.2, integration guide 5.4).
//
// This is the only place Klimper navigates anywhere on a dApp's behalf, and it is a place
// a leaked origin key (T18) would love to reach: a wallet that opens whatever URL a
// session carries is a one-tap redirect into a page the user has every reason to trust
// (T3).
//
// So the URL is checked against the session's own verified domain before anything opens,
// byte-exact on the whole authority. `xchonnect-core` already enforces the same rule where
// it binds the origin document to the claimed domain — a document whose `return_url` has a
// different authority is refused and pairing fails. This is the second lock, on the door
// that actually opens.
//
// **Not reachable yet.** The verified `return_url` exists in the plugin as
// `VerifiedUri::return_url` but is neither persisted with the session nor put on
// `SessionSummary`, so no caller has a URL to pass in. The guard and its tests are here so
// that the field, when it is added, has nothing left to decide.

import { openUrl } from '@tauri-apps/plugin-opener';
import { isReturnUrlAllowed } from './xchonnectLink';

export type ReturnOutcome = 'opened' | 'refused' | 'failed';

/**
 * Open the dApp's return URL, if it belongs to the session's verified domain.
 *
 * `refused` means the URL was not on that domain and nothing was opened; the caller shows
 * the "you can go back now" state instead, which is what the integration guide asks for
 * when there is no usable return URL.
 */
export async function returnToDapp(
  returnUrl: string | null | undefined,
  verifiedDomainAscii: string,
  open: (url: string) => Promise<void> = openUrl,
): Promise<ReturnOutcome> {
  if (
    returnUrl === null ||
    returnUrl === undefined ||
    !isReturnUrlAllowed(returnUrl, verifiedDomainAscii)
  ) {
    return 'refused';
  }
  try {
    await open(returnUrl);
    return 'opened';
  } catch {
    return 'failed';
  }
}
