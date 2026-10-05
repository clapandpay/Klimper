// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The approval screen is the product's safety promise (clapandpay TASK-080). These are the
// four rules it keeps, each as a test:
//
//   * only the wallet's own simulation is rendered — a dApp-supplied string cannot reach
//     this screen, including the name the website gave itself;
//   * a conditional receipt is labelled as conditional and is not part of any total;
//   * an unreadable contract needs a second confirmation that shows the puzzle hash, and
//     `AGG_SIG_UNSAFE` is loud;
//   * one confirmation per signature, and no control that answers two approvals.

import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Approval } from './xchonnect';

vi.mock('@/components/Header', () => ({
  default: ({ title }: { title: string }) => <h1>{title}</h1>,
}));
vi.mock('@/lib/features', () => ({ xchonnectEnabled: true }));
vi.mock('@tauri-apps/plugin-opener', () => ({
  openUrl: vi.fn().mockResolvedValue(undefined),
}));
// The dialog reads the active theme, which loads theme JSON through a Vite import
// attribute Node does not take. The theme is not what these tests are about.
vi.mock('theme-o-rama', () => ({
  useTheme: () => ({ currentTheme: { mostLike: 'light' } }),
}));

const pendingApprovals = vi.fn();
const resolveApproval = vi.fn().mockResolvedValue(null);
const listSessions = vi.fn().mockResolvedValue([]);
const handlers = new Map<string, (payload: unknown) => void>();

vi.mock('./xchonnect', () => ({
  xchonnect: {
    pendingApprovals: () => pendingApprovals(),
    resolveApproval: (id: string, approved: boolean) =>
      resolveApproval(id, approved),
    listSessions: () => listSessions(),
  },
  XCHONNECT_EVENTS: {
    approval: 'approval',
    approvalResolved: 'approval-resolved',
  },
  onXchonnectEvent: (event: string, handler: (payload: unknown) => void) => {
    handlers.set(event, handler);
    return Promise.resolve(() => handlers.delete(event));
  },
}));

const { renderScreen } = await import('./testHarness');
const XchonnectApproval = (await import('./XchonnectApproval')).default;

const KEY_A = `0x${'a'.repeat(96)}`;
const KEY_B = `0x${'b'.repeat(96)}`;
const PUZZLE = 'f'.repeat(64);
const TAIL = '1a2b3c4d'.repeat(8);

function spendPrompt(over: Record<string, unknown> = {}) {
  return {
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
      time_locks: {},
      unknown_puzzles: [],
      cost: 1,
      spends: [],
    },
    plan: {
      ours: [
        {
          spend: 0,
          opcode: 50,
          public_key: KEY_A.slice(2),
          message: 'ff',
          is_unsafe: false,
        },
      ],
      foreign: [],
      has_unsafe: false,
    },
    partial: false,
    binding: null,
    ...over,
  };
}

function request(prompt: unknown, id = 'a1'): Approval {
  return {
    id,
    sessionId: 's1',
    dapp: 'pengui.xyz',
    method: 'signCoinSpends',
    expiresAt: Math.floor(Date.now() / 1000) + 300,
    kind: 'request',
    prompt: prompt as Record<string, unknown>,
  };
}

function signature(index: number, total: number, publicKey: string): Approval {
  return {
    id: `s${index}`,
    sessionId: 's1',
    dapp: 'pengui.xyz',
    method: 'signCoinSpends',
    expiresAt: Math.floor(Date.now() / 1000) + 300,
    kind: 'signature',
    index,
    total,
    publicKey,
  };
}

beforeEach(() => {
  handlers.clear();
  vi.clearAllMocks();
  resolveApproval.mockResolvedValue(null);
  listSessions.mockResolvedValue([]);
  pendingApprovals.mockResolvedValue([]);
});

afterEach(cleanup);

