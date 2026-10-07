// SPDX-License-Identifier: AGPL-3.0-or-later
// Where to look a transaction or a block up: mempoolxch.space, the Chia explorer. Its network is a
// setting of the visitor's own, so a testnet link names it (`?network=testnet11`).

const EXPLORER = 'https://mempoolxch.space';

/** The explorer page of a transaction, or null when `txId` is not a 32-byte hex id. */
export function mempoolTxUrl(
  txId: string | null | undefined,
  testnet: boolean,
): string | null {
  const id = (txId ?? '').trim().replace(/^0x/i, '').toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(id)) return null;
  const url = `${EXPLORER}/tx/0x${id}`;
  return testnet ? `${url}?network=testnet11` : url;
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
  const url = `${EXPLORER}/block/${Number(h)}`;
  return testnet ? `${url}?network=testnet11` : url;
}
