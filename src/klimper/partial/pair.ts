// SPDX-License-Identifier: AGPL-3.0-or-later
// Whether Sage's offer state can be a partial offer: exactly one token for one other token.

import { OfferState } from '@/state';
import BigNumber from 'bignumber.js';

/** The one token offered and the one requested, if the offer is exactly that. */
export function partialPair(state: OfferState) {
  const offered = state.offered.tokens;
  const requested = state.requested.tokens;
  const nothingElse =
    state.offered.nfts.filter((n) => n).length === 0 &&
    state.requested.nfts.filter((n) => n).length === 0 &&
    state.offered.options.filter((o) => o).length === 0 &&
    state.requested.options.filter((o) => o).length === 0;
  if (!nothingElse || offered.length !== 1 || requested.length !== 1) {
    return null;
  }
  const [give] = offered;
  const [want] = requested;
  // A token row the user has not picked an asset for yet has the id '' (XCH is null).
  if (give.asset_id === '' || want.asset_id === '') return null;
  if (give.asset_id === want.asset_id) return null;
  const amount = (value: unknown) => BigNumber(String(value ?? ''));
  if (!amount(give.amount).gt(0) || !amount(want.amount).gt(0)) return null;
  return { give, want };
}