describe('only the wallet’s own simulation is rendered', () => {
  it('shows the computed net effect and the fee', async () => {
    pendingApprovals.mockResolvedValue([request(spendPrompt())]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() =>
      expect(screen.getByTestId('asset-net').textContent).toBe('−1.5 XCH'),
    );
    expect(screen.getByText('You pay')).toBeDefined();
    expect(screen.getByTestId('prompt-implied-fee').textContent).toContain(
      '0.0000005 XCH',
    );
    expect(screen.getByText(/not what the website says it does/)).toBeDefined();
  });

  it('renders no string the dApp put in the prompt', async () => {
    pendingApprovals.mockResolvedValue([
      request(
        spendPrompt({
          label: 'Free airdrop, nothing leaves your wallet',
          description: 'You receive 10 XCH',
          summary: {
            ...spendPrompt().summary,
            headline: 'Approved by Chia Network Inc.',
            assets: [
              {
                asset: {
                  type: 'cat',
                  asset_id: TAIL,
                  name: 'US Dollar (official)',
                },
                sent: 0,
                received: 0,
                conditional_received: 0,
                net: 0,
                label: 'a gift',
              },
            ],
          },
        }),
      ),
    ]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() => expect(screen.getByTestId('asset-net')).toBeDefined());

    // The whole rendered screen, including the disclosure that holds the raw prompt, is
    // searched for every claim the dApp made.
    const rendered = document.body.textContent ?? '';
    for (const claim of [
      'Free airdrop',
      'You receive 10 XCH',
      'Approved by Chia Network',
      'US Dollar (official)',
      'a gift',
    ]) {
      expect(rendered).not.toContain(claim);
    }
    // The asset is named from its TAIL hash, by the wallet.
    expect(screen.getByTestId('asset-net').textContent).toContain(
      `CAT ${TAIL.slice(0, 8)}…`,
    );
  });

  it('never shows the name the website gave itself', async () => {
    pendingApprovals.mockResolvedValue([request(spendPrompt())]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() => expect(screen.getByTestId('asset-net')).toBeDefined());
    // Only the verified domain identifies the requester here.
    expect(screen.queryByTestId('claimed-name')).toBeNull();
    expect(screen.getByText(/pengui\.xyz asks you to sign/)).toBeDefined();
  });

  it('refuses to describe a prompt it cannot read, and will not let it be approved', async () => {
    pendingApprovals.mockResolvedValue([
      request({ type: 'something_new', summary: { assets: [] } }),
    ]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() =>
      expect(screen.getByTestId('prompt-unreadable')).toBeDefined(),
    );
    expect(screen.getByRole('button', { name: 'Approve' })).toHaveProperty(
      'disabled',
      true,
    );
    expect(screen.getByRole('button', { name: 'Reject' })).toHaveProperty(
      'disabled',
      false,
    );
  });
});

describe('a conditional receipt never looks received', () => {
  it('labels it and keeps it out of the net effect', async () => {
    pendingApprovals.mockResolvedValue([
      request(
        spendPrompt({
          summary: {
            ...spendPrompt().summary,
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
        }),
      ),
    ]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() =>
      expect(screen.getByTestId('asset-conditional')).toBeDefined(),
    );
    const conditional = screen.getByTestId('asset-conditional').textContent;
    expect(conditional).toContain('Only if the counterparty completes');
    expect(conditional).toContain('+250');

    // The asset's own verdict line still says nothing changed for certain.
    const nets = screen
      .getAllByTestId('asset-net')
      .map((node) => node.textContent);
    expect(nets).toEqual(['−1 XCH', `0 CAT ${TAIL.slice(0, 8)}…`]);
    expect(screen.getByText('No change')).toBeDefined();
  });

  it('says what a partial signature depends on receiving', async () => {
    pendingApprovals.mockResolvedValue([
      request(
        spendPrompt({
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
        }),
      ),
    ]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() =>
      expect(screen.getByTestId('prompt-partial')).toBeDefined(),
    );
    expect(screen.getByTestId('bound-payment').textContent).toContain(
      'Depends on receiving',
    );
  });

  it('is loud about an unbound partial spend', async () => {
    pendingApprovals.mockResolvedValue([
      request(
        spendPrompt({
          partial: true,
          binding: {
            all_bound: false,
            unbound_user_spends: [0],
            bound_payments: [],
            bound_received: [],
          },
        }),
      ),
    ]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() =>
      expect(screen.getByTestId('prompt-unbound')).toBeDefined(),
    );
    expect(screen.getByText(/do not sign/)).toBeDefined();
  });
});

describe('the dangerous cases', () => {
  it('needs a second confirmation for an unknown contract, with the puzzle hash', async () => {
    pendingApprovals.mockResolvedValue([
      request(
        spendPrompt({
          summary: { ...spendPrompt().summary, unknown_puzzles: [PUZZLE] },
        }),
      ),
    ]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() =>
      expect(screen.getByTestId('prompt-unknown-contract')).toBeDefined(),
    );
    expect(screen.getByTestId('prompt-unknown-contract').textContent).toContain(
      PUZZLE,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Approve' }));

    // Nothing is approved by that press: the second decision is a separate one.
    expect(resolveApproval).not.toHaveBeenCalled();
    const dialog = await waitFor(() =>
      screen.getByTestId('unknown-contract-confirm'),
    );
    expect(dialog.textContent).toContain(PUZZLE);

    fireEvent.click(
      screen.getByRole('button', { name: 'I accept the unknown contract' }),
    );
    await waitFor(() =>
      expect(resolveApproval).toHaveBeenCalledWith('a1', true),
    );
  });

  it('lets the second confirmation be refused without approving anything', async () => {
    pendingApprovals.mockResolvedValue([
      request(
        spendPrompt({
          summary: { ...spendPrompt().summary, unknown_puzzles: [PUZZLE] },
        }),
      ),
    ]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() => screen.getByRole('button', { name: 'Approve' }));
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }));
    await waitFor(() => screen.getByTestId('unknown-contract-confirm'));
    fireEvent.click(screen.getByRole('button', { name: 'Go back' }));

    expect(resolveApproval).not.toHaveBeenCalled();
  });

  it('shows AGG_SIG_UNSAFE as a warning of its own', async () => {
    pendingApprovals.mockResolvedValue([
      request(
        spendPrompt({
          plan: { ...spendPrompt().plan, has_unsafe: true },
        }),
      ),
    ]);
    renderScreen(<XchonnectApproval />);

    const warning = await waitFor(() =>
      screen.getByTestId('prompt-unsafe-signature'),
    );
    expect(warning.textContent).toContain('Unsafe signature');
    expect(warning.textContent).toMatch(/valid in any spend/);
    expect(warning.className).toContain('border-destructive');
    expect(warning.getAttribute('role')).toBe('alert');
  });
});

