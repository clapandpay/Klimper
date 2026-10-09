// SPDX-License-Identifier: AGPL-3.0-or-later
// Privacy mode: amounts are hidden (•••) wherever the wallet shows a balance, and the coin
// sound and feel stay off. Kept in localStorage like Klimper's other look-and-feel choices.
//
// Confirmations reveal amounts again (`RevealAmounts`): what is about to be signed or sent is
// never hidden from the person deciding. The approval sheet for a website's request formats
// its own amounts and is never masked either.

import { t } from '@lingui/core/macro';
import { createContext, ReactNode, useContext } from 'react';
import { useLocalStorage } from 'usehooks-ts';

export const PRIVACY_KEY = 'klimper.privacy';
export const MASK = '•••••';

export function usePrivacy() {
  return useLocalStorage<boolean>(PRIVACY_KEY, false);
}

const Revealed = createContext(false);

/** Amounts inside are shown even in privacy mode: a confirmation, a signing prompt. */
export function RevealAmounts({ children }: { readonly children: ReactNode }) {
  return <Revealed.Provider value={true}>{children}</Revealed.Provider>;
}

/** Whether an amount here should be masked. */
export function useMaskAmounts(): boolean {
  const [privacy] = usePrivacy();
  const revealed = useContext(Revealed);
  return privacy && !revealed;
}

/** The mask itself; screen readers say "Hidden" rather than five bullets. */
export function Masked() {
  return (
    <span role='img' aria-label={t`Hidden`}>
      {MASK}
    </span>
  );
}
