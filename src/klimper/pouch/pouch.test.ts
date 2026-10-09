// SPDX-License-Identifier: AGPL-3.0-or-later
// The coin pouch's world and layout (TASK-124): coins settle and then cost nothing, stay in
// the box, knock audibly only when they really hit, and are sized and counted as promised.

import type { CoinRecord } from '@/bindings';
import { describe, expect, it } from 'vitest';
import { assetHue, DUST_SELECT, layoutCoins, MAX_DISCS } from './layout';
import { stamp } from './sprite';
import { Body, G, World } from './physics';

function heap(n: number, r = 12): Body[] {
  return Array.from({ length: n }, (_, i) => ({
    id: `c${i}`,
    x: 20 + ((i * 37) % 260),
    y: 20 + ((i * 13) % 60),
    vx: 0,
    vy: 0,
    r,
  }));
}

function settle(world: World, max = 2000): number {
  for (let i = 0; i < max; i++) {
    world.step();
    if (world.asleep) return i;
  }
  return max;
}

describe('the pouch world', () => {
  it('lets a heap of coins come to rest, and then does nothing', () => {
    const world = new World(300, 200);
    world.set(heap(30));
    expect(settle(world)).toBeLessThan(2000);
    const before = world.bodies.map((b) => [b.x, b.y]);
    expect(world.step()).toEqual([]);
    expect(world.bodies.map((b) => [b.x, b.y])).toEqual(before);
  });

  it('keeps every coin inside the box and apart from the others', () => {
    const world = new World(300, 200);
    world.set(heap(30));
    settle(world);
    for (const b of world.bodies) {
      expect(b.x).toBeGreaterThanOrEqual(b.r - 0.01);
      expect(b.x).toBeLessThanOrEqual(300 - b.r + 0.01);
      expect(b.y).toBeLessThanOrEqual(200 - b.r + 0.01);
    }
    for (const a of world.bodies) {
      for (const b of world.bodies) {
        if (a === b) continue;
        // A little give in a heap, never one coin inside another.
        expect(Math.hypot(a.x - b.x, a.y - b.y)).toBeGreaterThan(
          (a.r + b.r) * 0.8,
        );
      }
    }
  });

  it('wakes when the phone turns, and the coins roll to the low side', () => {
    const world = new World(300, 200);
    world.set(heap(10));
    settle(world);
    world.tilt(1, 0); // the right edge is down now
    expect(world.asleep).toBe(false);
    settle(world);
    const meanX =
      world.bodies.reduce((sum, b) => sum + b.x, 0) / world.bodies.length;
    expect(meanX).toBeGreaterThan(200);
  });

  it('is not woken by a turn the coins would not notice', () => {
    const world = new World(300, 200);
    world.set(heap(5));
    settle(world);
    world.tilt(0.01, 1);
    expect(world.asleep).toBe(true);
  });

  it('hears a coin that drops onto another, not one that lies still', () => {
    const world = new World(300, 200);
    world.set([
      { id: 'low', x: 150, y: 188, vx: 0, vy: 0, r: 12 },
      { id: 'high', x: 150, y: 20, vx: 0, vy: 0, r: 12 },
    ]);
    let heard = 0;
    for (let i = 0; i < 120; i++) heard += world.step().length;
    expect(heard).toBeGreaterThan(0);
    settle(world);
    world.wake();
    expect(world.step()).toEqual([]);
  });

  it('leaves a coin held by a finger where the finger is', () => {
    const world = new World(300, 200);
    world.set([{ id: 'held', x: 100, y: 50, vx: 0, vy: 0, r: 12, held: true }]);
    for (let i = 0; i < 60; i++) world.step();
    expect(world.bodies[0].y).toBe(50);
    expect(G).toBeGreaterThan(0);
  });
});

function coin(id: string, amount: number, extra: Partial<CoinRecord> = {}) {
  return {
    coin_id: id,
    address: '',
    amount,
    transaction_id: null,
    offer_id: null,
    clawback_timestamp: null,
    created_height: 1,
    spent_height: null,
    spent_timestamp: null,
    created_timestamp: null,
    ...extra,
  } as CoinRecord;
}

