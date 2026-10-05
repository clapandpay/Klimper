// SPDX-License-Identifier: AGPL-3.0-or-later
// What a scanned or pasted code is, decided in one place for the Scan tab.
//
// The Scan tab takes anything: a website's Xchonnect code, an offer, an address. This is
// the pure half — it says what the text is and never acts on it — so the rules can be
// tested without a camera, a router or the plugin.
//
// An Xchonnect link is recognised before anything else and carries no text of its own:
// a pairing code is a secret (spec 6.2), so the screen hands the raw string straight to
// `offerXchonnectLink` and never renders it. Text this module does not recognise is
// shown back to the user, which is why anything that even starts like an Xchonnect code
// is claimed here rather than falling through to `unknown`.

import { isValidAddress } from '@/lib/utils';
import { parseXchonnectLink } from './xchonnectLink';

export type ScanTarget =
  | { kind: 'xchonnect' }
  | { kind: 'offer'; offer: string }
  | { kind: 'address'; address: string }
  | { kind: 'unknown'; text: string };

export interface ScanFeatures {
  xchonnect: boolean;
  offers: boolean;
}

const XCHONNECT_SCHEME = 'xchonnect:';
const ADDRESS_PREFIXES = ['xch', 'txch'] as const;

export function classifyScan(raw: string, features: ScanFeatures): ScanTarget {
  const text = raw.trim();

  const link = parseXchonnectLink(text);
  if (link.kind !== 'notALink') {
    return features.xchonnect ? { kind: 'xchonnect' } : unknownSecret();
  }
  // A malformed `xchonnect:` code is still somebody's secret: never echo it.
  if (text.toLowerCase().startsWith(XCHONNECT_SCHEME)) return unknownSecret();

  // Wallets put an address behind a `chia:` scheme now and then.
  const bare = text.replace(/^chia:/i, '');
  for (const prefix of ADDRESS_PREFIXES) {
    if (isValidAddress(bare, prefix)) return { kind: 'address', address: bare };
  }

  if (features.offers && /^offer1[02-9ac-hj-np-z]+$/.test(text)) {
    return { kind: 'offer', offer: text };
  }

  return { kind: 'unknown', text };
}

/** Recognised as an Xchonnect code but not usable here; its text is not shown. */
function unknownSecret(): ScanTarget {
  return { kind: 'unknown', text: '' };
}
