// SPDX-License-Identifier: AGPL-3.0-or-later
// One coin of the pouch, drawn once per size and state and then only copied (TASK-124): a
// milled rim, a raised ring, the asset's ticker struck into the face, a soft shadow below. A
// coin locked in an offer carries a lock; a coin already being spent is faded.
//
// Cost: drawing is all here, at most once per size, selection and state; a frame is only
// `drawImage` of cached sprites. Small coins skip the details nobody could see.

/** Room around a coin's sprite for the selection ring's glow and the shadow. */
export const PAD = 7;

export type CoinState = 'free' | 'offered' | 'pending';

/**
 * The ticker as struck on a coin: as the asset spells it, at most 4 characters (never half of
 * one). A token whose ticker would read as XCH (`xch`, `XCH-PEPE`, `TXCH fake`) gets a plain
 * coin: a ticker is the token's own claim, and a coin must not pass for Chia's.
 */
export function stamp(
  ticker: string | null | undefined,
  assetId: string | null,
): string {
  const chars = Array.from((ticker ?? '').trim());
  const shown = chars.slice(0, 4).join('');
  if (assetId !== null && /^t?xch/i.test(chars.join(''))) return '';
  return shown;
}

export function coinSprite(
  r: number,
  hue: number,
  ticker: string,
  selected: boolean,
  state: CoinState,
  dpr: number,
): HTMLCanvasElement {
  const half = r + PAD;
  const sprite = document.createElement('canvas');
  sprite.width = sprite.height = Math.ceil(half * 2 * dpr);
  const ctx = sprite.getContext('2d');
  if (!ctx) return sprite;
  ctx.scale(dpr, dpr);
  const c = half;
  ctx.globalAlpha = state === 'pending' ? 0.4 : 1;

  // Shadow on the pouch's floor, a little below and right of the coin.
  ctx.fillStyle = 'rgb(0 0 0 / 0.22)';
  ctx.beginPath();
  ctx.arc(c + r * 0.06, c + r * 0.1, r, 0, Math.PI * 2);
  ctx.fill();

  // The edge: a darker band, milled when the coin is big enough to show it.
  const edge = ctx.createLinearGradient(c - r, c - r, c + r, c + r);
  edge.addColorStop(0, `hsl(${hue} 45% 62%)`);
  edge.addColorStop(1, `hsl(${hue} 50% 24%)`);
  ctx.fillStyle = edge;
  ctx.beginPath();
  ctx.arc(c, c, r, 0, Math.PI * 2);
  ctx.fill();
  if (r >= 10) {
    const ticks = Math.round(r * 1.5);
    ctx.lineWidth = Math.max(0.6, r * 0.035);
    ctx.strokeStyle = `hsl(${hue} 40% 18% / 0.55)`;
    ctx.beginPath();
    for (let i = 0; i < ticks; i++) {
      const a = (i / ticks) * Math.PI * 2;
      const cos = Math.cos(a);
      const sin = Math.sin(a);
      ctx.moveTo(c + cos * r * 0.89, c + sin * r * 0.89);
      ctx.lineTo(c + cos * r * 0.99, c + sin * r * 0.99);
    }
    ctx.stroke();
  }

  // The face, lit from the top left.
  const face = r * 0.86;
  const light = ctx.createRadialGradient(
    c - face * 0.4,
    c - face * 0.45,
    face * 0.05,
    c,
    c,
    face,
  );
  light.addColorStop(0, `hsl(${hue} 75% 86%)`);
  light.addColorStop(0.55, `hsl(${hue} 55% 54%)`);
  light.addColorStop(1, `hsl(${hue} 55% 36%)`);
  ctx.fillStyle = light;
  ctx.beginPath();
  ctx.arc(c, c, face, 0, Math.PI * 2);
  ctx.fill();

  // A raised ring: dark below, light above, as if struck.
  if (r >= 8) {
    const ring = r * 0.7;
    ctx.lineWidth = Math.max(0.6, r * 0.04);
    ctx.strokeStyle = `hsl(${hue} 50% 22% / 0.5)`;
    ctx.beginPath();
    ctx.arc(c + 0.4, c + 0.6, ring, 0, Math.PI * 2);
    ctx.stroke();
    ctx.strokeStyle = `hsl(${hue} 80% 92% / 0.55)`;
    ctx.beginPath();
    ctx.arc(c, c, ring, 0, Math.PI * 2);
    ctx.stroke();
  }

  // The ticker, struck the same way, where it can be read; a small coin's lock takes its middle.
  const small = r < 11;
  if (ticker && !small) {
    let size = r * 0.5;
    ctx.font = font(size);
    const width = ctx.measureText(ticker).width;
    if (width > r * 1.2) {
      size *= (r * 1.2) / width;
      ctx.font = font(size);
    }
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    // Engraved: dark letters with a light edge below, readable on every hue.
    const y = c + size * 0.05;
    ctx.fillStyle = `hsl(${hue} 80% 92% / 0.6)`;
    ctx.fillText(ticker, c, y + 0.8);
    ctx.fillStyle = `hsl(${hue} 60% 16% / 0.85)`;
    ctx.fillText(ticker, c, y);
  }

  ctx.globalAlpha = 1;
  if (state === 'offered') {
    // A dark badge with a white lock, inside the coin (where a tap still finds it): at the
    // lower right of a big coin, over the whole face of a small one.
    const b = small ? r * 0.8 : r * 0.3;
    const at = small ? 0 : r * 0.5;
    ctx.fillStyle = 'rgb(20 20 24 / 0.85)';
    ctx.beginPath();
    ctx.arc(c + at, c + at, b, 0, Math.PI * 2);
    ctx.fill();
    ctx.lineWidth = 1;
    ctx.strokeStyle = 'rgb(255 255 255 / 0.7)';
    ctx.stroke();
    lock(ctx, c + at, c + at, b * 1.15);
  }

  if (selected) {
    ctx.lineWidth = 2.5;
    ctx.strokeStyle = 'white';
    ctx.shadowColor = `hsl(${hue} 90% 60%)`;
    ctx.shadowBlur = 6;
    ctx.beginPath();
    ctx.arc(c, c, r + 1, 0, Math.PI * 2);
    ctx.stroke();
  }
  return sprite;
}

function font(px: number) {
  return `700 ${px.toFixed(1)}px ui-rounded, -apple-system, system-ui, sans-serif`;
}

/** A padlock `s` tall, centred on (x, y), in white. */
function lock(ctx: CanvasRenderingContext2D, x: number, y: number, s: number) {
  const w = s * 0.62;
  const h = s * 0.42;
  const top = y - s * 0.1;
  ctx.fillStyle = 'white';
  ctx.fillRect(x - w / 2, top, w, h);
  ctx.lineWidth = Math.max(0.8, s * 0.1);
  ctx.strokeStyle = 'white';
  // The shackle: a half ring standing on the body.
  ctx.beginPath();
  ctx.arc(x, top, w * 0.32, Math.PI, 0);
  ctx.stroke();
}
