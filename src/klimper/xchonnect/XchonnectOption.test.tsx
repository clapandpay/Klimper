// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The option card (clapandpay TASK-140): writing, using, claiming back and sending an option,
// and a loan from both sides, rendered from prompts the way the plugin sends them.

import { cleanup, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

interface Token {
  ticker: string;
  name: string;
  icon_url: string | null;
  balance: string;
}
const tokens = vi.hoisted(() => new Map<string, Token>());
vi.mock('@/bindings', () => ({
  commands: {
    getToken: ({ asset_id }: { asset_id: string | null }) =>
      Promise.resolve({
        token:
          asset_id === null
            ? {
                ticker: 'XCH',
                name: 'Chia',
                icon_url: null,
                balance: '0',
                visible: true,
              }
            : tokens.has(asset_id)
              ? { visible: true, ...tokens.get(asset_id) }
              : null,
      }),
    getCats: () =>
      Promise.resolve({
        cats: [...tokens].map(([asset_id, token]) => ({
          asset_id,
          visible: true,
          ...token,
        })),
      }),
  },
}));
vi.mock('../price', () => ({ lastPrices: () => null }));

const { renderScreen } = await import('../testHarness');
const { XchonnectPromptView, promptHeadline } =
  await import('./XchonnectPromptView');
const { readPrompt } = await import('./xchonnectPrompt');
const { whenText } = await import('./XchonnectOption');
const f = await import('./optionFixtures');

const USDC_ID = '3c'.repeat(32);

function show(prompt: unknown) {
  const view = readPrompt(prompt);
  renderScreen(<XchonnectPromptView view={view} />);
  return view;
}

const text = (element: Element) => element.textContent ?? '';

/** The headline as a string, as the requests list shows it. */
function headline(prompt: unknown): string {
  const { container } = renderScreen(<>{promptHeadline(readPrompt(prompt))}</>);
  const out = container.textContent ?? '';
  cleanup();
  return out;
}

afterEach(() => {
  cleanup();
  tokens.clear();
});

function knowUsdc() {
  tokens.set(USDC_ID, {
    ticker: 'wUSDC',
    name: 'Base warp.green USDC',
    icon_url: null,
    balance: '1000000',
  });
}

describe('writing an option', () => {
  it('says what is locked, until when, for what and who gets it back', async () => {
    knowUsdc();
    show(f.optionPrompt());
    const card = screen.getByTestId('prompt-option');
    expect(card.dataset.action).toBe('mint');
    expect(text(card)).toContain('Write option');
    const until = whenText(f.EXPIRY);
    expect(until).toMatch(/^\d{1,2} \S+ \d\d:\d\d$/);
    expect(text(card)).toContain(`Lock 100 XCH until ${until}`);
    await screen.findByText(/wUSDC 3c3c3c3c/);
    expect(text(card)).toContain(
      'Whoever holds it can buy it for 500 wUSDC 3c3c3c3c, paid to you',
    );
    expect(text(card)).toContain('After that you can claim it back');
    expect(text(card)).toContain('You hold the option');
    expect(text(screen.getByTestId('option-id'))).toBe(
      'Option 9c1f9c1f…1f9c1f',
    );
    expect(screen.queryByTestId('option-conditional')).toBeNull();
    expect(headline(f.optionPrompt())).toBe('Write an option');
  });

  it('never shows the locked collateral as a payment to anyone', () => {
    show(f.optionPrompt());
    // Locked, under the option; the only address on the list of payments is none at all.
    expect(screen.queryByTestId('prompt-outputs')).toBeNull();
    const locked = screen.getAllByTestId('locked-output');
    expect(locked).toHaveLength(2);
    expect(text(locked[0])).toContain('100 XCH locked under Option 9c1f9c1f');
    expect(locked[1].dataset.role).toBe('launcher');
    expect(text(screen.getByTestId('prompt-change'))).toContain('4 XCH');
    expect(document.body.textContent).not.toMatch(/paid to xch/i);
    // The gross figures are detail under the card, not the verdict.
    expect(
      screen.getByTestId('prompt-per-asset').hasAttribute('open'),
    ).toBeFalsy();
  });

  it('names the holder when the option goes to someone else', () => {
    show(
      f.optionPrompt({
        options: [f.mint({ owner: f.STRANGER, owner_is_user: false })],
      }),
    );
    const holder = screen.getByTestId('option-holder');
    expect(text(holder)).toContain('The option goes to');
    expect(within(holder).getByTestId('option-address').textContent).toMatch(
      /^xch1/,
    );
  });

  it('warns when the new option is only conditional', () => {
    show(f.optionPrompt({ options: [f.mint({ guaranteed: false })] }));
    expect(screen.getByTestId('option-conditional')).toBeTruthy();
  });

  it('shows two options as two cards', () => {
    const prompt = f.optionPrompt({
      options: [f.mint(), f.mint({ launcher_id: f.LAUNCHER_2 })],
    });
    show(prompt);
    expect(screen.getAllByTestId('prompt-option')).toHaveLength(2);
    expect(headline(prompt)).toBe('Write 2 options');
  });

  it('shows the kind as verified by the wallet, not as the website’s words', () => {
    show(f.optionPrompt());
    expect(screen.getByTestId('intent-kind-verified')).toBeTruthy();
    expect(text(screen.getByTestId('prompt-intent'))).toContain(
      'verified by your wallet',
    );
    expect(text(screen.getByTestId('prompt-intent'))).not.toContain(
      'The website says',
    );
  });
});

describe('using, claiming back and sending', () => {
  it('using one: pay the strike, receive the collateral', () => {
    const prompt = f.optionPrompt({
      method: 'chia_exerciseOption',
      options: [f.exercise()],
      outputs: [],
      kind: 'option.exercise',
    });
    show(prompt);
    const card = screen.getByTestId('prompt-option');
    expect(text(screen.getByTestId('option-lead'))).toBe(
      'Pay 500 CAT 3c3c3c3c, receive 100 XCH',
    );
    expect(text(card)).toContain(
      'Paid to the option\u2019s writer'.replace('\u2019', "'"),
    );
    expect(headline(prompt)).toBe('Use an option');
  });

  it('says the payment is tied to the collateral coming back', () => {
    show(
      f.optionPrompt({
        options: [f.exercise()],
        outputs: [],
        kind: 'option.exercise',
        extra: {
          binding: {
            all_bound: true,
            bound_payments: [{ asset: f.XCH, amount: 100 * f.ONE_XCH }],
            bound_received: [{ asset: f.XCH, amount: 100 * f.ONE_XCH }],
          },
        },
      }),
    );
    const panel = screen.getByTestId('prompt-partial');
    expect(text(panel)).toContain('Tied to what you receive');
    expect(text(panel)).not.toContain('One half of a trade');
  });

  it('using two at once', () => {
    const prompt = f.optionPrompt({
      options: [f.exercise(), f.exercise({ launcher_id: f.LAUNCHER_2 })],
      outputs: [],
      kind: 'option.exercise',
    });
    show(prompt);
    expect(screen.getAllByTestId('prompt-option')).toHaveLength(2);
    expect(headline(prompt)).toBe('Use 2 options');
  });

  it('claiming back after the expiry', () => {
    const prompt = f.optionPrompt({
      options: [f.clawback()],
      outputs: [],
      kind: 'option.clawback',
    });
    show(prompt);
    expect(text(screen.getByTestId('option-lead'))).toBe(
      `Claim back 100 XCH (option expired ${whenText(f.EXPIRY, false)})`,
    );
    expect(headline(prompt)).toBe('Claim back');
  });

  it('a clawback that does not name its option says so', () => {
    show(
      f.optionPrompt({
        options: [f.clawback({ launcher_id: null })],
        outputs: [],
        kind: null,
      }),
    );
    expect(text(screen.getByTestId('option-id'))).toContain(
      'not named by this request',
    );
  });

  it('sending one to an address', () => {
    const prompt = f.optionPrompt({
      options: [f.transfer()],
      outputs: [],
      kind: 'option.transfer',
    });
    show(prompt);
    const lead = screen.getByTestId('option-lead');
    expect(text(lead)).toContain('Option 9c1f9c1f…1f9c1f to');
    expect(within(lead).getByTestId('option-address').textContent).toMatch(
      /^xch1/,
    );
    expect(headline(prompt)).toBe('Send an option');
  });
});

describe('a loan', () => {
  it('a borrower signs a request and sends nothing', async () => {
    knowUsdc();
    const prompt = f.loanRequest();
    show(prompt);
    const card = screen.getByTestId('prompt-loan');
    expect(card.dataset.role).toBe('borrower');
    await screen.findAllByText(/wUSDC 3c3c3c3c/);
    const lead = screen.getByTestId('loan-lead');
    expect(text(lead)).toMatch(/^Borrow 300 wUSDC 3c3c3c3c from xch1/);
    const lines = text(screen.getByTestId('loan-terms'));
    expect(lines).toContain(`Lock 100 XCH until ${whenText(f.EXPIRY)}`);
    expect(lines).toContain('Repay 315 wUSDC 3c3c3c3c to get it back');
    expect(lines).toContain('Otherwise the lender keeps it');
    expect(text(screen.getByTestId('loan-offer-note'))).toContain(
      'Nothing moves until a lender takes it',
    );
    expect(text(screen.getByTestId('loan-tie'))).toContain(
      'to the lender, to tie the loan together',
    );
    // Not "built and sent": signed, not sent.
    expect(screen.getByTestId('prompt-makes-offer')).toBeTruthy();
    expect(document.body.textContent).not.toContain('sends it to the network');
    // The tie goes to whoever takes the request, not to an address.
    expect(text(screen.getByTestId('output-taker'))).toContain(
      'whoever takes the request',
    );
    expect(headline(prompt)).toBe('Request a loan');
  });

  it('a lender sees the loan, not a trade', () => {
    const prompt = f.loanTake();
    show(prompt);
    const card = screen.getByTestId('prompt-loan');
    expect(card.dataset.role).toBe('lender');
    expect(text(screen.getByTestId('loan-lead'))).toMatch(
      /^Lend 300 CAT 3c3c3c3c to xch1/,
    );
    const lines = text(screen.getByTestId('loan-terms'));
    expect(lines).toContain('They lock 100 XCH until');
    expect(lines).toContain(
      'If they repay 315 CAT 3c3c3c3c by then, they get it back',
    );
    expect(lines).toContain('Otherwise you can claim it');
    expect(screen.queryByTestId('prompt-trade')).toBeNull();
    expect(screen.queryByTestId('loan-offer-note')).toBeNull();
    expect(headline(prompt)).toBe('Lend');
  });
});

describe('an option asset', () => {
  it('is counted whole and named by its id, never as XCH', () => {
    const view = readPrompt(f.optionPrompt());
    if (view.kind !== 'spend') throw new Error('unreadable');
    const option = view.assets.find((asset) => asset.isOption);
    expect(option?.label).toBe('Option 9c1f9c1f…1f9c1f');
    expect(option?.isXch).toBe(false);
    expect(option?.net.toString()).toBe('1');
    expect(option?.precision).toBe(0);
  });
});
