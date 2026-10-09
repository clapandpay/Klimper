// SPDX-License-Identifier: AGPL-3.0-or-later
// Which XCHandles this wallet holds. Registering a handle mints its NFT into the XCHandles
// collection, named "@handle", and the registry slot points at that NFT (owner and resolved
// launcher id). So the wallet's own NFT database already knows the candidates, without
// telling anybody which wallet asks; each is then checked against the registry, because an
// NFT outlives an expired handle and only the registry says where a name points today.

import { commands } from '@/bindings';
import { puzzleHash } from '@/lib/utils';

/** The XCHandles collection on mainnet; the registry exists on mainnet only. */
export const XCHANDLES_COLLECTION_ID =
  'col12c9jrvt5dlf3km00fnry247v7sxqtqxer33c8tstr9tqt9vkp05q85ejyq';

const API_URL = 'https://api.xchandles.com';
const TIMEOUT_MS = 8000;
const HANDLE_PATTERN = /^[a-z0-9]{3,63}$/;
/** More handles than anyone holds; the list is not paged. */
const MAX_NFTS = 200;

export type OwnedHandleStatus =
  /** Registered, not expired, and pointing at this wallet's NFT. */
  | 'active'
  /** The registration ran out; the name can be registered again. */
  | 'expired'
  /** The name now points at another NFT: this one is a leftover. */
  | 'elsewhere'
  /** The registry could not be asked (offline, or it is catching up). */
  | 'unknown';

export interface OwnedHandle {
  handle: string;
  /** The handle NFT's launcher id (`nft1…`). */
  nftId: string;
  status: OwnedHandleStatus;
  /** Unix seconds, when known. */
  expiration: number | null;
}

/** "@anna" → "anna"; null for a name that is not a handle. */
export function handleFromNftName(name: string | null): string | null {
  const bare = (name ?? '').trim().replace(/^@/, '').toLowerCase();
  return HANDLE_PATTERN.test(bare) ? bare : null;
}

interface SlotResponse {
  slot?: {
    expiration?: number;
    owner_launcher_id?: string;
    resolved_launcher_id?: string;
  };
}

/** What the registry says about `handle` and the NFT with launcher id `launcherHex`. */
export function judge(
  body: SlotResponse,
  launcherHex: string,
  now: number,
): Pick<OwnedHandle, 'status' | 'expiration'> {
  const slot = body.slot;
  if (!slot || typeof slot.expiration !== 'number') {
    return { status: 'unknown', expiration: null };
  }
  const ours = [slot.owner_launcher_id, slot.resolved_launcher_id].some(
    (id) => (id ?? '').replace(/^0x/, '').toLowerCase() === launcherHex,
  );
  if (!ours) return { status: 'elsewhere', expiration: slot.expiration };
  if (slot.expiration <= now) {
    return { status: 'expired', expiration: slot.expiration };
  }
  return { status: 'active', expiration: slot.expiration };
}

async function check(
  handle: string,
  nftId: string,
  signal: AbortSignal,
): Promise<OwnedHandle> {
  let launcherHex: string;
  try {
    launcherHex = puzzleHash(nftId).toLowerCase();
  } catch {
    return { handle, nftId, status: 'unknown', expiration: null };
  }
  try {
    // An expired handle answers 410 unless the safety check is bypassed; we want to say so.
    const response = await fetch(
      `${API_URL}/handle/${handle}?bypass_expiration_safety_check=true`,
      { signal },
    );
    if (response.status === 404) {
      return { handle, nftId, status: 'elsewhere', expiration: null };
    }
    if (!response.ok) {
      return { handle, nftId, status: 'unknown', expiration: null };
    }
    const body = (await response.json()) as SlotResponse;
    return {
      handle,
      nftId,
      ...judge(body, launcherHex, Math.floor(Date.now() / 1000)),
    };
  } catch {
    return { handle, nftId, status: 'unknown', expiration: null };
  }
}

/** The handles whose NFT this wallet holds, checked against the registry, by name. */
export async function loadOwnedHandles(): Promise<OwnedHandle[]> {
  const { nfts } = await commands.getNfts({
    collection_id: XCHANDLES_COLLECTION_ID,
    offset: 0,
    limit: MAX_NFTS,
    sort_mode: 'name',
    include_hidden: true,
  });
  const candidates = nfts.flatMap((nft) => {
    const handle = handleFromNftName(nft.name);
    return handle ? [{ handle, nftId: nft.launcher_id }] : [];
  });
  if (candidates.length === 0) return [];
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
  try {
    const checked = await Promise.all(
      candidates.map((c) => check(c.handle, c.nftId, controller.signal)),
    );
    return checked
      .filter((h) => h.status !== 'elsewhere')
      .sort((a, b) => a.handle.localeCompare(b.handle));
  } finally {
    clearTimeout(timer);
  }
}
