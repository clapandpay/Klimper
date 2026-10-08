// SPDX-License-Identifier: AGPL-3.0-or-later
// The trust boundary, tested as a boundary (clapandpay TASK-080).
//
// `readPrompt` is the only reader of a wallet-kit prompt, so these cases are what the
// approval screen can and cannot be made to say.

import { describe, expect, it } from 'vitest';
import {
  claimedText,
  formatAmount,
  needsSecondConfirmation,
  readPrompt,
  sanitiseMessageText,
} from './xchonnectPrompt';

const TAIL = 'a'.repeat(64);
const KEY = 'b'.repeat(96);
const PUZZLE = 'c'.repeat(64);

/** A plain send of 1.5 XCH with a fee, as wallet-kit serialises it. */
const simpleSend = {
  type: 'sign_coin_spends',
  dapp: 'pengui.xyz',
  summary: {
    assets: [
      {
        asset: { type: 'xch' },
        sent: 1_500_000_000_000,
        received: 0,
        conditional_received: 0,
        net: -1_500_000_000_000,
      },
    ],
    implied_fee: 500_000,
    reserve_fee: 0,
    time_locks: {
      not_before_seconds: null,
      not_before_height: null,
      expires_seconds: null,
      expires_height: null,
      has_relative: false,
    },
    unknown_puzzles: [],
    cost: 1,
    spends: [],
  },
  plan: {
    ours: [
      {
        spend: 0,
        opcode: 50,
        public_key: KEY,
        message: 'ff',
        is_unsafe: false,
      },
    ],
    foreign: [],
    has_unsafe: false,
  },
  partial: false,
  binding: null,
};

/** A swap: XCH leaves for certain, a CAT arrives only if the other side completes. */
const swap = {
  ...simpleSend,
  summary: {
    ...simpleSend.summary,
    assets: [
      {
        asset: { type: 'xch' },
        sent: 1_000_000_000_000,
        received: 0,
        conditional_received: 0,
        net: -1_000_000_000_000,
      },
      {
        asset: { type: 'cat', asset_id: TAIL },
        sent: 0,
        received: 0,
        conditional_received: 250_000,
        net: 0,
      },
    ],
  },
};

