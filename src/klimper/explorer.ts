// SPDX-License-Identifier: AGPL-3.0-or-later
// Where to look a transaction, a block, a coin or an NFT up: mempoolxch.space, the Chia explorer.
// Its network is a setting of the visitor's own, so a testnet link names it (`?network=testnet11`).

const EXPLORER = 'https://mempoolxch.space';

function onNetwork(url: string, testnet: boolean): string {
  return testnet ? `${url}?network=testnet11` : url;
}

/** `id` as 64 lowercase hex digits, or null when it is not a 32-byte hex id. */
function id32(id: string | null | undefined): string | null {
  const hex = (id ?? '').trim().replace(/^0x/i, '').toLowerCase();
  return /^[0-9a-f]{64}$/.test(hex) ? hex : null;
}

/** The explorer page of a transaction, or null when `txId` is not a 32-byte hex id. */
export function mempoolTxUrl(
  txId: string | null | undefined,
  testnet: boolean,
): string | null {
  const id = id32(txId);
  return id ? onNetwork(`${EXPLORER}/tx/0x${id}`, testnet) : null;
}

/**
 * The explorer page of a block. A wallet transaction is the coins spent and created at one height,
 * so this is where its spends are seen. Null unless `height` is a whole block height.
 */
export function mempoolBlockUrl(
  height: string | number | null | undefined,
  testnet: boolean,
): string | null {
  const h = String(height ?? '').trim();
  if (!/^\d{1,9}$/.test(h)) return null;
  return onNetwork(`${EXPLORER}/block/${Number(h)}`, testnet);
}

/** The explorer page of a coin, or null when `coinId` is not a 32-byte hex id. */
export function mempoolCoinUrl(
  coinId: string | null | undefined,
  testnet: boolean,
): string | null {
  const id = id32(coinId);
  return id ? onNetwork(`${EXPLORER}/coin/0x${id}`, testnet) : null;
}

/** The explorer page of an NFT by its launcher id, or null when that is not a 32-byte hex id. */
export function mempoolNftUrl(
  launcherId: string | null | undefined,
  testnet: boolean,
): string | null {
  const id = id32(launcherId);
  return id ? onNetwork(`${EXPLORER}/nft/0x${id}`, testnet) : null;
}
