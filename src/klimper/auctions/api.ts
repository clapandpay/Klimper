// SPDX-License-Identifier: AGPL-3.0-or-later
// NFT auctions: the only file that names the commands behind the screens. Today they are
// Klimper's plugin, `tauri-plugin-klimper-auctions` (docs/auctions.md). Amounts are mojos as
// strings; times are Unix seconds.

import { t } from '@lingui/core/macro';
import { invoke } from '@tauri-apps/api/core';

export type AuctionStatus =
  | 'live'
  | 'ending'
  | 'sold'
  | 'unsold'
  | 'cancelled'
  | 'unknown';

export type Increment =
  | { kind: 'flat'; amount: string }
  | { kind: 'percent'; bps: number };

export interface AuctionBid {
  amount: string;
  address: string;
  is_mine: boolean;
}

export interface AuctionView {
  id: string;
  nft_id: string;
  nft_address: string;
  nft_coin_id: string;
  bid_asset_id: string | null;
  minimum_bid: string;
  increment: Increment;
  end_time: number;
  grace_period: number;
  buyers_premium_bps: number;
  buyers_premium_address: string;
  commission_bps: number;
  commission_address: string;
  royalty_bps: number;
  royalty_address: string;
  seller_address: string;
  winning_bid: AuctionBid | null;
  next_minimum_bid: string;
  next_minimum_cost: string;
  last_spend_time: number | null;
  in_grace: boolean;
  status: AuctionStatus;
  can_bid: boolean;
  cannot_bid_reason: string | null;
  can_end: boolean;
  is_mine: boolean;
  can_cancel: boolean;
}

export interface StartRequest {
  nft_id: string;
  minimum_bid: string;
  increment: Increment;
  end_time: number;
  grace_period: number;
  fee: string | null;
}

export interface Sent {
  id: string;
  transaction_id: string;
}

export interface MyAuction {
  id: string;
  role: 'started' | 'bid';
  created_at: number;
  view: AuctionView | null;
  error: string | null;
}

export interface AuctionError {
  code:
    | 'invalid'
    | 'not_auction'
    | 'closed'
    | 'wallet'
    | 'no_wallet'
    | 'no_peer'
    | 'not_mine';
  message: string;
}

export function isAuctionError(error: unknown): error is AuctionError {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    'message' in error
  );
}

export function auctionErrorText(error: unknown): string {
  if (!isAuctionError(error)) return String(error);
  switch (error.code) {
    case 'no_peer':
      return t`No connection to the network. Try again in a moment.`;
    case 'no_wallet':
      return t`Open a wallet first.`;
    default:
      return error.message;
  }
}

const call = <T>(command: string, args?: Record<string, unknown>) =>
  invoke<T>(`plugin:klimper-auctions|${command}`, args);

export const auctions = {
  view: (id: string) => call<AuctionView>('view_auction', { id }),
  start: (request: StartRequest) => call<Sent>('start_auction', { request }),
  bid: (id: string, amount: string, fee: string) =>
    call<Sent>('bid_auction', { id, amount, fee }),
  end: (id: string, fee: string) => call<Sent>('end_auction', { id, fee }),
  cancel: (id: string, fee: string) =>
    call<Sent>('cancel_auction', { id, fee }),
  list: () => call<MyAuction[]>('get_auctions'),
  forget: (id: string) => call<boolean>('delete_auction', { id }),
};

/** What a QR code or a shared text names an auction by. */
export function auctionShareText(id: string): string {
  return `auction:${id}`;
}

/** An auction id from what the user pasted: `auction:0x…`, `0x…` or 64 hex digits. */
export function parseAuctionId(text: string): string | null {
  const match = /^(?:auction:)?(?:0x)?([0-9a-f]{64})$/i.exec(text.trim());
  return match ? `0x${match[1].toLowerCase()}` : null;
}
