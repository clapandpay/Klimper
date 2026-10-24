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
import * as f from './optionFixtures';

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
        sent: '1500000000000',
        received: '0',
        conditional_received: '0',
        net: '-1500000000000',
      },
    ],
    implied_fee: '500000',
    reserve_fee: '0',
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
        sent: '1000000000000',
        received: '0',
        conditional_received: '0',
        net: '-1000000000000',
      },
      {
        asset: { type: 'cat', asset_id: TAIL },
        sent: '0',
        received: '0',
        conditional_received: '250000',
        net: '0',
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

  it('reads spends Klimper cannot account for strictly, and asks twice', () => {
    const COIN = 'd'.repeat(64);
    const output = (unaccounted: unknown) => ({
      asset: { type: 'xch' },
      amount: 1,
      puzzle_hash: PUZZLE,
      recipient: PUZZLE,
      to_user: true,
      option: null,
      unaccounted,
    });
    const view = readPrompt({
      ...simpleSend,
      summary: {
        ...simpleSend.summary,
        unknown_puzzles: [PUZZLE],
        unaccounted_spends: [COIN],
        outputs: [output(true), output(false)],
      },
    });
    if (view.kind !== 'spend') throw new Error('expected a spend');
    expect(view.unaccountedSpends).toEqual([COIN]);
    expect(view.outputs.map((o) => o.unaccounted)).toEqual([true, false]);
    expect(needsSecondConfirmation(view)).toBe(true);

    // Anything but a list of coin ids, or a flag that is not a boolean: unreadable.
    for (const summary of [
      { unaccounted_spends: ['not a coin'] },
      { unaccounted_spends: COIN },
      { unaccounted_spends: [COIN, null] },
      { outputs: [output('yes')] },
      { outputs: [output(null)] },
    ]) {
      expect(
        readPrompt({
          ...simpleSend,
          summary: { ...simpleSend.summary, ...summary },
        }).kind,
      ).toBe('unreadable');
    }
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

    it('reads time locks given as decimal strings or numbers', () => {
      const view = readPrompt({
        ...simpleSend,
        summary: {
          ...simpleSend.summary,
          time_locks: {
            not_before_seconds: '1794578400',
            not_before_height: 6_000_000,
            expires_seconds: 1_794_582_000,
            expires_height: null,
            has_relative: false,
          },
        },
      });
      if (view.kind !== 'spend') throw new Error('expected a spend');
      expect(view.timeLocks).toMatchObject({
        notBeforeSeconds: 1_794_578_400,
        notBeforeHeight: 6_000_000,
        expiresSeconds: 1_794_582_000,
        expiresHeight: null,
      });
    });

    it('refuses a time lock it cannot show exactly, rather than dropping it', () => {
      for (const expires of [
        'soon',
        '1794578400.5',
        '-1',
        '9007199254740992',
      ]) {
        const view = readPrompt({
          ...simpleSend,
          summary: {
            ...simpleSend.summary,
            time_locks: {
              ...simpleSend.summary.time_locks,
              expires_seconds: expires,
            },
          },
        });
        expect(view.kind, expires).toBe('unreadable');
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

describe('the trade overview (TASK-139)', () => {
  const trade = {
    offers: 3,
    pays: [{ asset: { type: 'xch' }, amount: '1000000000000' }],
    receives: [{ asset: { type: 'cat', asset_id: TAIL }, amount: '250000' }],
    fee: '100',
    price: {
      base: { type: 'cat', asset_id: TAIL },
      quote: { type: 'xch' },
      quote_per_base: '0.004',
      base_per_quote: '250',
    },
  };

  it('reads the wallet’s trade by explicit key', () => {
    const view = readPrompt({ ...swap, trade });
    if (view.kind !== 'spend') throw new Error('expected a spend');
    expect(view.trade?.offers).toBe(3);
    expect(view.trade?.pays[0].label).toBe('XCH');
    expect(view.trade?.pays[0].amount.toString()).toBe('1000000000000');
    expect(view.trade?.receives[0].assetId).toBe(TAIL);
    expect(view.trade?.receives[0].precision).toBe(3);
    expect(view.trade?.fee.toString()).toBe('100');
    expect(view.trade?.price?.quotePerBase).toBe('0.004');
    expect(view.trade?.price?.base.label).toBe(
      `CAT ${TAIL.slice(0, 8)}…${TAIL.slice(-6)}`,
    );
  });

  it('is absent when the wallet sent none', () => {
    const view = readPrompt(swap);
    if (view.kind !== 'spend') throw new Error('expected a spend');
    expect(view.trade).toBeNull();
  });

  it('drops a name or a label the trade grew, and a price that is not a number', () => {
    const view = readPrompt({
      ...swap,
      trade: {
        ...trade,
        label: 'Best deal ever',
        receives: [{ ...trade.receives[0], name: 'Totally XCH' }],
        price: { ...trade.price, quote_per_base: 'cheap!' },
      },
    });
    if (view.kind !== 'spend') throw new Error('expected a spend');
    expect(view.trade?.price).toBeNull();
    expect(JSON.stringify(view)).not.toContain('Best deal ever');
    expect(JSON.stringify(view)).not.toContain('Totally XCH');
  });

  it('makes the prompt unreadable when a figure of the trade cannot be shown', () => {
    for (const broken of [
      { ...trade, pays: [{ asset: { type: 'xch' }, amount: 1.5 }] },
      { ...trade, pays: [{ asset: { type: 'nft' }, amount: '1' }] },
      { ...trade, receives: [] },
      { ...trade, pays: [{ asset: { type: 'xch' }, amount: '0' }] },
      'a trade',
    ]) {
      expect(readPrompt({ ...swap, trade: broken }).kind).toBe('unreadable');
    }
  });

  it('ignores an offer count that is not a count', () => {
    for (const offers of [0, -2, 1.5, '3', 1e9]) {
      const view = readPrompt({ ...swap, trade: { ...trade, offers } });
      if (view.kind !== 'spend') throw new Error('expected a spend');
      expect(view.trade?.offers).toBeNull();
    }
  });
});

// ---------------------------------------------------------------------------------------
// Option contracts and loans (TASK-140): `summary.options`, `outputs[].option`, the option
// asset, `intent.kind_verified`, Klimper's `loan` and `makes_offer`, read by explicit key.
// ---------------------------------------------------------------------------------------

describe('option keys', () => {
  const spend = (prompt: unknown) => {
    const view = readPrompt(prompt);
    if (view.kind !== 'spend') throw new Error(`not a spend: ${view.kind}`);
    return view;
  };
  const unreadable = (prompt: unknown) =>
    expect(readPrompt(prompt).kind).toBe('unreadable');
  /** The mint prompt with `summary.options` replaced. */
  const withOptions = (options: unknown) => {
    const prompt = f.optionPrompt();
    return { ...prompt, summary: { ...prompt.summary, options } };
  };

  it('reads a mint exactly', () => {
    const view = spend(f.optionPrompt());
    expect(view.options).toHaveLength(1);
    const [mint] = view.options;
    if (mint?.action !== 'mint') throw new Error('not a mint');
    expect(mint.launcherId).toBe(f.LAUNCHER);
    expect(mint.terms.expiresAt).toBe(f.EXPIRY);
    expect(mint.terms.collateral.isXch).toBe(true);
    expect(mint.terms.collateral.amount.toString()).toBe(
      String(100 * f.ONE_XCH),
    );
    expect(mint.terms.strike.assetId).toBe('3c'.repeat(32));
    expect(mint.guaranteed).toBe(true);
    // Nothing the plugin did not list reaches the view.
    expect(Object.keys(mint).sort()).toEqual(
      [
        'action',
        'creatorIsUser',
        'fundedByUser',
        'guaranteed',
        'launcherId',
        'owner',
        'ownerIsUser',
        'terms',
      ].sort(),
    );
  });

  it('reads every action', () => {
    const view = spend(
      withOptions([f.mint(), f.exercise(), f.clawback(), f.transfer()]),
    );
    expect(view.options.map((o) => o.action)).toEqual([
      'mint',
      'exercise',
      'clawback',
      'transfer',
    ]);
  });

  it('absent options are none', () => {
    const prompt = f.optionPrompt();
    const summary: Record<string, unknown> = { ...prompt.summary };
    delete summary.options;
    const view = spend({ ...prompt, summary, intent: null });
    expect(view.options).toEqual([]);
    expect(view.loan).toBeNull();
    expect(view.makesOffer).toBe(false);
  });

  it('malformed options make the whole prompt unreadable', () => {
    unreadable(withOptions({}));
    unreadable(withOptions('mint'));
    unreadable(withOptions([null]));
    unreadable(withOptions([{ ...f.mint(), action: 'buy' }]));
    unreadable(withOptions([{ ...f.mint(), action: undefined }]));
    unreadable(withOptions([{ ...f.mint(), owner_is_user: 'true' }]));
    unreadable(withOptions([{ ...f.mint(), guaranteed: undefined }]));
    unreadable(withOptions([{ ...f.mint(), launcher_id: 'abcd' }]));
    unreadable(withOptions([{ ...f.mint(), terms: null }]));
    unreadable(
      withOptions([
        f.mint({ terms: { ...f.terms(), expiration_seconds: '1794578400.5' } }),
      ]),
    );
    unreadable(
      withOptions([
        f.mint({
          terms: { ...f.terms(), expiration_seconds: '9007199254740993' },
        }),
      ]),
    );
    unreadable(
      withOptions([
        f.mint({ terms: f.terms({ collateral: f.leg(f.XCH, 0) }) }),
      ]),
    );
    // An option backed by an option is not one this build reads.
    unreadable(
      withOptions([
        f.mint({
          terms: f.terms({ strike: f.leg(f.optionAsset(f.LAUNCHER_2), 1) }),
        }),
      ]),
    );
    unreadable(withOptions([{ ...f.transfer(), to: null }]));
    unreadable(withOptions([{ ...f.exercise(), by_user: 1 }]));
    unreadable(withOptions([{ ...f.clawback(), unlocks_at_seconds: -1 }]));
    unreadable(withOptions([{ ...f.clawback(), unlocks_at_seconds: '-1' }]));
    unreadable(withOptions([{ ...f.clawback(), launcher_id: 'zz' }]));
  });

  it('a clawback may leave its option unnamed (null), and only null', () => {
    const view = spend(withOptions([f.clawback({ launcher_id: null })]));
    expect(view.options[0]).toMatchObject({ launcherId: null });
    unreadable(withOptions([{ ...f.clawback(), launcher_id: undefined }]));
  });

  it('reads the option asset as its own kind, counted whole', () => {
    const view = spend(f.optionPrompt());
    const option = view.assets.find((a) => a.isOption);
    expect(option).toMatchObject({
      isXch: false,
      isOption: true,
      assetId: f.LAUNCHER,
      precision: 0,
    });
    unreadable(
      f.optionPrompt({ assets: [[{ type: 'option' } as f.Asset, 0, 1]] }),
    );
    unreadable(f.optionPrompt({ assets: [[{ type: 'nft' } as never, 0, 1]] }));
  });

  it('reads an option output as locked, with no recipient', () => {
    const view = spend(f.optionPrompt());
    const [collateral, launcher, change] = view.outputs;
    expect(collateral?.option).toEqual({
      launcherId: f.LAUNCHER,
      role: 'collateral',
    });
    expect(collateral?.recipient).toBeNull();
    expect(launcher?.option?.role).toBe('launcher');
    expect(change?.option).toBeNull();
    // A recipient on an option output is dropped: it is nobody's payment.
    const out = spend(
      f.optionPrompt({
        outputs: [
          {
            asset: f.XCH,
            amount: 5,
            puzzle_hash: f.STRANGER,
            recipient: f.STRANGER,
            to_user: false,
            option: { launcher_id: f.LAUNCHER, role: 'collateral' },
          },
        ],
      }),
    );
    expect(out.outputs[0]?.recipient).toBeNull();
  });

  it('a malformed option output makes the prompt unreadable', () => {
    const output = (option: unknown) =>
      f.optionPrompt({
        outputs: [
          {
            asset: f.XCH,
            amount: 5,
            puzzle_hash: 'f1'.repeat(32),
            recipient: null,
            to_user: false,
            option,
          },
        ],
      });
    unreadable(output({ launcher_id: f.LAUNCHER, role: 'payment' }));
    unreadable(output({ launcher_id: 'ab', role: 'collateral' }));
    unreadable(output('collateral'));
    expect(readPrompt(output(null)).kind).toBe('spend');
    expect(readPrompt(output(undefined)).kind).toBe('spend');
  });

  it('marks an output to the settlement puzzle', () => {
    const view = spend(f.loanRequest());
    expect(view.outputs.map((o) => o.toSettlement)).toEqual([false, true]);
  });

  it('reads kind_verified only for a verified kind', () => {
    const intent = (value: Record<string, unknown>) =>
      f.optionPrompt({ extra: { intent: value } });
    expect(
      spend(intent({ kind: 'option.mint', kind_verified: true })).intent
        ?.kindVerified,
    ).toBe(true);
    expect(
      spend(intent({ kind: 'option.buy', kind_verified: false })).intent
        ?.kindVerified,
    ).toBe(false);
    // Absent: an older wallet-kit, not verified.
    expect(spend(intent({ kind: 'option.mint' })).intent?.kindVerified).toBe(
      false,
    );
    unreadable(intent({ kind: 'option.mint', kind_verified: 'true' }));
    unreadable(intent({ kind: 'swap', kind_verified: true }));
    unreadable(intent({ kind: 'Option.Mint', kind_verified: true }));
    unreadable(intent({ kind_verified: true }));
  });

  it('reads a loan, tied to its mint', () => {
    const borrower = spend(f.loanRequest());
    expect(borrower.loan?.role).toBe('borrower');
    expect(borrower.loan?.principal.amount.toString()).toBe('300000');
    expect(borrower.loan?.counterparty).toBe(f.LENDER);
    expect(borrower.loan?.tie?.amount.toString()).toBe('1');
    expect(borrower.makesOffer).toBe(true);
    const lender = spend(f.loanTake());
    expect(lender.loan?.role).toBe('lender');
    expect(lender.loan?.counterparty).toBe(f.BORROWER);
    expect(lender.makesOffer).toBe(false);
    expect(
      spend(f.loanRequest({ loan: { ...loanOf(), tie: null } })).loan?.tie,
    ).toBeNull();
  });

  function loanOf() {
    return (f.loanRequest() as unknown as { loan: Record<string, unknown> })
      .loan;
  }

  it('a loan that does not match its option is unreadable', () => {
    unreadable(f.loanRequest({ loan: { ...loanOf(), role: 'friend' } }));
    unreadable(
      f.loanRequest({ loan: { ...loanOf(), counterparty: f.STRANGER } }),
    );
    unreadable(f.loanRequest({ loan: { ...loanOf(), principal: null } }));
    unreadable(f.loanRequest({ loan: { ...loanOf(), tie: 'one mojo' } }));
    unreadable(f.loanRequest({ loan: 'borrower' }));
    // The borrower's role with the lender's mint, and a loan with no mint at all.
    unreadable(f.loanTake({ loan: { ...loanOf(), counterparty: f.BORROWER } }));
    const noMint = f.loanRequest();
    unreadable({ ...noMint, summary: { ...noMint.summary, options: [] } });
    // Two mints: which one is the loan cannot be said.
    unreadable({
      ...noMint,
      summary: {
        ...noMint.summary,
        options: [...noMint.summary.options, ...noMint.summary.options],
      },
    });
  });

  it('makes_offer must be a boolean, and only counts on the wallet’s own build', () => {
    unreadable(f.loanRequest({ makes_offer: 'yes' }));
    const signOnly = { ...f.loanRequest(), type: 'sign_coin_spends' };
    expect(spend(signOnly).makesOffer).toBe(false);
  });

  it('reads linking strictly: sealed only on the wallet’s own build (spec 11.2)', () => {
    expect(spend({ ...f.optionPrompt(), linking: 'sealed' }).sealed).toBe(true);
    expect(spend({ ...f.optionPrompt(), linking: 'direct' }).sealed).toBe(
      false,
    );
    expect(spend(f.optionPrompt()).sealed).toBe(false);
    unreadable({ ...f.optionPrompt(), linking: 'Sealed' });
    unreadable({ ...f.optionPrompt(), linking: true });
    const website = {
      ...f.optionPrompt(),
      linking: 'sealed',
      wallet_built: false,
    };
    expect(spend(website).sealed).toBe(false);
    const signOnly = {
      ...f.optionPrompt(),
      linking: 'sealed',
      type: 'sign_coin_spends',
    };
    expect(spend(signOnly).sealed).toBe(false);
  });
});
