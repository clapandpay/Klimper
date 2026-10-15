// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The approval overlay and the requests page (clapandpay TASK-080). The rules:
//
//   * a request that arrives is shown at once, over whatever screen is open — before, it
//     waited unseen on the approval bridge and was refused after five minutes;
//   * the app starts fetching requests by itself, not only once an Xchonnect screen opens;
//   * "Later" hides the overlay without answering, and a new request brings it back;
//   * the requests page lists everything waiting, and any one can be picked, but Approve
//     exists only in the full view of the one picked.

import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Approval } from './xchonnect';

vi.mock('@/components/Layout', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('@/components/Header', () => ({
  default: ({ title }: { title: string }) => <h1>{title}</h1>,
}));
// Sage's "Require biometrics": confirmed unless a test says otherwise.
const promptIfEnabled = vi.fn().mockResolvedValue(true);
vi.mock('@/hooks/useBiometric', () => ({
  useBiometric: () => ({ promptIfEnabled: () => promptIfEnabled() }),
}));
vi.mock('@/lib/features', () => ({ xchonnectEnabled: true }));
// The wallet's default fee, as the user set it in the wallet's settings.
vi.mock('@/hooks/useDefaultFee', () => ({
  useDefaultFee: () => ({ fee: '0.0001', setFee: () => undefined }),
}));
vi.mock('../useTicker', () => ({ useTicker: () => 'XCH' }));
const openUrl = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl }));
vi.mock('@tauri-apps/plugin-os', () => ({ platform: () => 'macos' }));
const setFocus = vi.fn().mockResolvedValue(undefined);
vi.mock('@tauri-apps/api/window', () => ({
  UserAttentionType: { Critical: 1, Informational: 2 },
  getCurrentWindow: () => ({
    unminimize: vi.fn().mockResolvedValue(undefined),
    show: vi.fn().mockResolvedValue(undefined),
    setFocus,
    requestUserAttention: vi.fn().mockResolvedValue(undefined),
  }),
}));
vi.mock('react-toastify', () => ({
  toast: { success: vi.fn(), info: vi.fn() },
}));
vi.mock('theme-o-rama', () => ({
  useTheme: () => ({ currentTheme: { mostLike: 'light' } }),
}));

const pendingApprovals = vi.fn();
const resolveApproval = vi.fn().mockResolvedValue(null);
const listSessions = vi.fn().mockResolvedValue([]);
const setOwnFee = vi.fn().mockResolvedValue(null);
const handlers = new Map<string, (payload: unknown) => void>();

vi.mock('./xchonnect', () => ({
  xchonnect: {
    pendingApprovals: () => pendingApprovals(),
    resolveApproval: (id: string, approved: boolean) =>
      resolveApproval(id, approved),
    listSessions: () => listSessions(),
    setOwnFee: (mojos: string) => setOwnFee(mojos),
  },
  XCHONNECT_EVENTS: {
    approval: 'approval',
    approvalResolved: 'approval-resolved',
    sent: 'sent',
  },
  // Two hooks can listen at once (overlay and page), so keep every handler.
  onXchonnectEvent: (event: string, handler: (payload: unknown) => void) => {
    const previous = handlers.get(event);
    handlers.set(event, (payload) => {
      previous?.(payload);
      handler(payload);
    });
    return Promise.resolve(() => undefined);
  },
}));

const { renderScreen } = await import('../testHarness');
const { XchonnectApprovalHost } = await import('./XchonnectApprovalHost');
const XchonnectApproval = (await import('./XchonnectApproval')).default;
const { setXchonnectNavigator } = await import('./xchonnectLinkHandoff');

function spend(net: number) {
  return {
    type: 'sign_coin_spends',
    dapp: 'pengui.xyz',
    summary: {
      assets: [
        {
          asset: { type: 'xch' },
          sent: -net,
          received: 0,
          conditional_received: 0,
          net,
        },
      ],
      implied_fee: 0,
      reserve_fee: 0,
      time_locks: {},
      unknown_puzzles: [],
      cost: 1,
      spends: [],
    },
    plan: { ours: [], foreign: [], has_unsafe: false },
    partial: false,
    binding: null,
  };
}

function request(id: string, net: number, dapp = 'pengui.xyz'): Approval {
  return {
    id,
    sessionId: 's1',
    dapp,
    method: 'signCoinSpends',
    requestId: `r-${id}`,
    expiresAt: Math.floor(Date.now() / 1000) + 300,
    kind: 'request',
    prompt: spend(net) as Record<string, unknown>,
  };
}

const navigations: string[] = [];

/** Let the first `pendingApprovals()` load finish, so a later event is not overwritten. */
const settle = () => act(() => Promise.resolve());

