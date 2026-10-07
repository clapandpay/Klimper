// SPDX-License-Identifier: AGPL-3.0-or-later
// The Klimper sound, running (clapandpay TASK-111.03, decision-009). Mounted once by
// `KlimperRoot`, only when `VITE_ENABLE_KLIMPER_SOUND` is on, and loaded lazily so a build
// without the flag ships none of this. Renders nothing.
//
// Two moments. Coins arrive: the balance Sage keeps goes up on a synced wallet, and the ding
// of the received value rings once (several coins landing together ring once, for their sum).
// Nothing worth a cent, or no price at all (testnet, offline): silence, on both moments.
// The phone is shaken: the native side does all of it, in real time; the webview only tells
// it the tier of what is on the page (a token page: that token; the wallet page: the total;
// anything else: the whole wallet), and again whenever that changes.
//
// Cost when on: one subscription to the wallet store (a few comparisons per update), one to
// the page value, and on the phone an accelerometer stream at 40 Hz while the app is active.
// The XCH price is asked for at most once in ten minutes, and only when something is about to
// play or the wallet's tier is sent. Cost when the switch is off: nothing, and the
// accelerometer is stopped.
//
// Privacy and safety (docs/sound.md): reads the balance already on screen, sends nothing but
// the price lookup Sage's own price display makes, touches no keys, no transactions, nothing in
// the send or receive path, and cannot block them: all asynchronous, all failing quietly.

import { commands } from '@/bindings';
import { pageValue, subscribePageValue } from '@/klimper/pageValue';
import { useWalletState } from '@/state';
import { useEffect } from 'react';
import { onPhone, playDing, setShake } from './player';
import { Coalescer, ReceiveDetector, sampleOf } from './receive';
import { useKlimperSoundSetting } from './setting';
import { Tier, tierOf, usdOf } from './tiers';

/** Increases closer than this are one ding. */
export const COALESCE_MS = 1500;
const PRICE_TTL_MS = 10 * 60_000;

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

type Sync = ReturnType<typeof useWalletState.getState>['sync'];

export function KlimperSound() {
  const [on] = useKlimperSoundSetting();

  // Coins arrive: watch the balance Sage keeps.
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
      if (alive && tier !== null) void playDing(tier);
    });
    const look = (sync: Sync) => {
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

  // The phone is shaken: arm the native side with the tier of what the page shows, and keep
  // it current. The page's own dollar value wins (Token.tsx, TokenList.tsx publish it); a
  // page without one shakes the whole wallet.
  useEffect(() => {
    // Off: nothing to do. The watch was never armed, or the previous run of this effect
    // disarmed it on its way out.
    if (!onPhone() || !on) return;
    let alive = true;
    let sent: Tier | null = null;
    const arm = async () => {
      const page = pageValue();
      let usd = page;
      if (page === null) {
        const { sync } = useWalletState.getState();
        if (sync.receive_address === 'Unknown') return;
        const { balance } = sampleOf(sync);
        usd = usdOf(balance, await xchPrice());
      }
      const tier = tierOf(usd);
      if (!alive || tier === sent) return;
      sent = tier;
      // Nothing worth a cent on the page, or no price: the shake is silent.
      void setShake(tier !== null, tier ?? 'coin');
    };
    void arm();
    const unsubscribeWallet = useWalletState.subscribe(() => void arm());
    const unsubscribePage = subscribePageValue(() => void arm());
    return () => {
      alive = false;
      unsubscribeWallet();
      unsubscribePage();
      if (sent !== null) void setShake(false, sent);
    };
  }, [on]);

  return null;
}

export default KlimperSound;
