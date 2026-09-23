// SPDX-License-Identifier: AGPL-3.0-or-later
// XCHandles (https://www.xchandles.com): "@anna" instead of "xch1...". A handle is a slot in an
// on-chain registry that points to an NFT; whoever holds that NFT receives the payment.
// API reference: https://docs.xchandles.com/api

import { toAddress } from '@/lib/utils';

const API_URL = 'https://api.xchandles.com';
const TIMEOUT_MS = 8000;
const HANDLE_PATTERN = /^[a-z0-9]{3,63}$/;

export type HandleErrorCode =
  | 'invalid'
  | 'not_found'
  | 'expired'
  | 'not_payable'
  | 'unavailable';

export class HandleError extends Error {
  constructor(public code: HandleErrorCode) {
    super(code);
  }
}

export interface ResolvedHandle {
  handle: string;
  address: string;
  // Unix seconds.
  expiration: number;
  // An ownership change is queued on chain; the address may change soon.
  pendingTransfer: boolean;
}

// An input is a handle only with a leading "@": a bare address also fits the handle alphabet.
export function isHandleInput(input: string): boolean {
  return input.trim().startsWith('@');
}

// Returns the bare handle ("anna") or null. Lowercasing is the only normalization applied; the
// registry itself accepts none.
export function parseHandle(input: string): string | null {
  const trimmed = input.trim();
  if (!trimmed.startsWith('@')) return null;
  const handle = trimmed.slice(1).toLowerCase();
  return HANDLE_PATTERN.test(handle) ? handle : null;
}

interface HandleResponse {
  handle: string;
  slot: { expiration: number };
  resolved_singleton: {
    melted: boolean;
    nft: { p2_puzzle_hash: string } | null;
  } | null;
  pending_transfer: unknown;
}

export async function resolveHandle(
  input: string,
  prefix: string,
  fetchFn: typeof fetch = fetch,
): Promise<ResolvedHandle> {
  const handle = parseHandle(input);
  if (!handle) throw new HandleError('invalid');

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  let response: Response;
  try {
    response = await fetchFn(`${API_URL}/handle/${handle}`, {
      signal: controller.signal,
    });
  } catch {
    throw new HandleError('unavailable');
  } finally {
    clearTimeout(timer);
  }

  if (response.status === 400) throw new HandleError('invalid');
  if (response.status === 404) throw new HandleError('not_found');
  if (response.status === 410) throw new HandleError('expired');
  if (!response.ok) throw new HandleError('unavailable');

  let data: HandleResponse;
  try {
    data = await response.json();
  } catch {
    throw new HandleError('unavailable');
  }

  const singleton = data.resolved_singleton;
  const puzzleHash = singleton?.nft?.p2_puzzle_hash;

  if (data.handle !== handle) throw new HandleError('unavailable');
  if (!singleton || singleton.melted || !puzzleHash) {
    throw new HandleError('not_payable');
  }
  if (!/^[0-9a-f]{64}$/.test(puzzleHash)) throw new HandleError('unavailable');
  if (data.slot.expiration * 1000 <= Date.now()) {
    throw new HandleError('expired');
  }

  return {
    handle,
    address: toAddress(puzzleHash, prefix),
    expiration: data.slot.expiration,
    pendingTransfer: data.pending_transfer != null,
  };
}