beforeEach(() => {
  handlers.clear();
  vi.clearAllMocks();
  navigations.length = 0;
  setXchonnectNavigator((route) => navigations.push(route));
  resolveApproval.mockResolvedValue(null);
  listSessions.mockResolvedValue([]);
  pendingApprovals.mockResolvedValue([]);
  window.location.hash = '#/wallet';
});

afterEach(cleanup);

describe('the overlay', () => {
  it('starts fetching requests when the app starts', async () => {
    renderScreen(<XchonnectApprovalHost />);
    await waitFor(() => expect(listSessions).toHaveBeenCalled());
  });

  it("hands Klimper the wallet's default fee, in mojos, for what it builds itself", async () => {
    renderScreen(<XchonnectApprovalHost />);
    await waitFor(() => expect(setOwnFee).toHaveBeenCalledWith('100000000'));
  });

  it('shows a request that arrives, over any screen, in the full view', async () => {
    renderScreen(<XchonnectApprovalHost />);
    expect(screen.queryByTestId('approval-overlay')).toBeNull();
    await settle();

    handlers.get('approval')?.(request('a1', -1_500_000_000_000));

    const overlay = await waitFor(() => screen.getByTestId('approval-overlay'));
    expect(overlay.textContent).toContain('pengui.xyz asks you to sign');
    expect(screen.getByTestId('asset-net').textContent).toBe('−1.5 XCH');
    expect(screen.getByTestId('approval-countdown').textContent).toMatch(
      /refused by itself in [0-9]:[0-5][0-9]/,
    );
  });

  it('answers the request it shows, and only that one', async () => {
    pendingApprovals.mockResolvedValue([
      request('a1', -1_000_000_000_000),
      request('a2', -2_000_000_000_000),
    ]);
    renderScreen(<XchonnectApprovalHost />);

    await waitFor(() => screen.getByTestId('approval-overlay'));
    expect(screen.getByTestId('queue-count').textContent).toContain(
      '+1 waiting',
    );
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }));

    await waitFor(() =>
      expect(resolveApproval).toHaveBeenCalledWith('a1', true),
    );
    expect(resolveApproval).toHaveBeenCalledTimes(1);
    // The next one is now on screen, unanswered.
    await waitFor(() =>
      expect(screen.getByTestId('asset-net').textContent).toBe('−2 XCH'),
    );
  });

  it('asks for Face ID before approving, and signs nothing without it (TASK-126 audit)', async () => {
    pendingApprovals.mockResolvedValue([request('a1', -1_000_000_000_000)]);
    promptIfEnabled.mockResolvedValueOnce(false);
    renderScreen(<XchonnectApprovalHost />);

    await waitFor(() => screen.getByTestId('approval-overlay'));
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }));

    await waitFor(() => expect(promptIfEnabled).toHaveBeenCalledTimes(1));
    expect(resolveApproval).not.toHaveBeenCalled();
    // Still waiting: the person can approve again and confirm this time.
    expect(screen.getByTestId('approval-overlay')).toBeTruthy();
    await waitFor(() =>
      expect(
        (screen.getByRole('button', { name: 'Approve' }) as HTMLButtonElement)
          .disabled,
      ).toBe(false),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }));
    await waitFor(() =>
      expect(resolveApproval).toHaveBeenCalledWith('a1', true),
    );
  });

  it('rejects without asking for Face ID', async () => {
    pendingApprovals.mockResolvedValue([request('a1', -1_000_000_000_000)]);
    renderScreen(<XchonnectApprovalHost />);

    await waitFor(() => screen.getByTestId('approval-overlay'));
    fireEvent.click(screen.getByRole('button', { name: 'Reject' }));
    await waitFor(() =>
      expect(resolveApproval).toHaveBeenCalledWith('a1', false),
    );
    expect(promptIfEnabled).not.toHaveBeenCalled();
  });

  it('puts a request off with Later, without answering it, and a new one reopens', async () => {
    pendingApprovals.mockResolvedValue([request('a1', -1_000_000_000_000)]);
    renderScreen(<XchonnectApprovalHost />);

    await waitFor(() => screen.getByTestId('approval-overlay'));
    fireEvent.click(screen.getByTestId('approval-later'));

    await waitFor(() =>
      expect(screen.queryByTestId('approval-overlay')).toBeNull(),
    );
    expect(resolveApproval).not.toHaveBeenCalled();
    expect(screen.getByTestId('approval-waiting-pill').textContent).toContain(
      '1 request waiting',
    );

    handlers.get('approval')?.(request('a2', -3_000_000_000_000));
    await waitFor(() => screen.getByTestId('approval-overlay'));
  });

  it('reopens from the waiting pill', async () => {
    pendingApprovals.mockResolvedValue([request('a1', -1_000_000_000_000)]);
    renderScreen(<XchonnectApprovalHost />);

    await waitFor(() => screen.getByTestId('approval-overlay'));
    fireEvent.click(screen.getByTestId('approval-later'));
    fireEvent.click(
      await waitFor(() => screen.getByTestId('approval-waiting-pill')),
    );

    await waitFor(() => screen.getByTestId('approval-overlay'));
  });

  it('leads to the requests page', async () => {
    pendingApprovals.mockResolvedValue([
      request('a1', -1_000_000_000_000),
      request('a2', -2_000_000_000_000),
    ]);
    renderScreen(<XchonnectApprovalHost />);

    fireEvent.click(
      await waitFor(() => screen.getByTestId('approval-see-all')),
    );
    expect(navigations).toEqual(['/xchonnect/request/a1']);
  });

  it('stays out of the way on the requests page, which shows the same view', async () => {
    window.location.hash = '#/xchonnect/request';
    pendingApprovals.mockResolvedValue([request('a1', -1_000_000_000_000)]);
    renderScreen(<XchonnectApprovalHost />);

    await waitFor(() => expect(pendingApprovals).toHaveBeenCalled());
    expect(screen.queryByTestId('approval-overlay')).toBeNull();
    expect(screen.queryByTestId('approval-waiting-pill')).toBeNull();
  });

  it('brings the desktop window forward when it is not in front', async () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    renderScreen(<XchonnectApprovalHost />);
    await settle();

    handlers.get('approval')?.(request('a1', -1_000_000_000_000));
    await waitFor(() => expect(setFocus).toHaveBeenCalled());
  });
});

