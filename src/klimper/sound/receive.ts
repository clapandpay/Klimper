// SPDX-License-Identifier: AGPL-3.0-or-later
// When did coins arrive? (clapandpay TASK-111.03)
//
// Pure logic over the wallet state Sage already keeps (`useWalletState().sync`): no React,
// no Tauri, no timers of its own, so it is tested as a table. The hook feeds it every store
// update and acts on what comes back.
//
// Rules, each with a test:
// - The first real sample is the baseline and makes no sound: coins that came while the app
//   was closed are not announced, like a wallet that was filled while you were out.
// - Sage's default state (`receive_address === 'Unknown'`) means logged out or switching
//   wallets: the baseline is dropped and the next real sample starts over, silently.
// - A sample of a wallet that is catching up (`synced_coins < total_coins`) is ignored: a new
//   coin is unsynced until Sage has its puzzle, and the balance and the flag do not change in
//   the same store update, so the baseline is only ever a synced sample and the delta is taken
//   between synced samples. The launch catch-up is therefore silent (its first synced sample
//   is the baseline) and a receive is heard once its coin is synced.
// - A decrease is a spend, and silent.
// - An increase of a synced wallet is a receive; the caller confirms the wallet is still the
//   same one (`fingerprint`) before anything plays, because an address alone does not tell.

import type { GetSyncStatusResponse } from '@/bindings';

export interface Sample {
  address: string;
  balance: bigint;
  synced: boolean;
}

export function sampleOf(sync: GetSyncStatusResponse): Sample {
  return {
    address: sync.receive_address,
    balance: toMojos(sync.selectable_balance),
    synced: sync.synced_coins >= sync.total_coins,
  };
}

/** Sage gives the balance as a string or a number of mojos; anything odd counts as zero. */
export function toMojos(amount: string | number): bigint {
  try {
    if (typeof amount === 'number') {
      return Number.isFinite(amount) ? BigInt(Math.trunc(amount)) : 0n;
    }
    return /^\d+$/.test(amount) ? BigInt(amount) : 0n;
  } catch {
    return 0n;
  }
}

export type Outcome =
  | { kind: 'quiet' }
  /** The wallet changed or went away; the hook forgets its fingerprint. */
  | { kind: 'reset' }
  /** Coins arrived: this many mojos more than before. */
  | { kind: 'received'; mojos: bigint };

export const NO_WALLET = 'Unknown';

export class ReceiveDetector {
  private baseline: Sample | null = null;

  observe(sample: Sample): Outcome {
    if (sample.address === NO_WALLET) {
      const had = this.baseline !== null;
      this.baseline = null;
      return had ? { kind: 'reset' } : { kind: 'quiet' };
    }
    if (!sample.synced) return { kind: 'quiet' };
    const before = this.baseline;
    this.baseline = sample;
    if (before === null || sample.balance <= before.balance) {
      return { kind: 'quiet' };
    }
    return { kind: 'received', mojos: sample.balance - before.balance };
  }

  /** The caller found the wallet is another one: start over from this sample. */
  rebase(sample: Sample) {
    this.baseline = sample;
  }
}

/**
 * Several coins landing within a moment are one jingle, not several: increases within
 * `windowMs` of the first are summed and reported once, when the window closes. Timers are
 * injected so a test can drive time.
 */
export class Coalescer {
  private sum = 0n;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private readonly windowMs: number,
    private readonly flush: (mojos: bigint) => void,
    private readonly timers: {
      set: typeof setTimeout;
      clear: typeof clearTimeout;
    } = { set: setTimeout, clear: clearTimeout },
  ) {}

  add(mojos: bigint) {
    this.sum += mojos;
    if (this.timer !== null) return;
    this.timer = this.timers.set(() => {
      const total = this.sum;
      this.sum = 0n;
      this.timer = null;
      if (total > 0n) this.flush(total);
    }, this.windowMs);
  }

  cancel() {
    if (this.timer !== null) this.timers.clear(this.timer);
    this.timer = null;
    this.sum = 0n;
  }
}
