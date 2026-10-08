// SPDX-License-Identifier: AGPL-3.0-or-later
// What the pouch shows when the coins change (TASK-124). Pure: given the coins in the pouch and
// the coins the wallet has now, where each goes. It reads the coin set, not the buttons, so it
// shows what the chain did, whatever asked for it:
//
// - Coins spent and coins made together (a combine, a split, a payment with change): the spent
//   coins pull into one point and vanish; the new coins come out of that point, flung apart when
//   there are several (a split). Coins worth the same go in as come out, so it is one movement.
// - Coins spent with nothing back (everything sent): they rise out of the pouch.
// - Coins arrived with nothing spent (received): they drop in from the top.
// - Coins in one pending transaction lean together (`group`) until it confirms.

import type { Disc } from './layout';
import type { Body } from './physics';

/** How long spent coins take to pull together or leave. */
export const GONE_MS = 420;
/** How long a new coin takes to grow to its size. */
export const GROW_MS = 320;

/** A spent coin on its way out: drawn, not simulated. */
export interface Ghost {
  r: number;
  x: number;
  y: number;
  /** Where it goes. */
  tx: number;
  ty: number;
  start: number;
}

/** A new coin, entering the world at `at`. */
export interface Birth {
  body: Body;
  at: number;
}

export interface Change {
  /** The coins that stay, in their places. */
  bodies: Body[];
  ghosts: Ghost[];
  births: Birth[];
  /** Spent coins became new ones here (a knock when they meet), or null. */
  meet: { x: number; y: number; at: number; size: number } | null;
}

export function change(
  before: Body[],
  discs: Disc[],
  width: number,
  height: number,
  now: number,
  random: () => number = Math.random,
): Change {
  const was = new Map(before.map((b) => [b.id, b]));
  const is = new Set(discs.map((d) => d.id));
  const bodies: Body[] = [];
  const added: Disc[] = [];
  for (const d of discs) {
    const old = was.get(d.id);
    if (old) {
      bodies.push({
        ...old,
        r: d.r,
        held: false,
        group: d.pending ?? undefined,
      });
    } else {
      added.push(d);
    }
  }
  const removed = before.filter((b) => !is.has(b.id));

  // The first fill of an empty pouch: everything simply drops in.
  if (before.length === 0) {
    return {
      bodies: added.map((d, i) => dropIn(d, i, width, random)),
      ghosts: [],
      births: [],
      meet: null,
    };
  }

  if (removed.length === 0) {
    // Received: the new coins fall in from the top, one after the other.
    return {
      bodies,
      ghosts: [],
      births: added.map((d, i) => ({
        body: { ...dropIn(d, 0, width, random), vy: 120 },
        at: now + i * 90,
      })),
      meet: null,
    };
  }

  // Where the spent coins meet: their middle, weighted by size.
  const weight = removed.reduce((sum, b) => sum + b.r * b.r, 0);
  const cx = removed.reduce((sum, b) => sum + b.x * b.r * b.r, 0) / weight;
  const cy = removed.reduce((sum, b) => sum + b.y * b.r * b.r, 0) / weight;

  if (added.length === 0) {
    // Sent away: up and out.
    return {
      bodies,
      ghosts: removed.map((b) => ({
        ...ghostOf(b, now),
        tx: b.x,
        ty: -b.r * 2,
      })),
      births: [],
      meet: null,
    };
  }

  const at = now + GONE_MS * 0.8;
  const burst = added.length > 1;
  const births = added.map((d, i): Birth => {
    const angle = (i / added.length) * Math.PI * 2 + random() * 0.6;
    const speed = burst ? 260 + random() * 160 : 0;
    return {
      body: {
        id: d.id,
        r: d.r,
        x: clamp(cx, d.r, width - d.r),
        y: clamp(cy, d.r, height - d.r),
        vx: Math.cos(angle) * speed,
        vy: Math.sin(angle) * speed,
        born: at,
      },
      at,
    };
  });
  const biggest = Math.max(...added.map((d) => d.r));
  return {
    bodies,
    ghosts: removed.map((b) => ({ ...ghostOf(b, now), tx: cx, ty: cy })),
    births,
    meet: {
      x: cx,
      y: cy,
      at,
      size: biggest / Math.max(...discs.map((d) => d.r)),
    },
  };
}

function ghostOf(b: Body, now: number): Ghost {
  return { r: b.r, x: b.x, y: b.y, tx: b.x, ty: b.y, start: now };
}

function dropIn(d: Disc, i: number, width: number, random: () => number): Body {
  return {
    id: d.id,
    r: d.r,
    x: d.r + random() * Math.max(1, width - 2 * d.r),
    y: d.r + (i % 4) * d.r,
    vx: 0,
    vy: 0,
    group: d.pending ?? undefined,
  };
}

function clamp(v: number, lo: number, hi: number) {
  return Math.min(Math.max(v, lo), Math.max(lo, hi));
}

/** 0..1 through an animation; eased so a coin speeds up into the meeting point. */
export function progress(start: number, ms: number, now: number): number {
  const t = Math.min(1, Math.max(0, (now - start) / ms));
  return t * t;
}

/** A new coin's size: it pops a little past full and settles. */
export function grown(born: number | undefined, now: number): number {
  if (born === undefined || now >= born + GROW_MS) return 1;
  if (now <= born) return 0;
  const t = (now - born) / GROW_MS;
  const s = 1.7;
  const u = t - 1;
  return 1 + (s + 1) * u * u * u + s * u * u;
}
