// SPDX-License-Identifier: AGPL-3.0-or-later
// The coins in the wallet (TASK-124): an asset's coins as discs in a pouch, lying, rolling and
// knocking as the phone tilts. Tapping coins selects them; the selection is the coin list's
// own, so Split and Combine below work on it exactly as on the list. It shows Chia's coin set
// model as it is: a balance is a handful of coins, a payment spends whole coins and gives change,
// dust is many tiny ones.
//
// Cost: nothing while the coins rest. The animation frame runs only while something moves, the
// tilt stream only while the pouch is on screen and the app in front, and gravity arrives only
// when the phone turned (`openPouch`). Knocks are heard and felt through the coin sound's own
// pieces (`clink`), at most one per CLINK_MS, and only with the coin sound on.

import { CoinRecord, commands, events, TokenRecord } from '@/bindings';
import { NumberFormat } from '@/components/NumberFormat';
import { useErrors } from '@/hooks/useErrors';
import { fromMojos } from '@/lib/utils';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { RowSelectionState } from '@tanstack/react-table';
import BigNumber from 'bignumber.js';
import {
  Dispatch,
  SetStateAction,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { usePrivacy } from '../privacy';
import { clink, openPouch } from '../sound/player';
import { useKlimperSoundSetting } from '../sound/setting';
import { Hint } from '../Hint';
import { Birth, change, Ghost, GONE_MS, grown, progress } from './changes';
import { assetHue, layoutCoins } from './layout';
import { Body, STEP, World } from './physics';

const HEIGHT = 200;
const CLINK_MS = 45;
/** How long spent coins wait for what they became before they are shown leaving. */
const WAIT_MS = 1500;
/** A pointer that moved less than this was a tap. */
const TAP_PX = 6;
/** More than Sage's list ever pages through at once; beyond MAX_DISCS it is dust anyway. */
const FETCH_LIMIT = 1000;

export interface CoinPouchProps {
  asset: TokenRecord;
  selectedCoins: RowSelectionState;
  setSelectedCoins: Dispatch<SetStateAction<RowSelectionState>>;
}

export function CoinPouch({
  asset,
  selectedCoins,
  setSelectedCoins,
}: CoinPouchProps) {
  const { addError } = useErrors();
  const [privacy] = usePrivacy();
  const [soundOn] = useKlimperSoundSetting();
  const audible = soundOn && !privacy;

  const [coins, setCoins] = useState<CoinRecord[]>([]);
  const [width, setWidth] = useState(0);
  const box = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const world = useRef(new World(1, HEIGHT));
  const locked = useRef(new Set<string>());
  const selected = useRef(selectedCoins);
  selected.current = selectedCoins;
  const audibleRef = useRef(audible);
  audibleRef.current = audible;
  const wake = useRef<() => void>(() => undefined);
  // Coins on their way out and on their way in (`changes.ts`), and where spent coins meet.
  const ghosts = useRef<Ghost[]>([]);
  const births = useRef<Birth[]>([]);
  const meet = useRef<{ at: number; size: number } | null>(null);
  const waited = useRef(false);

  // Another asset: an empty pouch that fills without animations.
  useEffect(() => {
    world.current.set([]);
    ghosts.current = [];
    births.current = [];
    meet.current = null;
  }, [asset.asset_id]);

  // The coins: all owned ones, largest first; again whenever the wallet's coins change.
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = () =>
      commands
        .getCoins({
          asset_id: asset.asset_id,
          offset: 0,
          limit: FETCH_LIMIT,
          sort_mode: 'amount',
          ascending: false,
          filter_mode: 'owned',
        })
        .then((res) => alive && setCoins(res.coins))
        .catch(addError);
    void load();
    const unlisten = events.syncEvent.listen((event) => {
      const type = event.payload.type;
      if (type !== 'coin_state' && type !== 'puzzle_batch_synced') return;
      clearTimeout(timer);
      timer = setTimeout(load, 400); // a burst of updates is one reload
    });
    return () => {
      alive = false;
      clearTimeout(timer);
      void unlisten.then((u) => u());
    };
  }, [asset.asset_id, addError]);

  // The box's width follows the card.
  useEffect(() => {
    const element = box.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) =>
      setWidth(Math.round(entry.contentRect.width)),
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const layout = useMemo(
    () => (width > 0 ? layoutCoins(coins, width, HEIGHT, privacy) : null),
    [coins, width, privacy],
  );

  // Coins into the world: the ones already there keep their place; spent ones pull together
  // or leave, new ones come out where they met, or drop in (`changes.ts`).
  useEffect(() => {
    if (!layout) return;
    const w = world.current;
    w.resize(width, HEIGHT);
    locked.current = new Set(
      layout.discs.filter((d) => d.locked).map((d) => d.id),
    );
    // Coins still waiting to appear count as there.
    const now = performance.now();
    const before = [...w.bodies, ...births.current.map((b) => b.body)];
    // A pending transaction's coins went, nothing came yet: what they became may come with
    // the next update. Wait for it a moment, so a combine is not shown as a send.
    const ids = new Set(layout.discs.map((d) => d.id));
    const had = new Set(before.map((b) => b.id));
    const leaving = before.filter((b) => !ids.has(b.id));
    const arriving = layout.discs.some((d) => !had.has(d.id));
    if (leaving.length > 0 && !arriving && leaving.every((b) => b.group)) {
      if (!waited.current) {
        waited.current = true;
        const timer = setTimeout(() => setCoins((c) => [...c]), WAIT_MS);
        return () => clearTimeout(timer);
      }
    }
    waited.current = false;
    const next = change(before, layout.discs, width, HEIGHT, now);
    w.set(next.bodies);
    ghosts.current = [...ghosts.current, ...next.ghosts];
    births.current = next.births;
    if (next.meet) meet.current = next.meet;
    wake.current();
  }, [layout, width]);

  const hue = assetHue(asset.asset_id);

  // Drawing and the loop.
  useEffect(() => {
    const element = canvas.current;
    if (!element || width === 0) return;
    const dpr = window.devicePixelRatio || 1;
    element.width = Math.round(width * dpr);
    element.height = Math.round(HEIGHT * dpr);
    const ctx = element.getContext('2d');
    if (!ctx) return;
    ctx.scale(dpr, dpr);
    const sprites = new Map<string, HTMLCanvasElement>();

    const spriteOf = (r: number, sel: boolean, lock: boolean) => {
      const key = `${Math.round(r * 2)}|${sel}|${lock}`;
      let sprite = sprites.get(key);
      if (!sprite) {
        sprite = coinSprite(Math.round(r * 2) / 2, hue, sel, lock, dpr);
        sprites.set(key, sprite);
      }
      return sprite;
    };
    /** A coin's sprite at (x, y), `scale` of its size. */
    const coin = (
      sprite: HTMLCanvasElement,
      x: number,
      y: number,
      r: number,
      scale: number,
    ) => {
      const half = (r + PAD) * scale;
      ctx.drawImage(sprite, x - half, y - half, half * 2, half * 2);
    };

    const draw = (now: number) => {
      ctx.clearRect(0, 0, width, HEIGHT);
      // Spent coins: pulled into their meeting point (or out of the pouch), shrinking, fading.
      ghosts.current = ghosts.current.filter((g) => now < g.start + GONE_MS);
      for (const g of ghosts.current) {
        const t = progress(g.start, GONE_MS, now);
        ctx.globalAlpha = 1 - t * t;
        coin(
          spriteOf(g.r, false, true),
          g.x + (g.tx - g.x) * t,
          g.y + (g.ty - g.y) * t,
          g.r,
          1 - 0.45 * t,
        );
      }
      ctx.globalAlpha = 1;
      for (const b of world.current.bodies) {
        const scale = grown(b.born, now);
        if (scale <= 0) continue;
        const sprite = spriteOf(
          b.r,
          !!selected.current[b.id],
          locked.current.has(b.id),
        );
        coin(sprite, b.x, b.y, b.r, scale);
      }
    };

    /** Something is still playing that the world's sleep does not see. */
    const animating = (now: number) =>
      ghosts.current.length > 0 ||
      births.current.length > 0 ||
      world.current.bodies.some((b) => grown(b.born, now) < 1);

    let frame = 0;
    let last = 0;
    let carry = 0;
    let lastClink = 0;
    let onScreen = false;
    const tick = (now: number) => {
      frame = 0;
      const w = world.current;
      carry = Math.min(carry + (last ? (now - last) / 1000 : STEP), STEP * 4);
      last = now;
      // New coins whose moment came join the world.
      const due = births.current.filter((b) => b.at <= now);
      if (due.length > 0) {
        births.current = births.current.filter((b) => b.at > now);
        for (const b of due) w.add(b.body);
      }
      // The spent coins meet: one knock, the size of what they became.
      const met = meet.current;
      if (met && met.at <= now) {
        meet.current = null;
        if (audibleRef.current) clink(met.size, 0.8);
      }
      while (carry >= STEP) {
        carry -= STEP;
        const impacts = w.step();
        const strongest = impacts[0];
        if (strongest && audibleRef.current && now - lastClink >= CLINK_MS) {
          lastClink = now;
          clink(strongest.size, strongest.force);
        }
      }
      draw(now);
      // At rest: no frame until something wakes the coins.
      if ((!w.asleep || animating(now)) && onScreen) {
        frame = requestAnimationFrame(tick);
      }
    };
    wake.current = () => {
      world.current.wake();
      if (frame === 0 && onScreen && document.visibilityState === 'visible') {
        last = 0;
        frame = requestAnimationFrame(tick);
      }
    };

    // Tilt only while the pouch is seen and the app in front.
    let close: (() => void) | null = null;
    const follow = () => {
      const want = onScreen && document.visibilityState === 'visible';
      if (want && !close) {
        close = openPouch((m) => {
          // Phone axes (y up) to the screen's (y down).
          if ('gx' in m) world.current.tilt(m.gx, -m.gy);
          else world.current.jolt(m.jx, -m.jy);
          wake.current();
        });
      } else if (!want && close) {
        close();
        close = null;
      }
      if (want) wake.current();
    };
    const observer = new IntersectionObserver(([entry]) => {
      onScreen = entry.isIntersecting;
      follow();
    });
    observer.observe(element);
    document.addEventListener('visibilitychange', follow);
    draw(performance.now());

    return () => {
      observer.disconnect();
      document.removeEventListener('visibilitychange', follow);
      close?.();
      cancelAnimationFrame(frame);
      wake.current = () => undefined;
    };
  }, [width, hue]);

  // A selection made in the list shows on the coins, even at rest.
  useEffect(() => {
    wake.current();
  }, [selectedCoins]);

  // Tap to select; drag to throw a coin.
  const pointer = useRef<{
    body: Body;
    x: number;
    y: number;
    t: number;
    moved: boolean;
    vx: number;
    vy: number;
  } | null>(null);
  const local = (event: React.PointerEvent) => {
    const rect = event.currentTarget.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };
  const onDown = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const { x, y } = local(event);
    const body = world.current.at(x, y);
    if (!body) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    pointer.current = {
      body,
      x,
      y,
      t: event.timeStamp,
      moved: false,
      vx: 0,
      vy: 0,
    };
  };
  const onMove = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const p = pointer.current;
    if (!p) return;
    const { x, y } = local(event);
    if (!p.moved && Math.hypot(x - p.x, y - p.y) < TAP_PX) return;
    p.moved = true;
    p.body.held = true;
    const dt = Math.max(1, event.timeStamp - p.t) / 1000;
    p.vx = (x - p.body.x) / dt;
    p.vy = (y - p.body.y) / dt;
    p.body.x = x;
    p.body.y = y;
    p.t = event.timeStamp;
    wake.current();
  };
  const toggle = useCallback(
    (id: string) => {
      if (locked.current.has(id)) return;
      setSelectedCoins((current) =>
        current[id]
          ? Object.fromEntries(
              Object.entries(current).filter(([key]) => key !== id),
            )
          : { ...current, [id]: true },
      );
    },
    [setSelectedCoins],
  );
  const onUp = () => {
    const p = pointer.current;
    pointer.current = null;
    if (!p) return;
    if (!p.moved) {
      toggle(p.body.id);
      return;
    }
    p.body.held = false;
    p.body.vx = Math.max(-1500, Math.min(1500, p.vx));
    p.body.vy = Math.max(-1500, Math.min(1500, p.vy));
    wake.current();
  };

  const picked = coins.filter((c) => selectedCoins[c.coin_id]);
  const pickedSum = picked.reduce(
    (sum, c) => sum.plus(c.amount),
    new BigNumber(0),
  );
  const dust = layout?.dust ?? [];
  const ticker = asset.ticker ?? '';
  const pickedCount = picked.length;
  const coinCount = coins.length;
  const dustCount = dust.length;

  return (
    <div className='mb-4'>
      <div
        ref={box}
        className='klimper-pouch relative overflow-hidden rounded-2xl'
        style={{ height: HEIGHT }}
      >
        <canvas
          ref={canvas}
          className='block h-full w-full touch-none'
          aria-label={t`Your coins`}
          role='img'
          onPointerDown={onDown}
          onPointerMove={onMove}
          onPointerUp={onUp}
          onPointerCancel={onUp}
        />
        {coins.length === 0 && (
          <p className='absolute inset-0 flex items-center justify-center text-sm text-muted-foreground'>
            <Trans>No coins yet</Trans>
          </p>
        )}
      </div>
      <div className='mt-2 flex min-h-6 items-center gap-2 text-sm text-muted-foreground'>
        {picked.length > 0 ? (
          <span>
            <Trans>{pickedCount} selected</Trans> ·{' '}
            <NumberFormat
              value={fromMojos(pickedSum, asset.precision)}
              maximumFractionDigits={asset.precision}
            />{' '}
            {ticker}
          </span>
        ) : (
          <span>
            <Trans>{coinCount} coins</Trans>
          </span>
        )}
        <Hint label={t`About your coins`}>
          <Trans>
            Each coin is one disc: the bigger, the more it holds. A payment
            spends whole coins and gives change back as a new coin. Tilt the
            phone to move them, tap coins to select them, then split or combine
            them below.
          </Trans>
        </Hint>
        {dust.length > 0 && (
          <button
            type='button'
            className='ml-auto underline underline-offset-4'
            onClick={() =>
              setSelectedCoins((current) => {
                const next = { ...current };
                for (const id of dust) next[id] = true;
                return next;
              })
            }
          >
            <Trans>Select {dustCount} tiny coins</Trans>
          </button>
        )}
      </div>
    </div>
  );
}

