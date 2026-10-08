// SPDX-License-Identifier: AGPL-3.0-or-later
// The two calls into the native side (clapandpay TASK-111.03): ring the ding of a tier once
// (coins arrived), and arm or disarm the shake with the page's tier (the native side turns every
// turn of the hand into a few coins, heard and felt). Calls go out one after the other, so a quick
// off-and-on cannot arrive in the wrong order. On the desktop the ding
// goes through the web audio API, decoded once per tier; a desk cannot be shaken.
//
// Everything here fails quietly: a sound that did not play is not an error the user needs.

import { Channel, invoke } from '@tauri-apps/api/core';
import { onPhone } from '../haptics';
import type { Tier } from './tiers';

// One answer for the whole app (`haptics.ts` asks it too); re-exported for the sound's callers.
export { onPhone };

let warned = false;
function quiet(error: unknown) {
  // Once per run is enough to notice in a log; never a toast.
  if (warned) return;
  warned = true;
  console.warn('klimper sound:', error);
}

let queue: Promise<unknown> = Promise.resolve();
function inOrder(call: () => Promise<unknown>): Promise<void> {
  const next = queue.then(call).catch(quiet);
  queue = next;
  return next.then(() => undefined);
}

/** Coins arrived: ring the tier's ding once. */
export async function playDing(tier: Tier): Promise<void> {
  if (!onPhone()) return playOnDesktop(tier).catch(quiet);
  return inOrder(() => invoke('plugin:klimper-jingle|play', { tier }));
}

/**
 * Arm the shake with the wallet's tier, or disarm it. Phone only; on the desktop this does
 * nothing. Call again when the tier changes so a shake answers with the right weight.
 */
export async function setShake(enabled: boolean, tier: Tier): Promise<void> {
  if (!onPhone()) return;
  return inOrder(() =>
    invoke('plugin:klimper-jingle|set_shake', { enabled, tier }),
  );
}

/** What the phone tells an open coin pouch: gravity, or a jolt of the hand (in g, phone axes). */
export type PouchMotion =
  | { gx: number; gy: number }
  | { jx: number; jy: number };

/**
 * Open the coin pouch (TASK-124): the phone's tilt and jolts come to `onMotion`, and the shake
 * and tilt stop sounding on their own, since the pouch's coins knock instead ([`clink`]).
 * Returns the close. Phone only; elsewhere the pouch has no motion and this does nothing.
 */
export function openPouch(onMotion: (motion: PouchMotion) => void): () => void {
  if (!onPhone()) return () => undefined;
  // The phone has one motion stream: the first pouch opens it, the last one closes it, and
  // every open pouch hears it.
  listeners.add(onMotion);
  if (listeners.size === 1) {
    const channel = new Channel<PouchMotion>();
    channel.onmessage = (motion) =>
      listeners.forEach((listen) => listen(motion));
    void inOrder(() =>
      invoke('plugin:klimper-jingle|pouch', { open: true, channel }),
    );
  }
  let open = true;
  return () => {
    if (!open) return;
    open = false;
    listeners.delete(onMotion);
    if (listeners.size > 0) return;
    void inOrder(() =>
      invoke('plugin:klimper-jingle|pouch', {
        open: false,
        channel: new Channel(),
      }),
    );
  };
}

const listeners = new Set<(motion: PouchMotion) => void>();

/** A knock between coins in an open pouch, heard and felt. `size` and `force` 0..1. */
export function clink(size: number, force: number): void {
  if (!onPhone()) return;
  // Not queued: a knock is now or never, and order between knocks does not matter.
  void invoke('plugin:klimper-jingle|clink', { size, force }).catch(quiet);
}

// Desktop: one AudioContext, buffers decoded on first use and kept.
let context: AudioContext | null = null;
const buffers = new Map<Tier, Promise<AudioBuffer>>();

async function playOnDesktop(tier: Tier) {
  if (typeof AudioContext === 'undefined') return;
  const audio = (context ??= new AudioContext());
  if (audio.state === 'suspended') await audio.resume();
  let buffer = buffers.get(tier);
  if (!buffer) {
    // Vite resolves the four files from this template at build time (docs/sound.md).
    buffer = fetch(new URL(`./klimper-ding-${tier}.wav`, import.meta.url))
      .then((r) => r.arrayBuffer())
      .then((bytes) => audio.decodeAudioData(bytes));
    buffers.set(tier, buffer);
    buffer.catch(() => buffers.delete(tier));
  }
  const source = audio.createBufferSource();
  source.buffer = await buffer;
  source.connect(audio.destination);
  source.start();
}