describe('one confirmation per signature', () => {
  it('renders one signature at a time and answers only that one', async () => {
    pendingApprovals.mockResolvedValue([
      signature(1, 2, KEY_A),
      signature(2, 2, KEY_B),
    ]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() =>
      expect(screen.getByTestId('signature-approval')).toBeDefined(),
    );
    // The second signature is counted, not rendered: its key is nowhere on screen.
    expect(screen.getAllByTestId('signature-approval')).toHaveLength(1);
    expect(screen.getByTestId('signature-key').textContent).toBe(KEY_A);
    expect(document.body.textContent).not.toContain(KEY_B);
    expect(screen.getByTestId('queue-count').textContent).toContain(
      '1 more waiting',
    );

    // Exactly one authorising control exists, and it names which signature it is.
    const authorise = screen
      .getAllByRole('button')
      .filter((button) => /Authorise/.test(button.textContent ?? ''));
    expect(authorise).toHaveLength(1);
    expect(authorise[0].textContent).toBe('Authorise signature 1 of 2');

    fireEvent.click(authorise[0]);
    await waitFor(() =>
      expect(resolveApproval).toHaveBeenCalledWith('s1', true),
    );
    expect(resolveApproval).toHaveBeenCalledTimes(1);

    // The next one now asks on its own.
    await waitFor(() =>
      expect(screen.getByTestId('signature-key').textContent).toBe(KEY_B),
    );
    expect(
      screen.getByRole('button', { name: 'Authorise signature 2 of 2' }),
    ).toBeDefined();
  });

  it('rejecting a signature answers only that signature', async () => {
    pendingApprovals.mockResolvedValue([
      signature(1, 2, KEY_A),
      signature(2, 2, KEY_B),
    ]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() => screen.getByTestId('signature-approval'));
    fireEvent.click(screen.getByRole('button', { name: 'Reject' }));

    await waitFor(() =>
      expect(resolveApproval).toHaveBeenCalledWith('s1', false),
    );
    expect(resolveApproval).toHaveBeenCalledTimes(1);
  });
});

describe('the queue', () => {
  it('rebuilds itself after a reload', async () => {
    // The request is parked on the approval bridge in Rust; a reloaded webview has to
    // find it again or the user watches it time out.
    pendingApprovals.mockResolvedValue([request(spendPrompt())]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() => expect(pendingApprovals).toHaveBeenCalled());
    expect(screen.getByTestId('asset-net')).toBeDefined();
  });

  it('takes an approval that arrives while it is open, and drops a resolved one', async () => {
    renderScreen(<XchonnectApproval />);
    await waitFor(() =>
      expect(screen.getByTestId('nothing-waiting')).toBeDefined(),
    );

    handlers.get('approval')?.(request(spendPrompt()));
    await waitFor(() => expect(screen.getByTestId('asset-net')).toBeDefined());

    handlers.get('approval-resolved')?.({ id: 'a1', approved: false });
    await waitFor(() =>
      expect(screen.getByTestId('nothing-waiting')).toBeDefined(),
    );
  });

  it('answers a /req link with nothing pending by syncing, not by guessing', async () => {
    const {
      offerXchonnectLink,
      resetXchonnectLinkHandoff,
      setXchonnectNavigator,
    } = await import('./xchonnectLinkHandoff');
    resetXchonnectLinkHandoff();
    setXchonnectNavigator(() => undefined);
    offerXchonnectLink('https://clapandpay.com/req#mbx=bWFpbA');

    renderScreen(<XchonnectApproval />);

    await waitFor(() => expect(listSessions).toHaveBeenCalled());
    expect(screen.getByTestId('nothing-waiting').textContent).toContain(
      'nothing has arrived yet',
    );
  });
});

describe('a message request', () => {
  it('shows the message in its own labelled block', async () => {
    pendingApprovals.mockResolvedValue([
      request(
        {
          type: 'sign_message',
          dapp: 'pengui.xyz',
          message_hex: '48656c6c6f',
          message_text: 'Hello from Pengui',
          public_key: KEY_A.slice(2),
        },
        'm1',
      ),
    ]);
    renderScreen(<XchonnectApproval />);

    await waitFor(() =>
      expect(screen.getByTestId('message-body').textContent).toBe(
        'Hello from Pengui',
      ),
    );
    expect(screen.getByText(/asks you to sign this text/)).toBeDefined();
    expect(screen.getByText(/moves no money/)).toBeDefined();
    // No amount line is invented for a message.
    expect(screen.queryByTestId('asset-net')).toBeNull();
  });
});
