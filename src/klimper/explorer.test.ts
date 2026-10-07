// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from 'vitest';
import { mempoolBlockUrl, mempoolTxUrl } from './explorer';

const ID = 'cd'.repeat(32);

describe('mempoolTxUrl', () => {
  it('links a mainnet transaction', () => {
    expect(mempoolTxUrl(`0x${ID}`, false)).toBe(
      `https://mempoolxch.space/tx/0x${ID}`,
    );
  });
  it('names the network on testnet', () => {
    expect(mempoolTxUrl(ID, true)).toBe(
      `https://mempoolxch.space/tx/0x${ID}?network=testnet11`,
    );
  });
  it('links nothing else', () => {
    expect(mempoolTxUrl('0x12', false)).toBeNull();
    expect(mempoolTxUrl(null, true)).toBeNull();
  });
});

describe('mempoolBlockUrl', () => {
  it('links a block by height', () => {
    expect(mempoolBlockUrl('7000000', false)).toBe(
      'https://mempoolxch.space/block/7000000',
    );
    expect(mempoolBlockUrl(42, true)).toBe(
      'https://mempoolxch.space/block/42?network=testnet11',
    );
  });
  it('links nothing else', () => {
    expect(mempoolBlockUrl('abc', false)).toBeNull();
    expect(mempoolBlockUrl(undefined, false)).toBeNull();
  });
});
