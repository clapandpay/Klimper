// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The connected-websites list (clapandpay TASK-084).

import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { SessionSummary } from './xchonnect';

vi.mock('@/components/Layout', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('@/components/Header', () => ({
  default: ({ title, children }: { title: string; children?: unknown }) => (
    <h1>
      {title}
      {children as never}
    </h1>
  ),
}));
vi.mock('@/hooks/useBiometric', () => ({
  useBiometric: () => ({ promptIfEnabled: () => Promise.resolve(true) }),
}));
vi.mock('@/lib/features', () => ({ xchonnectEnabled: true }));

const listSessions = vi.fn();
const endSession = vi.fn();
const rotateSessionKeys = vi.fn();
const pendingApprovals = vi.fn();
const pushStatus = vi.fn();

vi.mock('./xchonnect', () => ({
  xchonnect: {
    status: () =>
      Promise.resolve({
        developerModeAllowed: true,
        fingerprint: 1,
        chainId: 'mainnet',
        sessionCount: 0,
        keyringService: 'com.clapandpay.klimper.xchonnect',
      }),
    listSessions: () => listSessions(),
    endSession: (id: string) => endSession(id),
    rotateSessionKeys: (id: string) => rotateSessionKeys(id),
    pendingApprovals: () => pendingApprovals(),
    pushStatus: () => pushStatus(),
  },
  XCHONNECT_EVENTS: {
    sessions: 'sessions',
    session: 'session',
    approval: 'approval',
    approvalResolved: 'approval-resolved',
    push: 'push',
  },
  onXchonnectEvent: () => Promise.resolve(() => undefined),
}));

const { renderScreen } = await import('../testHarness');
const XchonnectDapps = (await import('./XchonnectDapps')).default;

const session: SessionSummary = {
  sessionId: 'sess-1',
  dapp: 'pengui.xyz',
  domainUnicode: 'pengui.xyz',
  domainDiffers: false,
  warnings: [],
  dappName: 'Pengui',
  chainId: 'mainnet',
  createdAt: 1_760_000_000,
  epoch: 2,
  sasConfirmed: true,
  needsRepair: false,
  revoked: false,
  rotating: false,
  developerMode: false,
  methods: ['chainId', 'connect', 'signCoinSpends'],
  exposedKeys: [`0x${'a'.repeat(96)}`],
  limits: {},
  optionLimit: { perRequest: 1, perDay: 5 },
  allowAggSigUnsafe: false,
  allowUnknownContracts: false,
  allReceiveKeys: false,
  connected: true,
};

beforeEach(() => {
  vi.clearAllMocks();
  pendingApprovals.mockResolvedValue([]);
  pushStatus.mockResolvedValue({ enabled: false, permission: null });
  endSession.mockResolvedValue(null);
  rotateSessionKeys.mockResolvedValue(null);
});

afterEach(cleanup);

it('explains how to connect when nothing is connected', async () => {
  listSessions.mockResolvedValue([]);
  renderScreen(<XchonnectDapps />);

  await waitFor(() => expect(screen.getByTestId('empty-state')).toBeDefined());
  expect(screen.getByTestId('empty-state').textContent).toMatch(
    /press its connect button, and scan or paste the code/,
  );
});

it('lists a session with its domain, what it may call and whether it is listening', async () => {
  listSessions.mockResolvedValue([session]);
  renderScreen(<XchonnectDapps />);

  await waitFor(() => expect(screen.getByTestId('dapp-card')).toBeDefined());
  const card = screen.getByTestId('dapp-card').textContent ?? '';
  expect(card).toContain('pengui.xyz');
  expect(card).toContain('chainId, connect, signCoinSpends');
  expect(card).toContain('listening for requests');
  expect(card).toContain('renewed 2 times');
  expect(card).toContain('none set');
});

it('shows both domain forms and every warning on the card', async () => {
  listSessions.mockResolvedValue([
    {
      ...session,
      domainUnicode: 'pengúi.xyz',
      domainDiffers: true,
      warnings: ['NonAscii'],
    },
  ]);
  renderScreen(<XchonnectDapps />);

  await waitFor(() =>
    expect(screen.getByTestId('domain-ascii').textContent).toContain(
      'pengui.xyz',
    ),
  );
  expect(screen.getByTestId('domain-warning')).toBeDefined();
});

it('marks a session that lost its state as unusable', async () => {
  listSessions.mockResolvedValue([
    { ...session, needsRepair: true, connected: false },
  ]);
  renderScreen(<XchonnectDapps />);

  await waitFor(() => expect(screen.getByTestId('needs-repair')).toBeDefined());
  expect(screen.getByTestId('needs-repair').textContent).toMatch(
    /cannot be used/,
  );
});

it('marks a session whose dangerous permissions are on', async () => {
  listSessions.mockResolvedValue([
    { ...session, allowAggSigUnsafe: true, allowUnknownContracts: true },
  ]);
  renderScreen(<XchonnectDapps />);

  await waitFor(() =>
    expect(screen.getByTestId('dangerous-switch-warning')).toBeDefined(),
  );
  expect(screen.getByTestId('dangerous-switch-warning').textContent).toMatch(
    /unsafe signatures and contracts Klimper cannot read/,
  );
});

it('ends a connection without asking twice', async () => {
  listSessions.mockResolvedValue([session]);
  renderScreen(<XchonnectDapps />);

  await waitFor(() => screen.getByRole('button', { name: 'End connection' }));
  listSessions.mockResolvedValue([]);
  fireEvent.click(screen.getByRole('button', { name: 'End connection' }));

  await waitFor(() => expect(endSession).toHaveBeenCalledWith('sess-1'));
  await waitFor(() => expect(screen.getByTestId('empty-state')).toBeDefined());
});

it('renews the keys of a live connection', async () => {
  listSessions.mockResolvedValue([session]);
  renderScreen(<XchonnectDapps />);

  await waitFor(() => screen.getByRole('button', { name: 'Renew keys' }));
  listSessions.mockResolvedValue([{ ...session, rotating: true }]);
  fireEvent.click(screen.getByRole('button', { name: 'Renew keys' }));

  await waitFor(() => expect(rotateSessionKeys).toHaveBeenCalledWith('sess-1'));
  await waitFor(() => expect(screen.getByText('being renewed')).toBeDefined());
  expect(
    (screen.getByRole('button', { name: 'Renew keys' }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
});

it('cannot renew a connection that is not listening', async () => {
  listSessions.mockResolvedValue([{ ...session, connected: false }]);
  renderScreen(<XchonnectDapps />);

  await waitFor(() => screen.getByRole('button', { name: 'Renew keys' }));
  expect(
    (screen.getByRole('button', { name: 'Renew keys' }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
});

it('marks a revoked connection and only offers to remove it', async () => {
  listSessions.mockResolvedValue([
    { ...session, revoked: true, connected: false },
  ]);
  renderScreen(<XchonnectDapps />);

  await waitFor(() => expect(screen.getByTestId('revoked')).toBeDefined());
  expect(screen.getByTestId('revoked').textContent).toMatch(
    /withdrew the key this connection was made with/,
  );
  expect(screen.queryByRole('button', { name: 'Renew keys' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'What it may do' })).toBeNull();
  listSessions.mockResolvedValue([]);
  fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
  await waitFor(() => expect(endSession).toHaveBeenCalledWith('sess-1'));
});

it('points at the approval screen when something is waiting', async () => {
  listSessions.mockResolvedValue([session]);
  pendingApprovals.mockResolvedValue([
    {
      id: 'a1',
      sessionId: 'sess-1',
      dapp: 'pengui.xyz',
      method: 'signCoinSpends',
      requestId: 'r1',
      expiresAt: 0,
      kind: 'request',
      prompt: {},
    },
  ]);
  renderScreen(<XchonnectDapps />);

  await waitFor(() =>
    expect(screen.getByTestId('waiting-banner')).toBeDefined(),
  );
});

it('says where the connection keys live, and that it is not the signing keychain', async () => {
  listSessions.mockResolvedValue([]);
  renderScreen(<XchonnectDapps />);

  await waitFor(() =>
    expect(
      screen.getByText(/com\.clapandpay\.klimper\.xchonnect/),
    ).toBeDefined(),
  );
  expect(
    screen.getByText(/separate from the keys that sign your money/),
  ).toBeDefined();
});

it('opens on the connection that was just made, with the way back to the wallet', async () => {
  listSessions.mockResolvedValue([session]);
  renderScreen(<XchonnectDapps />, { state: { connected: 'sess-1' } });

  await waitFor(() =>
    expect(screen.getByTestId('connected-banner')).toBeDefined(),
  );
  expect(screen.getByTestId('connected-banner').textContent).toContain(
    'Connected to pengui.xyz',
  );
  expect(
    screen.getByRole('button', { name: 'Back to my wallet' }),
  ).toBeDefined();
});

it('shows no banner for a connection that is not in the list', async () => {
  listSessions.mockResolvedValue([session]);
  renderScreen(<XchonnectDapps />, { state: { connected: 'someone-else' } });

  await waitFor(() => expect(screen.getByTestId('dapp-card')).toBeDefined());
  expect(screen.queryByTestId('connected-banner')).toBeNull();
});

it('says so when notifications are refused and wake-ups are built in', async () => {
  listSessions.mockResolvedValue([session]);
  pushStatus.mockResolvedValue({ enabled: true, permission: 'denied' });
  renderScreen(<XchonnectDapps />);

  await waitFor(() => expect(screen.getByTestId('push-denied')).toBeDefined());
  expect(screen.getByTestId('push-denied').textContent).toMatch(
    /only sees a request from a website while it is open/,
  );
});

it.each([
  ['granted', true],
  ['undetermined', true],
  ['denied', false],
])(
  'stays quiet about notifications when they are %s (wake-ups built in: %s)',
  async (permission, enabled) => {
    listSessions.mockResolvedValue([session]);
    pushStatus.mockResolvedValue({ enabled, permission });
    renderScreen(<XchonnectDapps />);

    await waitFor(() => expect(screen.getByTestId('dapp-card')).toBeDefined());
    // A build without a gateway has no wake-ups to explain, whatever the phone says.
    expect(screen.queryByTestId('push-denied')).toBeNull();
  },
);
