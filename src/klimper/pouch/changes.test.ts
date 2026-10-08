// SPDX-License-Identifier: AGPL-3.0-or-later
// What the pouch shows when the coins change (TASK-124): merges pull together and become one,
// splits burst out of one, sends leave, receipts drop in, and nothing animates on first fill.

import { describe, expect, it } from 'vitest';
import { change, GONE_MS, GROW_MS, grown } from './changes';
import type { Disc } from './layout';
import type { Body } from './physics';

const body = (id: string, x: number, y: number, r = 10): Body => ({
  id,
  x,
  y,
  r,
  vx: 0,
  vy: 0,
});
const disc = (id: string, r = 10, pending: string | null = null): Disc => ({
  id,
  r,
  locked: pending !== null,
  pending,
});
const fixed = () => 0.5;

describe('the pouch when coins change', () => {
  it('fills an empty pouch without animations', () => {
    const next = change([], [disc('a'), disc('b')], 300, 200, 0, fixed);
    expect(next.bodies.map((b) => b.id)).toEqual(['a', 'b']);
    expect(next.ghosts).toEqual([]);
    expect(next.births).toEqual([]);
  });

  it('merges: the spent coins meet in their middle and the new coin comes out there', () => {
    const before = [
      body('a', 100, 100),
      body('b', 140, 100),
      body('keep', 250, 150),
    ];
    const next = change(
      before,
      [disc('keep'), disc('new', 14)],
      300,
      200,
      1000,
      fixed,
    );

    expect(next.bodies.map((b) => b.id)).toEqual(['keep']);
    expect(next.ghosts).toHaveLength(2);
    for (const g of next.ghosts) expect([g.tx, g.ty]).toEqual([120, 100]);

    expect(next.births).toHaveLength(1);
    const [birth] = next.births;
    expect([birth.body.x, birth.body.y]).toEqual([120, 100]);
    // One coin out: no burst.
    expect([birth.body.vx, birth.body.vy]).toEqual([0, 0]);
    // It appears as the spent coins arrive, and the knock is then.
    expect(birth.at).toBeGreaterThan(1000);
    expect(birth.at).toBeLessThan(1000 + GONE_MS);
    expect(next.meet?.at).toBe(birth.at);
  });

  it('splits: the new coins burst apart from where the spent one was', () => {
    const before = [body('big', 150, 100, 20)];
    const next = change(
      before,
      [disc('p1', 8), disc('p2', 8), disc('p3', 8)],
      300,
      200,
      0,
      fixed,
    );
    expect(next.ghosts).toHaveLength(1);
    expect(next.births).toHaveLength(3);
    for (const b of next.births) {
      expect([b.body.x, b.body.y]).toEqual([150, 100]);
      expect(Math.hypot(b.body.vx, b.body.vy)).toBeGreaterThan(200);
    }
    // Flung in different directions.
    const angles = new Set(
      next.births.map((b) => Math.round(Math.atan2(b.body.vy, b.body.vx) * 10)),
    );
    expect(angles.size).toBe(3);
  });

  it('sends: spent coins with nothing back rise out of the pouch', () => {
    const next = change(
      [body('a', 100, 150), body('b', 200, 150)],
      [disc('b')],
      300,
      200,
      0,
      fixed,
    );
    expect(next.births).toEqual([]);
    expect(next.ghosts).toHaveLength(1);
    expect(next.ghosts[0].ty).toBeLessThan(0);
    expect(next.meet).toBeNull();
  });

  it('receives: new coins with nothing spent drop in from the top, one after another', () => {
    const next = change(
      [body('a', 100, 150)],
      [disc('a'), disc('n1'), disc('n2')],
      300,
      200,
      0,
      fixed,
    );
    expect(next.ghosts).toEqual([]);
    expect(next.births.map((b) => b.body.id)).toEqual(['n1', 'n2']);
    expect(next.births[1].at).toBeGreaterThan(next.births[0].at);
    for (const b of next.births) expect(b.body.y).toBeLessThan(50);
  });

  it('marks the coins of one pending transaction as a group, so they lean together', () => {
    const next = change(
      [body('a', 100, 100), body('b', 200, 100)],
      [disc('a', 10, 'tx1'), disc('b', 10, 'tx1')],
      300,
      200,
      0,
      fixed,
    );
    expect(next.bodies.map((b) => b.group)).toEqual(['tx1', 'tx1']);
  });

  it('grows a new coin in, a little past full, then exactly full', () => {
    expect(grown(100, 50)).toBe(0);
    expect(grown(100, 100 + GROW_MS * 0.7)).toBeGreaterThan(1);
    expect(grown(100, 100 + GROW_MS)).toBe(1);
    expect(grown(undefined, 0)).toBe(1);
  });
});