describe('a transaction Klimper sent', () => {
  it('is announced with a link to look it up on mempoolxch.space', async () => {
    const { toast } = await import('react-toastify');
    renderScreen(<XchonnectApprovalHost />);
    await settle();

    const id = 'ef'.repeat(32);
    handlers.get('sent')?.({
      dapp: 'pengui.space',
      method: 'chia_send',
      transactionId: `0x${id}`,
      status: 'pending',
    });
    await waitFor(() => expect(toast.success).toHaveBeenCalled());
    const element = vi.mocked(toast.success).mock.calls.at(-1)?.[0];
    // As Sage's toast container renders it: outside I18nProvider, where `Trans` throws
    // and blacked out the app (TASK-105).
    render(<>{element as React.ReactNode}</>);
    expect(screen.getByTestId('sent-toast').textContent).toContain(
      'pengui.space',
    );
    fireEvent.click(screen.getByTestId('sent-explorer-link'));
    expect(openUrl).toHaveBeenCalledWith(`https://mempoolxch.space/tx/0x${id}`);
  });
});

describe('the requests page', () => {
  it('lists everything waiting, with no amount and no Approve in the list', async () => {
    pendingApprovals.mockResolvedValue([
      request('a1', -1_000_000_000_000),
      request('a2', -2_000_000_000_000, 'dexie.space'),
    ]);
    renderScreen(<XchonnectApproval />, {
      path: '/xchonnect/request',
      route: '/xchonnect/request',
    });

    const list = await waitFor(() => screen.getByTestId('request-list'));
    const rows = screen.getAllByTestId('request-row');
    expect(rows).toHaveLength(2);
    expect(list.textContent).toContain('dexie.space');
    expect(list.textContent).not.toContain('XCH');
    expect(list.querySelectorAll('button')).toHaveLength(2);
    // One Approve on the page: the one in the full view.
    expect(screen.getAllByRole('button', { name: 'Approve' })).toHaveLength(1);
  });

  it('answers the one picked, out of order, from its full view', async () => {
    pendingApprovals.mockResolvedValue([
      request('a1', -1_000_000_000_000),
      request('a2', -2_000_000_000_000, 'dexie.space'),
    ]);
    renderScreen(<XchonnectApproval />, {
      path: '/xchonnect/request/:approvalId',
      route: '/xchonnect/request/a2',
    });

    await waitFor(() =>
      expect(screen.getByTestId('approval-title').textContent).toBe(
        'dexie.space asks you to sign',
      ),
    );
    expect(screen.getByTestId('asset-net').textContent).toBe('−2 XCH');

    fireEvent.click(screen.getByRole('button', { name: 'Reject' }));
    await waitFor(() =>
      expect(resolveApproval).toHaveBeenCalledWith('a2', false),
    );
    expect(resolveApproval).toHaveBeenCalledTimes(1);
  });

  it('falls back to the oldest when the picked one is no longer waiting', async () => {
    pendingApprovals.mockResolvedValue([request('a1', -1_000_000_000_000)]);
    renderScreen(<XchonnectApproval />, {
      path: '/xchonnect/request/:approvalId',
      route: '/xchonnect/request/gone',
    });

    await waitFor(() =>
      expect(screen.getByTestId('asset-net').textContent).toBe('−1 XCH'),
    );
  });
});
