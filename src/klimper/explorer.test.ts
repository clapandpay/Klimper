// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from 'vitest';
import {
  mempoolBlockUrl,
  mempoolCoinUrl,
  mempoolNftUrl,
  mempoolTxUrl,
} from './explorer';

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

describe('mempoolCoinUrl', () => {
  it('links a coin', () => {
    expect(mempoolCoinUrl(ID, false)).toBe(
      `https://mempoolxch.space/coin/0x${ID}`,
    );
    expect(mempoolCoinUrl(`0X${ID.toUpperCase()}`, true)).toBe(
      `https://mempoolxch.space/coin/0x${ID}?network=testnet11`,
    );
  });
  it('links nothing else', () => {
    expect(mempoolCoinUrl('', false)).toBeNull();
    expect(mempoolCoinUrl(`${ID}/../x`, false)).toBeNull();
  });
});

describe('mempoolNftUrl', () => {
  it('links an NFT by its launcher id', () => {
    expect(mempoolNftUrl(ID, false)).toBe(
      `https://mempoolxch.space/nft/0x${ID}`,
    );
    expect(mempoolNftUrl(ID, true)).toBe(
      `https://mempoolxch.space/nft/0x${ID}?network=testnet11`,
    );
  });
  it('links nothing else', () => {
    expect(mempoolNftUrl(undefined, false)).toBeNull();
    expect(mempoolNftUrl('nft1abc', false)).toBeNull();
  });
});
