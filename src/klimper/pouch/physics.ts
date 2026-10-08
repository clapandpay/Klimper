// SPDX-License-Identifier: AGPL-3.0-or-later
// The coins in the pouch (TASK-124): discs in a box, gravity from the phone's tilt, knocks
// between them. Small on purpose: circles only, a fixed step, a sort-and-sweep pass for the
// pairs, and a world that falls asleep when nothing moves, so a resting pouch costs nothing.
//
// Units: CSS pixels and seconds. Pure: no DOM, no timers; the view drives it.

export interface Body {
  id: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  r: number;
  /** Held by a finger: moved by the pointer, not by the world. */
  held?: boolean;
}

/** A knock worth hearing: how big the coins are (0..1) and how hard they met (0..1). */
export interface Impact {
  size: number;
  force: number;
  /** Which coin, for a tap's pitch: the smaller of the two. */
  id: string;
}

export const STEP = 1 / 60;
/** px/s² for one g in the plane of the screen. */
export const G = 1800;
const RESTITUTION = 0.3;
/** px/s: a contact slower than this does not bounce at all, so a heap can come to rest. */
const DEAD_BELOW = 60;
/** Velocity kept per second while rolling: coins on leather stop quickly. */
const DAMPING_PER_S = 0.08;
/** A knock slower than this (px/s, along the line of the coins) is silent. */
const HEARD_FROM = 140;
/** px a coin may still move in one step and count as lying still. A heap under gravity keeps
 * a little velocity (pushed down, pushed back), so the test is how far coins actually moved. */
const REST_MOVE = 0.15;
const REST_STEPS = 30;

export class World {
  bodies: Body[] = [];
  gx = 0;
  gy = G;
  private resting = 0;
  /** Largest radius, for the impact's size. */
  private rMax = 1;

  constructor(
    public width: number,
    public height: number,
  ) {}

  set(bodies: Body[]) {
    this.bodies = bodies;
    this.rMax = Math.max(1, ...bodies.map((b) => b.r));
    this.wake();
  }

  /** Gravity in the plane of the screen, in g (x right, y down). */
  tilt(gx: number, gy: number) {
    const nx = gx * G;
    const ny = gy * G;
    // A change the coins would not notice does not wake them.
    if (Math.hypot(nx - this.gx, ny - this.gy) > G * 0.03) this.wake();
    this.gx = nx;
    this.gy = ny;
  }

  /** A jolt of the hand (in g): the coins lag behind it. */
  jolt(ax: number, ay: number) {
    for (const b of this.bodies) {
      if (b.held) continue;
      const spread = 0.8 + Math.random() * 0.4;
      b.vx -= ax * 260 * spread;
      b.vy -= ay * 260 * spread;
    }
    this.wake();
  }

  wake() {
    this.resting = 0;
  }

  get asleep(): boolean {
    return this.resting >= REST_STEPS;
  }

  /** One fixed step. Returns the knocks worth hearing, strongest first. */
  step(dt = STEP): Impact[] {
    if (this.asleep) return [];
    const impacts: Impact[] = [];
    const keep = Math.pow(DAMPING_PER_S, dt);
    const before = this.bodies.map((b) => [b.x, b.y]);

    for (const b of this.bodies) {
      if (b.held) continue;
      b.vx = (b.vx + this.gx * dt) * keep;
      b.vy = (b.vy + this.gy * dt) * keep;
      b.x += b.vx * dt;
      b.y += b.vy * dt;
    }

    // Two passes settle a heap without visible overlap.
    for (let pass = 0; pass < 2; pass++) {
      this.collide(pass === 0 ? impacts : null);
      for (const b of this.bodies) this.walls(b, pass === 0 ? impacts : null);
    }

    let moved = 0;
    this.bodies.forEach((b, i) => {
      const [x, y] = before[i];
      moved = Math.max(moved, Math.abs(b.x - x), Math.abs(b.y - y));
    });
    this.resting = moved < REST_MOVE ? this.resting + 1 : 0;
    if (this.asleep) {
      for (const b of this.bodies) {
        b.vx = 0;
        b.vy = 0;
      }
    }
    return impacts.sort((a, b) => b.force - a.force);
  }

