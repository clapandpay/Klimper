// SPDX-License-Identifier: AGPL-3.0-or-later
// Playing a tier (clapandpay TASK-111.03): on the phone through the jingle plugin (native
// playback under the ambient session, haptics, nothing in the background); on the desktop
// through the web audio API, decoded once per tier. The files are the same four WAVs.
//
// Everything here fails quietly: a sound that did not play is not an error the user needs.

import { invoke } from '@tauri-apps/api/core';
import { platform } from '@tauri-apps/plugin-os';
import { HAPTICS, type Tier } from './tiers';

export function onPhone(): boolean {
  try {
    const os = platform();
    return os === 'ios' || os === 'android';
  } catch {
    return false;
  }
}

/** Play a tier. `haptics` false plays the sound alone (a receive is heard, a shake is felt). */
export async function playJingle(tier: Tier, haptics: boolean): Promise<void> {
  try {
    if (onPhone()) {
      await invoke('plugin:klimper-jingle|play', {
        tier,
        impacts: haptics ? HAPTICS[tier] : [],
      });
    } else {
      await playOnDesktop(tier);
    }
  } catch (error) {
    // Once per run is enough to notice in a log; never a toast.
    if (!warned) {
      warned = true;
      console.warn('klimper sound:', error);
    }
  }
}

let warned = false;

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
    buffer = fetch(new URL(`./klimper-${tier}.wav`, import.meta.url))
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

/** Watch the accelerometer (phone only). On the desktop this does nothing. */
export async function setShakeEnabled(enabled: boolean): Promise<void> {
  if (!onPhone()) return;
  try {
    await invoke('plugin:klimper-jingle|set_shake_enabled', { enabled });
  } catch (error) {
    if (!warned) {
      warned = true;
      console.warn('klimper sound:', error);
    }
  }
}
