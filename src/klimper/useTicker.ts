// SPDX-License-Identifier: AGPL-3.0-or-later
// The network's name for its coin, as the wallet shows it everywhere else: `XCH` on
// mainnet, `TXCH` on testnet11 (Sage sets it from the network config on login). Its own
// module so the approval screens can be tested without the wallet state, which talks to
// the backend as soon as it is imported.

import { useWalletState } from '@/state';

export function useTicker(): string {
  return useWalletState((state) => state.sync.unit.ticker) || 'XCH';
}
