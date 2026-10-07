// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it, vi } from 'vitest';
import { Coalescer, ReceiveDetector, sampleOf, toMojos } from './receive';

const wallet = (balance: bigint, synced = true, address = 'xch1abc') => ({
  address,
  balance,
  synced,
});
const NONE = wallet(0n, true, 'Unknown');

describe('the receive detector', () => {
  it('is silent on the first sample, then hears an increase', () => {
    const d = new ReceiveDetector();
    expect(d.observe(wallet(100n))).toEqual({ kind: 'quiet' });
    expect(d.observe(wallet(150n))).toEqual({ kind: 'received', mojos: 50n });
    expect(d.observe(wallet(150n))).toEqual({ kind: 'quiet' });
  });

  it('is silent on a spend', () => {
    const d = new ReceiveDetector();
    d.observe(wallet(100n));
    expect(d.observe(wallet(40n))).toEqual({ kind: 'quiet' });
    expect(d.observe(wallet(60n))).toEqual({ kind: 'received', mojos: 20n });
  });

  it('follows a catching-up wallet silently', () => {
    const d = new ReceiveDetector();
    d.observe(wallet(0n, false));
    expect(d.observe(wallet(500n, false))).toEqual({ kind: 'quiet' });
    expect(d.observe(wallet(900n, false))).toEqual({ kind: 'quiet' });
    // Synced now: only what came after the catch-up counts.
    expect(d.observe(wallet(900n, true))).toEqual({ kind: 'quiet' });
    expect(d.observe(wallet(1000n, true))).toEqual({
      kind: 'received',
      mojos: 100n,
    });
  });

  it('starts over when the wallet goes away or is switched', () => {
    const d = new ReceiveDetector();
    expect(d.observe(NONE)).toEqual({ kind: 'quiet' });
    d.observe(wallet(100n));
    expect(d.observe(NONE)).toEqual({ kind: 'reset' });
    // Another wallet, richer: not a receive.
    expect(d.observe(wallet(5000n, true, 'xch1other'))).toEqual({
      kind: 'quiet',
    });
    expect(d.observe(wallet(5001n, true, 'xch1other'))).toEqual({
      kind: 'received',
      mojos: 1n,
    });
  });

  it('can be rebased by the caller when the fingerprint changed', () => {
    const d = new ReceiveDetector();
    d.observe(wallet(100n));
    expect(d.observe(wallet(5000n))).toEqual({
      kind: 'received',
      mojos: 4900n,
    });
    d.rebase(wallet(5000n));
    expect(d.observe(wallet(5000n))).toEqual({ kind: 'quiet' });
  });

  it('reads Sage sync state', () => {
    const sync = {
      receive_address: 'xch1abc',
      selectable_balance: '1234',
      synced_coins: 3,
      total_coins: 3,
    } as unknown as Parameters<typeof sampleOf>[0];
    expect(sampleOf(sync)).toEqual({
      address: 'xch1abc',
      balance: 1234n,
      synced: true,
    });
    expect(toMojos(12)).toBe(12n);
    expect(toMojos('not a number')).toBe(0n);
    expect(toMojos(Number.NaN)).toBe(0n);
    expect(toMojos('12.5')).toBe(0n);
  });
});

describe('the coalescer', () => {
  it('sums increases within the window into one report', () => {
    vi.useFakeTimers();
    const flush = vi.fn();
    const c = new Coalescer(1500, flush);
    c.add(10n);
    vi.advanceTimersByTime(500);
    c.add(20n);
    vi.advanceTimersByTime(999);
    expect(flush).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(flush).toHaveBeenCalledExactlyOnceWith(30n);
    // A later one is its own report.
    c.add(5n);
    vi.advanceTimersByTime(1500);
    expect(flush).toHaveBeenLastCalledWith(5n);
    expect(flush).toHaveBeenCalledTimes(2);
    vi.useRealTimers();
  });

  it('forgets everything when cancelled', () => {
    vi.useFakeTimers();
    const flush = vi.fn();
    const c = new Coalescer(1500, flush);
    c.add(10n);
    c.cancel();
    vi.advanceTimersByTime(3000);
    expect(flush).not.toHaveBeenCalled();
    vi.useRealTimers();
  });
});