describe('readPrompt', () => {
  it('reads a simple send', () => {
    const view = readPrompt(simpleSend);
    expect(view.kind).toBe('spend');
    if (view.kind !== 'spend') return;
    expect(view.assets).toHaveLength(1);
    expect(view.assets[0].label).toBe('XCH');
    expect(formatAmount(view.assets[0].net, view.assets[0].precision)).toBe(
      '-1.5',
    );
    expect(formatAmount(view.impliedFee ?? view.reserveFee, 12)).toBe(
      '0.0000005',
    );
    expect(view.hasUnsafe).toBe(false);
    expect(view.signatures).toHaveLength(1);
    expect(needsSecondConfirmation(view)).toBe(false);
  });

  it('keeps a conditional receipt out of the net effect', () => {
    // The rule this whole module exists for: a receipt that depends on somebody else is
    // not income, and must never be folded into a figure the user reads as one.
    const view = readPrompt(swap);
    if (view.kind !== 'spend') throw new Error('expected a spend');
    const cat = view.assets[1];
    expect(cat.label).toBe(`CAT ${TAIL.slice(0, 8)}…${TAIL.slice(-6)}`);
    expect(cat.net.toString()).toBe('0');
    expect(cat.conditionalReceived.toString()).toBe('250000');
    // And it is not quietly added anywhere else either.
    const total = view.assets.reduce(
      (sum, asset) => sum.plus(asset.net),
      view.assets[0].net.minus(view.assets[0].net),
    );
    expect(total.toString()).toBe('-1000000000000');
  });

  it('reads a partial sign with its bound payments', () => {
    const view = readPrompt({
      ...swap,
      partial: true,
      binding: {
        all_bound: true,
        unbound_user_spends: [],
        bound_payments: [
          {
            user_spend: 0,
            settlement_spend: 1,
            asset: { type: 'cat', asset_id: TAIL },
            amount: 250_000,
          },
        ],
        bound_received: [],
      },
    });
    if (view.kind !== 'spend') throw new Error('expected a spend');
    expect(view.partial).toBe(true);
    expect(view.binding?.allBound).toBe(true);
    expect(view.binding?.payments).toHaveLength(1);
    expect(view.binding?.payments[0].amount.toString()).toBe('250000');
  });

  it('reads an unknown contract and asks for a second confirmation', () => {
    const view = readPrompt({
      ...simpleSend,
      summary: { ...simpleSend.summary, unknown_puzzles: [PUZZLE] },
    });
    if (view.kind !== 'spend') throw new Error('expected a spend');
    expect(view.unknownPuzzles).toEqual([PUZZLE]);
    expect(needsSecondConfirmation(view)).toBe(true);
  });

  it('flags an unsafe signature from either place it is written', () => {
    const fromFlag = readPrompt({
      ...simpleSend,
      plan: { ...simpleSend.plan, has_unsafe: true },
    });
    const fromRequirement = readPrompt({
      ...simpleSend,
      plan: {
        ...simpleSend.plan,
        has_unsafe: false,
        ours: [
          {
            spend: 0,
            opcode: 49,
            public_key: KEY,
            message: 'ff',
            is_unsafe: true,
          },
        ],
      },
    });
    expect(fromFlag.kind === 'spend' && fromFlag.hasUnsafe).toBe(true);
    expect(fromRequirement.kind === 'spend' && fromRequirement.hasUnsafe).toBe(
      true,
    );
  });

  it('reads a signMessage prompt', () => {
    const view = readPrompt({
      type: 'sign_message',
      dapp: 'pengui.xyz',
      message_hex: '48656c6c6f',
      message_text: 'Hello',
      public_key: KEY,
    });
    expect(view).toMatchObject({
      kind: 'message',
      messageText: 'Hello',
      messageHex: '48656c6c6f',
      publicKey: KEY,
    });
  });

  it('refuses a prompt it cannot describe', () => {
    expect(readPrompt(null).kind).toBe('unreadable');
    expect(readPrompt({ type: 'take_everything' }).kind).toBe('unreadable');
    expect(readPrompt({ type: 'sign_message', message_hex: 'zz' }).kind).toBe(
      'unreadable',
    );
  });

  describe('a string from the dApp cannot reach the screen', () => {
    it('ignores every extra key, whatever it is called', () => {
      const view = readPrompt({
        ...simpleSend,
        label: 'Harmless top-up',
        description: 'You receive 10 XCH',
        amount: '10 XCH',
        recipient: 'xch1friend',
        summary: {
          ...simpleSend.summary,
          label: 'Harmless top-up',
          total: '+10 XCH',
          assets: [
            {
              ...simpleSend.summary.assets[0],
              label: 'A gift',
              name: 'Totally Safe Coin',
              memo: 'approved by your bank',
            },
          ],
        },
      });
      // Only the fields the rendering table lists came through, and the serialised view
      // holds none of the invented ones.
      expect(JSON.stringify(view)).not.toMatch(
        /Harmless top-up|Totally Safe Coin|A gift|approved by your bank|xch1friend/,
      );
      if (view.kind !== 'spend') throw new Error('expected a spend');
      expect(view.assets[0].label).toBe('XCH');
    });

    it('refuses an asset type it does not know rather than calling it XCH', () => {
      const view = readPrompt({
        ...simpleSend,
        summary: {
          ...simpleSend.summary,
          assets: [
            {
              asset: { type: 'nft', launcher_id: TAIL },
              sent: 1,
              received: 0,
              conditional_received: 0,
              net: -1,
            },
          ],
        },
      });
      expect(view.kind).toBe('unreadable');
    });

    it('derives an asset name instead of reading one', () => {
      const view = readPrompt({
        ...simpleSend,
        summary: {
          ...simpleSend.summary,
          assets: [
            {
              asset: { type: 'cat', asset_id: TAIL, name: 'US Dollars' },
              sent: 0,
              received: 1,
              conditional_received: 0,
              net: 1,
            },
          ],
        },
      });
      if (view.kind !== 'spend') throw new Error('expected a spend');
      expect(view.assets[0].label).toBe(
        `CAT ${TAIL.slice(0, 8)}…${TAIL.slice(-6)}`,
      );
    });

    it('drops a puzzle hash that is prose rather than hex', () => {
      const view = readPrompt({
        ...simpleSend,
        summary: {
          ...simpleSend.summary,
          unknown_puzzles: ['the official Pengui vault', PUZZLE],
        },
      });
      if (view.kind !== 'spend') throw new Error('expected a spend');
      expect(view.unknownPuzzles).toEqual([PUZZLE]);
    });

    it('drops a public key that is not a key', () => {
      const view = readPrompt({
        ...simpleSend,
        plan: {
          ...simpleSend.plan,
          ours: [
            {
              spend: 0,
              opcode: 50,
              public_key: 'your own savings key',
              message: 'ff',
              is_unsafe: false,
            },
          ],
        },
      });
      if (view.kind !== 'spend') throw new Error('expected a spend');
      expect(view.signatures[0].publicKey).toBe('');
    });

    it('never treats a text amount as a number', () => {
      const view = readPrompt({
        ...simpleSend,
        summary: {
          ...simpleSend.summary,
          assets: [
            {
              asset: { type: 'xch' },
              sent: 0,
              received: 0,
              conditional_received: 0,
              net: '+1000 XCH, trust me',
            },
          ],
        },
      });
      // Never shown as some other figure: the request cannot be described, so it cannot
      // be approved.
      expect(view.kind).toBe('unreadable');
    });

    it('refuses amounts that lost precision or are not whole', () => {
      for (const net of [2 ** 53 + 2, 1.5, Number.NaN]) {
        const view = readPrompt({
          ...simpleSend,
          summary: {
            ...simpleSend.summary,
            assets: [
              {
                asset: { type: 'xch' },
                sent: 0,
                received: 0,
                conditional_received: 0,
                net,
              },
            ],
          },
        });
        expect(view.kind, String(net)).toBe('unreadable');
      }
    });

    it('reads large amounts given as decimal strings exactly', () => {
      const view = readPrompt({
        ...simpleSend,
        summary: {
          ...simpleSend.summary,
          assets: [
            {
              asset: { type: 'xch' },
              sent: '9007199254740993',
              received: 0,
              conditional_received: 0,
              net: '-9007199254740993',
            },
          ],
        },
      });
      if (view.kind !== 'spend') throw new Error('expected a spend');
      expect(view.assets[0].net.toFixed()).toBe('-9007199254740993');
    });
  });
});