  private walls(b: Body, impacts: Impact[] | null) {
    const hit = (speed: number) => {
      if (impacts && speed > HEARD_FROM) {
        impacts.push({
          id: b.id,
          size: b.r / this.rMax,
          force: Math.min(1, (speed - HEARD_FROM) / 900),
        });
      }
    };
    if (b.x < b.r) {
      b.x = b.r;
      if (b.vx < 0) {
        hit(-b.vx);
        b.vx = -b.vx * bounce(b.vx);
      }
    } else if (b.x > this.width - b.r) {
      b.x = this.width - b.r;
      if (b.vx > 0) {
        hit(b.vx);
        b.vx = -b.vx * bounce(b.vx);
      }
    }
    if (b.y < b.r) {
      b.y = b.r;
      if (b.vy < 0) {
        hit(-b.vy);
        b.vy = -b.vy * bounce(b.vy);
      }
    } else if (b.y > this.height - b.r) {
      b.y = this.height - b.r;
      if (b.vy > 0) {
        hit(b.vy);
        b.vy = -b.vy * bounce(b.vy);
      }
    }
  }

  /** Sort and sweep along x: only discs whose spans overlap are compared. */
  private collide(impacts: Impact[] | null) {
    const order = this.bodies.slice().sort((a, b) => a.x - a.r - (b.x - b.r));
    for (let i = 0; i < order.length; i++) {
      const a = order[i];
      for (let j = i + 1; j < order.length; j++) {
        const b = order[j];
        if (b.x - b.r > a.x + a.r) break;
        const dx = b.x - a.x;
        const dy = b.y - a.y;
        const reach = a.r + b.r;
        const d2 = dx * dx + dy * dy;
        if (d2 >= reach * reach) continue;
        const d = Math.sqrt(d2) || 0.001;
        const nx = d2 === 0 ? 1 : dx / d;
        const ny = d2 === 0 ? 0 : dy / d;
        // Mass grows with area; a held coin is immovable.
        const ma = a.held ? Infinity : a.r * a.r;
        const mb = b.held ? Infinity : b.r * b.r;
        const ia = a.held ? 0 : 1 / ma;
        const ib = b.held ? 0 : 1 / mb;
        if (ia + ib === 0) continue;
        // Push apart along the line between them.
        const overlap = reach - d;
        const share = overlap / (ia + ib);
        a.x -= nx * share * ia;
        a.y -= ny * share * ia;
        b.x += nx * share * ib;
        b.y += ny * share * ib;
        // Bounce: only when closing in.
        const closing = (a.vx - b.vx) * nx + (a.vy - b.vy) * ny;
        if (closing <= 0) continue;
        const impulse = ((1 + bounce(closing)) * closing) / (ia + ib);
        a.vx -= impulse * ia * nx;
        a.vy -= impulse * ia * ny;
        b.vx += impulse * ib * nx;
        b.vy += impulse * ib * ny;
        if (impacts && closing > HEARD_FROM) {
          const small = a.r < b.r ? a : b;
          impacts.push({
            id: small.id,
            size: (a.r + b.r) / 2 / this.rMax,
            force: Math.min(1, (closing - HEARD_FROM) / 900),
          });
        }
      }
    }
  }

  /** The coin under a point, the topmost (last drawn) first. */
  at(x: number, y: number): Body | null {
    for (let i = this.bodies.length - 1; i >= 0; i--) {
      const b = this.bodies[i];
      if ((b.x - x) ** 2 + (b.y - y) ** 2 <= b.r * b.r) return b;
    }
    return null;
  }

  /** New size of the box: keep every coin inside. */
  resize(width: number, height: number) {
    this.width = width;
    this.height = height;
    for (const b of this.bodies) {
      b.x = Math.min(Math.max(b.x, b.r), width - b.r);
      b.y = Math.min(Math.max(b.y, b.r), height - b.r);
    }
    this.wake();
  }
}

function bounce(speed: number): number {
  return Math.abs(speed) < DEAD_BELOW ? 0 : RESTITUTION;
}