describe('the pouch layout', () => {
  it('draws a bigger coin bigger, on a log scale', () => {
    const { discs } = layoutCoins(
      [coin('a', 1), coin('b', 1_000), coin('c', 1_000_000)],
      320,
      200,
      false,
    );
    const r = Object.fromEntries(discs.map((d) => [d.id, d.r]));
    expect(r.c).toBeGreaterThan(r.b);
    expect(r.b).toBeGreaterThan(r.a);
    // Log scale: equal steps for equal ratios.
    expect(r.c - r.b).toBeCloseTo(r.b - r.a, 5);
  });

  it('draws every coin the same size in privacy mode', () => {
    const { discs } = layoutCoins(
      [coin('a', 1), coin('b', 1_000_000)],
      320,
      200,
      true,
    );
    expect(discs[0].r).toBe(discs[1].r);
  });

  it('never fills more of the pouch than it can hold', () => {
    const coins = Array.from({ length: 100 }, (_, i) =>
      coin(`c${i}`, 10 ** (i % 12)),
    );
    const { discs } = layoutCoins(coins, 320, 200, false);
    const area = discs.reduce((sum, d) => sum + Math.PI * d.r * d.r, 0);
    expect(area).toBeLessThanOrEqual(320 * 200 * 0.43);
  });

  it('counts the smallest coins as dust beyond the discs it draws', () => {
    const coins = Array.from({ length: MAX_DISCS + 7 }, (_, i) =>
      coin(`c${i}`, i + 1),
    );
    const { discs, dust } = layoutCoins(coins, 320, 200, false);
    expect(discs).toHaveLength(MAX_DISCS);
    expect(dust).toEqual(['c0', 'c1', 'c2', 'c3', 'c4', 'c5', 'c6']);
  });

  it('leaves locked coins out of the dust to select, and selects at most DUST_SELECT', () => {
    const coins = Array.from({ length: MAX_DISCS + DUST_SELECT + 50 }, (_, i) =>
      coin(`c${i}`, i + 1, i === 0 ? { transaction_id: 'tx' } : {}),
    );
    const { dust } = layoutCoins(coins, 320, 200, false);
    expect(dust).not.toContain('c0');
    expect(dust).toHaveLength(DUST_SELECT);
    expect(dust[0]).toBe('c1');
  });

  it('marks coins in an offer and coins in a pending transaction apart', () => {
    const { discs } = layoutCoins(
      [
        coin('free', 5),
        coin('offered', 5, { offer_id: 'o' }),
        coin('leaving', 5, { transaction_id: 'tx' }),
      ],
      320,
      200,
      false,
    );
    const of = (id: string) => discs.find((d) => d.id === id);
    expect([of('free')?.offered, of('free')?.pending]).toEqual([false, null]);
    expect([of('offered')?.offered, of('offered')?.pending]).toEqual([
      true,
      null,
    ]);
    expect(of('leaving')?.pending).toBe('tx');
  });

  it('leaves coins in an offer out of the dust to select', () => {
    const coins = Array.from({ length: MAX_DISCS + 3 }, (_, i) =>
      coin(`c${i}`, i + 1, i === 1 ? { offer_id: 'o' } : {}),
    );
    const { dust } = layoutCoins(coins, 320, 200, false);
    expect(dust).toEqual(['c0', 'c2']);
  });
});

describe('what a coin shows', () => {
  it('strikes the ticker as the asset spells it, at most 4 characters', () => {
    expect(stamp('XCH', null)).toBe('XCH');
    expect(stamp('wUSDC.b', 'a1')).toBe('wUSD');
    expect(stamp('sbx', 'a1')).toBe('sbx');
    // Never half of a character.
    expect(stamp('🚀🚀🚀🚀🚀', 'a1')).toBe('🚀🚀🚀🚀');
  });

  it('gives a token that would read as XCH a plain coin and no green', () => {
    for (const ticker of ['xch', 'XCH-PEPE', ' XCH fake', 'TXCH'])
      expect(stamp(ticker, 'a1')).toBe('');
    expect(stamp('TXCH', null)).toBe('TXCH');
    expect(assetHue(null)).toBe(140);
    for (let i = 0; i < 2000; i++)
      expect(Math.abs(assetHue(`cat${i}`) - 140)).toBeGreaterThanOrEqual(30);
  });
});
