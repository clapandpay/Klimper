// SPDX-License-Identifier: AGPL-3.0-or-later
// The card issuer this build was made for (TASK-164), as the screens need it. Rust decides
// (`card_status`, KLIMPER_CARD_ISSUER); no screen names an issuer itself. Everything below
// the card screen reads it here: its name, scheme, network and top-up asset for the text,
// and what it can do, so a screen hides what the issuer does not offer.

import { createContext, ReactNode, useContext } from 'react';
import type { CardIssuer } from './card';

const IssuerContext = createContext<CardIssuer | null>(null);

export function CardIssuerProvider({
  issuer,
  children,
}: {
  readonly issuer: CardIssuer;
  readonly children: ReactNode;
}) {
  return (
    <IssuerContext.Provider value={issuer}>{children}</IssuerContext.Provider>
  );
}

/** The build's issuer. Only screens under `CardIssuerProvider` (the card screen) use it. */
export function useCardIssuer(): CardIssuer {
  const issuer = useContext(IssuerContext);
  if (!issuer) throw new Error('useCardIssuer outside CardIssuerProvider');
  return issuer;
}
