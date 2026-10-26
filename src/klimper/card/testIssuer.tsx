// SPDX-License-Identifier: AGPL-3.0-or-later
// The issuer the card tests render under (TASK-164): Gnosis Pay as `card_status` describes it.

import { ReactNode } from 'react';
import type { CardIssuer } from './card';
import { CardIssuerProvider } from './issuer';

export const GNOSIS_PAY: CardIssuer = {
  id: 'gnosis-pay',
  name: 'Gnosis Pay',
  scheme: 'Visa',
  network: 'Gnosis Chain',
  topUpAsset: 'EURe',
  environment: 'sandbox',
  capabilities: { cardDetails: true, freeze: true },
};

export function withIssuer(screen: ReactNode, issuer: CardIssuer = GNOSIS_PAY) {
  return <CardIssuerProvider issuer={issuer}>{screen}</CardIssuerProvider>;
}