/** Room around a coin's sprite for the selection ring. */
const PAD = 3;

/** One coin, drawn once per size and state: a metal disc lit from the top left, rim and edge. */
function coinSprite(
  r: number,
  hue: number,
  selected: boolean,
  locked: boolean,
  dpr: number,
): HTMLCanvasElement {
  const half = r + PAD;
  const sprite = document.createElement('canvas');
  sprite.width = sprite.height = Math.ceil(half * 2 * dpr);
  const ctx = sprite.getContext('2d');
  if (!ctx) return sprite;
  ctx.scale(dpr, dpr);
  ctx.globalAlpha = locked ? 0.4 : 1;
  const face = ctx.createRadialGradient(
    half - r * 0.35,
    half - r * 0.4,
    r * 0.05,
    half,
    half,
    r,
  );
  face.addColorStop(0, `hsl(${hue} 70% 82%)`);
  face.addColorStop(0.5, `hsl(${hue} 55% 52%)`);
  face.addColorStop(1, `hsl(${hue} 60% 28%)`);
  ctx.fillStyle = face;
  ctx.beginPath();
  ctx.arc(half, half, r, 0, Math.PI * 2);
  ctx.fill();
  // The rim, and the raised edge just inside it.
  ctx.lineWidth = Math.max(1, r * 0.08);
  ctx.strokeStyle = `hsl(${hue} 50% 22% / 0.8)`;
  ctx.beginPath();
  ctx.arc(half, half, r * 0.95, 0, Math.PI * 2);
  ctx.stroke();
  if (r > 6) {
    ctx.lineWidth = Math.max(0.75, r * 0.04);
    ctx.strokeStyle = `hsl(${hue} 70% 88% / 0.4)`;
    ctx.beginPath();
    ctx.arc(half, half, r * 0.72, 0, Math.PI * 2);
    ctx.stroke();
  }
  if (selected) {
    ctx.globalAlpha = 1;
    ctx.lineWidth = 2.5;
    ctx.strokeStyle = 'white';
    ctx.shadowColor = `hsl(${hue} 90% 60%)`;
    ctx.shadowBlur = 6;
    ctx.beginPath();
    ctx.arc(half, half, r + 1, 0, Math.PI * 2);
    ctx.stroke();
  }
  return sprite;
}

export default CoinPouch;