describe('sanitiseMessageText', () => {
  it('neutralises the characters that make text lie about itself', () => {
    const nasty = `pay\u202Eevil\u200Bnow\u0007`;
    const { text } = sanitiseMessageText(nasty);
    for (const character of ['\u202E', '\u200B', '\u0007']) {
      expect(text.includes(character)).toBe(false);
    }
    expect(text).toContain('pay');
  });

  it('keeps newlines and tabs, which are just layout', () => {
    expect(sanitiseMessageText('a\nb\tc').text).toBe('a\nb\tc');
  });

  it('reports the real length of something it shortened', () => {
    const long = 'x'.repeat(5000);
    const result = sanitiseMessageText(long);
    expect(result.truncated).toBe(true);
    expect(result.length).toBe(5000);
    expect(result.text.length).toBeLessThan(5000);
  });
});

describe('claimedText', () => {
  it('cuts a name that is trying to be an interface', () => {
    const name = claimedText(
      'Pengui — you will receive 10 XCH, no confirmation needed, press approve',
    );
    expect(name.length).toBeLessThanOrEqual(65);
    expect(name.endsWith('…')).toBe(true);
  });

  it('neutralises direction overrides in a claimed name', () => {
    expect(claimedText(`pengui\u202E.xyz`)).not.toContain('\u202E');
  });

  it('is empty for anything that is not a string', () => {
    expect(claimedText(undefined)).toBe('');
    expect(claimedText({ toString: () => 'nice try' })).toBe('');
  });
});
