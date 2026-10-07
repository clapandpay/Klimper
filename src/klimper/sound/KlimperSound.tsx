// SPDX-License-Identifier: AGPL-3.0-or-later
// The Klimper sound, running (clapandpay TASK-111.03, decision-009). Mounted once by
// `KlimperRoot`, only when `VITE_ENABLE_KLIMPER_SOUND` is on, and loaded lazily so a build
// without the flag ships none of this. Renders nothing.
//
// Cost when on: one subscription to the wallet store (a few comparisons per update), one
// event listener, and on the phone an accelerometer stream at 30 Hz while the app is active.
// The XCH price is asked for at most once in ten minutes, and only when something is about
// to play. Cost when the switch is off: nothing, and the accelerometer is stopped.

import { commands } from '@/bindings';
import { useWalletState } from '@/state';
import { listen } from '@tauri-apps/api/event';
import { useEffect } from 'react';
import { onPhone, playJingle, setShakeEnabled } from './player';
import { Coalescer, ReceiveDetector, sampleOf } from './receive';
import { useKlimperSoundSetting } from './setting';
import { tierOf, usdOf } from './tiers';

/** Increases closer than this are one jingle. */
export const COALESCE_MS = 1500;
const PRICE_TTL_MS = 10 * 60_000;
const SHAKE_EVENT = 'klimper-jingle://shake';

let price: { usd: number; at: number } | null = null;

/** Tests only: forget the cached price between cases. */
export function forgetPrice() {
  price = null;
}

/** The XCH price, cached; null when there is none (testnet, offline). */
async function xchPrice(): Promise<number | null> {
  if (price && Date.now() - price.at < PRICE_TTL_MS) return price.usd;
  try {
    const { usd } = await commands.getXchUsdPrice({});
    price = { usd, at: Date.now() };
    return usd;
  } catch {
    return null;
  }
}

async function currentFingerprint(): Promise<number | null> {
  try {
    const { key } = await commands.getKey({});
    return key?.fingerprint ?? null;
  } catch {
    return null;
  }
}

export function KlimperSound() {
  const [on] = useKlimperSoundSetting();

  // Receives: watch the balance Sage keeps.
  useEffect(() => {
    if (!on) return;
    let alive = true;
    const detector = new ReceiveDetector();
    let fingerprint: number | null = null;
    let fingerprintAsked = false;
    const coalescer = new Coalescer(COALESCE_MS, async (mojos) => {
      // The same wallet as the baseline? Switching wallets goes through Sage's default
      // state and resets the detector, but belt and braces: a fingerprint is cheap here.
      const now = await currentFingerprint();
      if (fingerprint !== null && now !== fingerprint) {
        fingerprint = now;
        detector.rebase(sampleOf(useWalletState.getState().sync));
        return;
      }
      fingerprint ??= now;
      const tier = tierOf(usdOf(mojos, await xchPrice()));
      if (alive) void playJingle(tier, false);
    });
    const look = (sync: ReturnType<typeof useWalletState.getState>['sync']) => {
      const outcome = detector.observe(sampleOf(sync));
      if (outcome.kind === 'reset') {
        fingerprint = null;
        fingerprintAsked = false;
        coalescer.cancel();
      } else if (outcome.kind === 'received') {
        coalescer.add(outcome.mojos);
      } else if (!fingerprintAsked && sync.receive_address !== 'Unknown') {
        fingerprintAsked = true;
        void currentFingerprint().then((f) => {
          fingerprint ??= f;
        });
      }
    };
    look(useWalletState.getState().sync);
    const unsubscribe = useWalletState.subscribe((state) => look(state.sync));
    return () => {
      alive = false;
      unsubscribe();
      coalescer.cancel();
    };
  }, [on]);

  // Shakes: the whole wallet, felt and heard. Phone only.
  useEffect(() => {
    // Off: nothing to do. The watch was never started, or the previous run of this
    // effect stopped it on its way out.
    if (!onPhone() || !on) return;
    let stopped = false;
    let unlisten: (() => void) | null = null;
    void listen(SHAKE_EVENT, async () => {
      const { sync } = useWalletState.getState();
      if (sync.receive_address === 'Unknown') return;
      const balance = sampleOf(sync).balance;
      const tier = tierOf(usdOf(balance, await xchPrice()));
      if (!stopped) void playJingle(tier, true);
    }).then((stop) => {
      if (stopped) stop();
      else unlisten = stop;
    });
    void setShakeEnabled(true);
    return () => {
      stopped = true;
      unlisten?.();
      void setShakeEnabled(false);
    };
  }, [on]);

  return null;
}

export default KlimperSound;
