// SPDX-License-Identifier: AGPL-3.0-or-later
// Partial offers (CHIP-0052, TASK-164): the only file that names the commands behind the
// screens (docs/partial-offers.md, part 3). Today they are Klimper's plugin,
// `tauri-plugin-klimper-partial-offers`. When Sage ships partial offers, this file points at
// Sage's commands and the screens stay as they are.

import { t } from '@lingui/core/macro';
import { invoke } from '@tauri-apps/api/core';

/** One side's asset. `asset_id` is null for XCH. */
export interface PartialSide {
  asset_id: string | null;
  hidden_puzzle_hash: string | null;
  revocable: boolean;
}

export type PartialStatus =
  | 'open'
  | 'partly_filled'
  | 'filled'
  | 'cancelled'
  | 'expired'
  | 'unknown'
  /** This wallet sent a cancel; the chain has not confirmed it yet. */
  | 'cancelling';

export interface PartialFill {
  coin_id: string;
  /** What the taker received of the offered asset, in mojos. */
  bought: string;
  /** What the taker paid of the requested asset, in mojos. */
  paid: string;
}

/** A partial offer as the screens show it. Amounts are mojos, as strings. */
export interface PartialOfferView {
  /** The newest `partial1…` text: the one to take or share. */
  offer: string;
  offered: PartialSide;
  requested: PartialSide;
  initial_offered: string;
  initial_requested: string;
  remaining_offered: string;
  remaining_requested: string;
  min_fill_requested: string;
  min_fill_offered: string;
  /** The blockchain fee each fill must pay, apart from the network fee. */
  required_fee: string | null;
  /** Unix seconds. */
  expires_at: number | null;
  maker_address: string;
  coin_id: string;
  first: boolean;
  status: PartialStatus;
  fills: PartialFill[];
  is_mine: boolean;
}

/** A cancel this wallet sent that the chain has not confirmed yet. */
export interface PendingCancel {
  transaction_id: string;
  /** The partial coin the cancel spends. */
  coin_id: string;
  /** Unix seconds. */
  sent_at: number;
}

export interface MyPartialOffer {
  id: string;
  created_at: number;
  fills: PartialFill[];
  cancel: PendingCancel | null;
  view: PartialOfferView;
}

export interface TakeResult {
  transaction_id: string;
  bought: string;
  paid: string;
  next_offer: string | null;
}

export interface MakeRequest {
  offered_asset_id: string | null;
  offered_amount: string;
  requested_asset_id: string | null;
  requested_amount: string;
  min_fill_requested: string;
  required_fee: string | null;
  expires_at: number | null;
}

/** What a command answers when it refuses. */
export interface PartialError {
  code:
    | 'not_partial'
    | 'invalid'
    | 'wallet'
    | 'offer_changed'
    | 'closed'
    | 'no_wallet'
    | 'no_peer'
    | 'no_signing_key'
    | 'not_mine';
  message: string;
}

export function isPartialError(error: unknown): error is PartialError {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    'message' in error
  );
}

const call = <T>(command: string, args: Record<string, unknown> = {}) =>
  invoke<T>(`plugin:klimper-partial-offers|${command}`, args);

export const partialOffers = {
  view: (offer: string) =>
    call<PartialOfferView>('view_partial_offer', { offer }),
  take: (offer: string, amount: string, fee: string) =>
    call<TakeResult>('take_partial_offer', { offer, amount, fee }),
  make: (request: MakeRequest) =>
    call<{ id: string; offer: string }>('make_partial_offer', { request }),
  cancel: (id: string, fee: string) =>
    call<{ transaction_id: string }>('cancel_partial_offer', { id, fee }),
  list: () => call<MyPartialOffer[]>('get_partial_offers'),
  forget: (id: string) => call<boolean>('delete_partial_offer', { id }),
};

/**
 * Why a partial offer could not be read. A bad checksum almost always means the text was cut
 * on the way (messengers split long texts), so say that rather than the bech32 error.
 */
export function viewErrorText(error: unknown): string {
  if (!isPartialError(error)) return String(error);
  if (error.code === 'not_partial' && /checksum/i.test(error.message)) {
    return t`This partial offer text is incomplete or changed: it was probably cut off when it was sent. Scan its QR code with Klimper instead, or copy the whole text again.`;
  }
  return error.message;
}
