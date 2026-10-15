// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The trade card (clapandpay TASK-139), rendered from prompts with 1, 2, 5 and 10 assets the
// way the plugin sends them: wallet-kit's prompt with Klimper's own `trade` key.

import { cleanup, fireEvent, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

interface Token {
  ticker: string;
  name: string;
  icon_url: string | null;
  balance: string;
  visible?: boolean;
}
const tokens = vi.hoisted(() => new Map<string, Token>());
/** CATs the wallet knows that are not part of the request (for the twin check). */
const others = vi.hoisted(() => new Map<string, Token>());
const prices = vi.hoisted(() => ({ current: null as unknown }));
vi.mock('@/bindings', () => ({
  commands: {
    getToken: ({ asset_id }: { asset_id: string | null }) => {
      if (asset_id === null) {
        return Promise.resolve({
          token: {
            ticker: 'XCH',
            name: 'Chia',
            icon_url: 'https://icons.example/xch.png',
            balance: '0',
            visible: true,
          },
        });
      }
      const token = tokens.get(asset_id);
      return Promise.resolve({
        token: token ? { visible: true, ...token } : null,
      });
    },
    getCats: () =>
      Promise.resolve({
        cats: [...tokens, ...others].map(([asset_id, token]) => ({
          asset_id,
          visible: true,
          ...token,
        })),
      }),
  },
}));
vi.mock('../price', () => ({ lastPrices: () => prices.current }));

const { renderScreen } = await import('../testHarness');
const { XchonnectPromptView, promptHeadline } =
  await import('./XchonnectPromptView');
const { readPrompt } = await import('./xchonnectPrompt');
const { monogramColour, groupedAmount } = await import('./XchonnectTrade');
const { safeTicker, tickerSkeleton } = await import('./ticker');
const BigNumber = (await import('bignumber.js')).default;

type Asset = { type: 'xch' } | { type: 'cat'; asset_id: string };
const XCH: Asset = { type: 'xch' };
const cat = (byte: string): Asset => ({
  type: 'cat',
  asset_id: byte.repeat(32),
});
type Leg = [Asset, number];

/** A take as the plugin sends it: the simulation, the binding, and Klimper's trade. */
function take(
  pays: Leg[],
  receives: Leg[],
  offers: number,
  fee: number,
  price: Record<string, unknown> | null = null,
) {
  const leg = ([asset, amount]: Leg) => ({ asset, amount: String(amount) });
  return {
    type: 'submit_coin_spends',
    dapp: 'pengui.space',
    method: 'chia_takeOffer',
    wallet_built: true,
    summary: {
      assets: [
        ...pays.map(([asset, amount]) => ({
          asset,
          sent: amount,
          received: 0,
          conditional_received: 0,
          net: -amount,
        })),
        ...receives.map(([asset, amount]) => ({
          asset,
          sent: 0,
          received: 0,
          conditional_received: amount,
          net: 0,
        })),
      ],
      implied_fee: fee,
      reserve_fee: fee,
      time_locks: {},
      unknown_puzzles: [],
      outputs: [],
    },
    plan: { ours: [], foreign: [{}], has_unsafe: false },
    binding: {
      all_bound: true,
      bound_payments: receives.map(([asset, amount]) => ({ asset, amount })),
      bound_received: receives.map(([asset, amount]) => ({ asset, amount })),
    },
    intent: { kind: 'offer.take', verified: [], recipients_complete: false },
    cross_checked: true,
    trade: {
      offers,
      pays: pays.map(leg),
      receives: receives.map(leg),
      fee: String(fee),
      price,
    },
  };
}

function show(prompt: unknown) {
  const view = readPrompt(prompt);
  renderScreen(<XchonnectPromptView view={view} />);
  return view;
}

/** All the text of an element, as a user would read it. */
const text = (element: Element) => element.textContent ?? '';

const legs = (side: 'pay' | 'receive') =>
  within(screen.getByTestId(`trade-${side}`)).queryAllByTestId('trade-leg');

afterEach(() => {
  cleanup();
  tokens.clear();
  others.clear();
  prices.current = null;
});

describe('the trade card', () => {
  it('is not shown for a plain send (one asset)', () => {
    show({
      type: 'submit_coin_spends',
      method: 'chia_send',
      wallet_built: true,
      summary: {
        assets: [
          {
            asset: XCH,
            sent: 1_000,
            received: 0,
            conditional_received: 0,
            net: -1_000,
          },
        ],
        implied_fee: 0,
        reserve_fee: 0,
      },
      plan: { ours: [] },
    });
    expect(screen.queryByTestId('prompt-trade')).toBeNull();
    expect(screen.getAllByTestId('prompt-asset')).toHaveLength(1);
  });

  it('shows two assets with the price both ways round, the offers and the fee', async () => {
    tokens.set('5b'.repeat(32), {
      ticker: 'SBX',
      name: 'Spacebucks',
      icon_url: 'https://icons.example/sbx.png',
      balance: '2000',
    });
    const view = show(
      take([[XCH, 1_500_000_000_000]], [[cat('5b'), 3_000_000]], 3, 100, {
        base: cat('5b'),
        quote: XCH,
        quote_per_base: '0.0005',
        base_per_quote: '2000',
      }),
    );
    expect(legs('pay')).toHaveLength(1);
    expect(legs('receive')).toHaveLength(1);
    expect(text(legs('pay')[0])).toContain('−1.5 XCH');
    // The wallet's ticker as the unit, its name, and the asset id, always.
    expect(await screen.findByText('Spacebucks')).toBeTruthy();
    expect(text(legs('receive')[0])).toContain('+3 000 SBX');
    expect(
      text(within(legs('receive')[0]).getByTestId('trade-asset-id')),
    ).toContain('CAT 5b5b5b5b…5b5b5b');
    expect(screen.getByTestId('trade-icon').getAttribute('src')).toBe(
      'https://icons.example/sbx.png',
    );

    // The price names the CAT by ticker and the start of its id.
    expect(text(screen.getByTestId('trade-price-text'))).toContain(
      '1 SBX 5b5b5b5b = 0.0005 XCH',
    );
    fireEvent.click(screen.getByTestId('trade-price-flip'));
    expect(text(screen.getByTestId('trade-price-text'))).toContain(
      '1 XCH = 2000 SBX 5b5b5b5b',
    );

    expect(text(screen.getByTestId('trade-offers'))).toContain(
      '3 separate offers taken together — all or nothing',
    );
    expect(text(screen.getByTestId('trade-fee'))).toContain('0.0000000001 XCH');
    // The per-asset verdict is still there, folded under the card.
    expect(screen.getByTestId('prompt-per-asset')).toBeTruthy();

    cleanup();
    renderScreen(<>{promptHeadline(view)}</>);
    expect(screen.getByText('Take 3 offers together')).toBeTruthy();
  });

  it('says nothing about offers for a single offer, and its headline is one take', () => {
    const view = show(
      take([[XCH, 500]], [[cat('5b'), 1_000]], 1, 0, {
        base: cat('5b'),
        quote: XCH,
        quote_per_base: '0.0000000005',
        base_per_quote: '2000000000',
      }),
    );
    expect(screen.queryByTestId('trade-offers')).toBeNull();
    expect(screen.queryByTestId('trade-fee')).toBeNull();
    // Without a ticker, a CAT's price names it by its id.
    expect(text(screen.getByTestId('trade-price-text'))).toContain(
      '1 CAT 5b5b5b5b = 0.0000000005 XCH',
    );
    cleanup();
    renderScreen(<>{promptHeadline(view)}</>);
    expect(screen.getByText('Take an offer')).toBeTruthy();
  });

  it('lists five assets in full, with no single price', () => {
    show(
      take(
        [
          [XCH, 2_250_000_000_000],
          [cat('7e'), 1_250_000],
        ],
        [
          [cat('5b'), 3_000_000],
          [cat('d1'), 420_000],
          [cat('a9'), 99_999_999],
        ],
        4,
        0,
      ),
    );
    expect(legs('pay')).toHaveLength(2);
    expect(legs('receive')).toHaveLength(3);
    expect(screen.queryByTestId('trade-receive-more')).toBeNull();
    expect(screen.queryByTestId('trade-price')).toBeNull();
    expect(text(screen.getByTestId('trade-receive'))).toContain('3 assets');
  });

  it('folds ten assets to the first three of a side, and opens them all', () => {
    const receives: Leg[] = [
      '5b',
      'd1',
      'a9',
      '7e',
      'c4',
      'e2',
      '0f',
      '91',
    ].map((byte, index) => [cat(byte), (index + 1) * 1_000]);
    show(
      take(
        [
          [XCH, 12_345_678_901_234],
          [cat('3c'), 250_000_000],
        ],
        receives,
        9,
        100_000_000,
      ),
    );
    expect(legs('pay')).toHaveLength(2);
    expect(legs('receive')).toHaveLength(3);
    const more = screen.getByTestId('trade-receive-more');
    expect(text(more)).toContain('and 5 more');
    expect(more.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(more);
    expect(legs('receive')).toHaveLength(8);
    expect(text(more)).toContain('Show fewer');
    expect(text(screen.getByTestId('trade-offers'))).toContain(
      '9 separate offers taken together',
    );
    // Eight bound payments are folded in the binding panel under the card.
    expect(text(screen.getByTestId('bound-payments-folded'))).toContain(
      'Depends on 8 payments to you',
    );
  });

  it('never uses a ticker or an icon that passes for the network coin', async () => {
    const lookalikes = [
      'ХСН', // Cyrillic
      'ΧCΗ', // Greek
      'ＸＣＨ', // fullwidth
      'XCH.',
      'X-CH',
      'wXCH',
      'xch',
      'CH1A',
      'Chia',
    ];
    for (const ticker of lookalikes) {
      tokens.set('c4'.repeat(32), {
        ticker,
        name: 'Lookalike',
        icon_url: 'https://icons.example/chia-logo.png',
        balance: '5000',
      });
      show(take([[XCH, 500]], [[cat('c4'), 5_000]], 1, 0));
      expect(await screen.findByText('Lookalike')).toBeTruthy();
      const received = legs('receive')[0];
      expect(text(within(received).getByTestId('trade-amount'))).toContain(
        '+5 CAT',
      );
      // No icon of the token's own, not even one the wallet holds: a monogram of its id.
      expect(screen.queryByTestId('trade-icon')).toBeNull();
      expect(within(received).getByTestId('trade-monogram').textContent).toBe(
        'C4',
      );
      expect(screen.queryByTestId('trade-twin')).toBeNull();
      cleanup();
      tokens.clear();
    }
  });

  it('never uses a ticker that claims to be the network coin', async () => {
    tokens.set('c4'.repeat(32), {
      ticker: 'XCH',
      name: 'Totally XCH',
      icon_url: null,
      balance: '0',
    });
    show(take([[XCH, 500]], [[cat('c4'), 5_000]], 1, 0));
    expect(await screen.findByText('Totally XCH')).toBeTruthy();
    const received = legs('receive')[0];
    expect(text(within(received).getByTestId('trade-amount'))).toContain(
      '+5 CAT',
    );
    expect(text(within(received).getByTestId('trade-asset-id'))).toContain(
      'CAT c4c4c4c4…c4c4c4',
    );
  });

  it('loads no icon for a CAT the wallet does not hold or hides, and none for XCH', async () => {
    tokens.set('5b'.repeat(32), {
      ticker: 'SBX',
      name: 'Spacebucks',
      icon_url: 'https://icons.example/sbx.png',
      balance: '0',
    });
    tokens.set('d1'.repeat(32), {
      ticker: 'DBX',
      name: 'dexie bucks',
      icon_url: 'https://icons.example/dbx.png',
      balance: '12000',
      visible: false,
    });
    show(
      take(
        [[XCH, 500]],
        [
          [cat('5b'), 5_000],
          [cat('d1'), 7_000],
        ],
        1,
        0,
      ),
    );
    expect(await screen.findByText('Spacebucks')).toBeTruthy();
    expect(await screen.findByText('dexie bucks')).toBeTruthy();
    expect(screen.queryByTestId('trade-icon')).toBeNull();
    expect(
      screen.getAllByTestId('trade-monogram').map((m) => m.textContent),
    ).toEqual(['SB', 'DB']);
  });

  it('says so when another token in the wallet reads the same', async () => {
    tokens.set('5b'.repeat(32), {
      ticker: 'SBX',
      name: 'Spacebucks',
      icon_url: null,
      balance: '0',
    });
    // The real one, already in the wallet; the one offered is spelled with a 5.
    others.set('aa'.repeat(32), {
      ticker: '5BX',
      name: 'Spacebucks',
      icon_url: null,
      balance: '1000',
    });
    show(take([[XCH, 500]], [[cat('5b'), 5_000]], 1, 0));
    expect(text(await screen.findByTestId('trade-twin'))).toContain(
      'Two tokens in your wallet use the ticker SBX',
    );
  });

  it('shows dollars for XCH and never for a CAT, whatever its last price', async () => {
    prices.current = {
      xchUsd: 20,
      cats: { ['5b'.repeat(32)]: { lastPrice: 1_000, askPrice: 1_000 } },
      source: 'nodexch',
      at: 0,
    };
    tokens.set('5b'.repeat(32), {
      ticker: 'SBX',
      name: 'Spacebucks',
      icon_url: null,
      balance: '0',
    });
    show(take([[XCH, 500_000_000_000]], [[cat('5b'), 5_000]], 1, 0));
    expect(await screen.findByText('Spacebucks')).toBeTruthy();
    expect(
      within(legs('pay')[0]).getByTestId('trade-usd').textContent,
    ).toContain('$10.00');
    expect(within(legs('receive')[0]).queryByTestId('trade-usd')).toBeNull();
  });

  it('marks a CAT the wallet has never held, and draws it a monogram of its own', async () => {
    show(
      take(
        [[XCH, 500]],
        [
          [cat('e2'), 5_000],
          [cat('0f'), 7_000],
        ],
        2,
        0,
      ),
    );
    expect(await screen.findAllByTestId('trade-unheld')).toHaveLength(2);
    const monograms = screen.getAllByTestId('trade-monogram');
    expect(monograms.map((m) => m.textContent)).toEqual(['E2', '0F']);
    expect(monogramColour('e2'.repeat(32))).toEqual(
      monogramColour('e2'.repeat(32)),
    );
  });
});

describe('groupedAmount', () => {
  it('sets thousands apart and keeps every digit', () => {
    expect(groupedAmount(BigNumber(99_999_999), 3)).toBe('99 999.999');
    expect(groupedAmount(BigNumber(1_500_000_000_000), 12)).toBe('1.5');
    expect(groupedAmount(BigNumber(1_234_567_000), 3)).toBe('1 234 567');
  });
});

describe('safeTicker', () => {
  it('keeps plain ASCII tickers, folded', () => {
    expect(safeTicker('SBX')).toBe('SBX');
    expect(safeTicker('wUSDC.b')).toBe('wUSDC.b');
    expect(safeTicker(' $HOA ')).toBe('$HOA');
    expect(safeTicker('ＳＢＸ')).toBe('SBX');
  });

  it('refuses anything that is not plain ASCII, or too long', () => {
    expect(safeTicker('ЅВХ')).toBeNull();
    expect(safeTicker('SBX🚀')).toBeNull();
    expect(safeTicker('A'.repeat(13))).toBeNull();
    expect(safeTicker('')).toBeNull();
    expect(safeTicker(null)).toBeNull();
  });

  it('refuses whatever reads as XCH or Chia', () => {
    for (const ticker of [
      'XCH',
      'xch',
      'TXCH',
      'XCH.',
      'X-CH',
      'X_C_H',
      'wXCH',
      'XCHX',
      'CHIA',
      'CH1A',
      'chia',
      'ＸＣＨ',
      'ХСН',
      'ΧCΗ',
    ]) {
      expect(safeTicker(ticker), ticker).toBeNull();
    }
    expect(safeTicker('TXCH', 'TXCH')).toBeNull();
  });

  it('gives look-alikes one skeleton', () => {
    expect(tickerSkeleton('ХСН')).toBe('XCH');
    expect(tickerSkeleton('ΧCΗ')).toBe('XCH');
    expect(tickerSkeleton('x.c-h')).toBe('XCH');
    expect(tickerSkeleton('5BX')).toBe(tickerSkeleton('SBX'));
  });
});
