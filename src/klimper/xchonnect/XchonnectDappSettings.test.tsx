// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// Per-dApp permissions and limits (clapandpay TASK-083, spec 9.3).
//
// What is checked here: the spec's defaults are what a new connection shows, a dangerous
// permission cannot be turned on without its own confirmation, and a limit the plugin
// could not store exactly is refused rather than silently rounded.

import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from './xchonnect';

vi.mock('@/components/Layout', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('@/components/Header', () => ({
  default: ({ title }: { title: string }) => <h1>{title}</h1>,
}));
vi.mock('@/lib/features', () => ({ xchonnectEnabled: true }));
vi.mock('../useTicker', () => ({ useTicker: () => 'XCH' }));
vi.mock('theme-o-rama', () => ({
  useTheme: () => ({ currentTheme: { mostLike: 'light' } }),
}));

const listSessions = vi.fn();
const setDappPolicy = vi.fn();

vi.mock('./xchonnect', () => ({
  xchonnect: {
    status: () =>
      Promise.resolve({
        developerModeAllowed: true,
        fingerprint: 1,
        chainId: 'mainnet',
        sessionCount: 1,
        keyringService: 'com.clapandpay.klimper.xchonnect',
      }),
    listSessions: () => listSessions(),
    setDappPolicy: (id: string, policy: unknown) => setDappPolicy(id, policy),
  },
  XCHONNECT_EVENTS: { sessions: 'sessions', session: 'session' },
  onXchonnectEvent: () => Promise.resolve(() => undefined),
}));

const { renderScreen } = await import('../testHarness');
const XchonnectDappSettings = (await import('./XchonnectDappSettings')).default;
const { DEFAULT_METHODS, parseLimit } = await import('./XchonnectDappSettings');

/** A session exactly as pairing creates it (spec 9.3 defaults). */
const fresh: SessionSummary = {
  sessionId: 'sess-1',
  dapp: 'pengui.xyz',
  domainUnicode: 'pengui.xyz',
  domainDiffers: false,
  warnings: [],
  dappName: 'Pengui',
  chainId: 'mainnet',
  createdAt: 1_760_000_000,
  epoch: 0,
  sasConfirmed: true,
  needsRepair: false,
  revoked: false,
  rotating: false,
  developerMode: false,
  methods: [...DEFAULT_METHODS],
  exposedKeys: [`0x${'a'.repeat(96)}`],
  limits: {},
  allowAggSigUnsafe: false,
  allowUnknownContracts: false,
  allReceiveKeys: false,
  connected: true,
};

function openSettings(session: SessionSummary = fresh) {
  listSessions.mockResolvedValue([session]);
  return renderScreen(<XchonnectDappSettings />, {
    path: '/xchonnect/dapp/:sessionId',
    route: `/xchonnect/dapp/${session.sessionId}`,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  setDappPolicy.mockResolvedValue(fresh);
});

afterEach(cleanup);

describe('the defaults of a new connection', () => {
  it('is the default methods, one key, no limits and both switches off', async () => {
    openSettings();

    await waitFor(() =>
      expect(screen.getByLabelText('signCoinSpends')).toBeDefined(),
    );

    // Spec 9.1's required set, plus the two methods where Klimper signs and sends
    // (xchonnect-wiki 04). Each still asks.
    expect([...DEFAULT_METHODS]).toEqual([
      'chainId',
      'connect',
      'getPublicKeys',
      'signCoinSpends',
      'signMessage',
      'xchonnect_submitCoinSpends',
      'chia_send',
    ]);
    for (const method of DEFAULT_METHODS) {
      expect(screen.getByLabelText(method).getAttribute('data-state')).toBe(
        'checked',
      );
    }
    // chainId and connect cannot be taken away; the rest can.
    expect(screen.getByLabelText('chainId')).toHaveProperty('disabled', true);
    expect(screen.getByLabelText('signCoinSpends')).toHaveProperty(
      'disabled',
      false,
    );

    // One key of its own: it does not see the receive addresses.
    expect(
      screen
        .getByLabelText('Can see my receive addresses')
        .getAttribute('data-state'),
    ).toBe('unchecked');

    // No limits: both XCH fields are empty.
    expect(screen.getByLabelText('Per request')).toHaveProperty('value', '');
    expect(screen.getByLabelText('Per day')).toHaveProperty('value', '');

    // Both dangerous switches off.
    expect(
      screen
        .getByLabelText(/Allow unsafe signatures/)
        .getAttribute('data-state'),
    ).toBe('unchecked');
    expect(
      screen
        .getByLabelText(/Allow contracts Klimper cannot read/)
        .getAttribute('data-state'),
    ).toBe('unchecked');
  });

  it('says that a limit refuses a request before it is ever shown', async () => {
    openSettings();
    await waitFor(() =>
      expect(screen.getByText(/refused before it reaches you/)).toBeDefined(),
    );
    expect(
      screen.getByText(/will not see a signing prompt for it/),
    ).toBeDefined();
  });
});

describe('the addresses the website sees', () => {
  it('is one switch, the explanation behind an info dot, no full key on screen', async () => {
    openSettings();
    await waitFor(() => screen.getByTestId('address-switch'));
    expect(document.body.textContent).not.toContain('a'.repeat(96));
    const row = screen.getByTestId('address-switch');
    expect(row.querySelector('button.klimper-hint')).not.toBeNull();
  });

  it('turns on at once: every request still needs approval', async () => {
    openSettings();
    await waitFor(() => screen.getByLabelText('Can see my receive addresses'));
    fireEvent.click(screen.getByLabelText('Can see my receive addresses'));
    expect(screen.queryByTestId('dangerous-confirm')).toBeNull();
    await waitFor(() =>
      expect(setDappPolicy).toHaveBeenCalledWith('sess-1', {
        allReceiveKeys: true,
      }),
    );
  });
});

describe('a dangerous permission needs its own confirmation', () => {
  it('does not turn on unsafe signatures until the dialog is accepted', async () => {
    openSettings();
    await waitFor(() => screen.getByLabelText(/Allow unsafe signatures/));

    fireEvent.click(screen.getByLabelText(/Allow unsafe signatures/));

    // Nothing was written by the switch itself.
    expect(setDappPolicy).not.toHaveBeenCalled();
    const dialog = await waitFor(() => screen.getByTestId('dangerous-confirm'));
    expect(dialog.textContent).toMatch(
      /reused in a completely different spend/,
    );

    fireEvent.click(
      screen.getByRole('button', { name: 'I understand, turn it on' }),
    );
    await waitFor(() =>
      expect(setDappPolicy).toHaveBeenCalledWith('sess-1', {
        allowAggSigUnsafe: true,
      }),
    );
  });

  it('leaves it off when the dialog is refused', async () => {
    openSettings();
    await waitFor(() => screen.getByLabelText(/Allow unsafe signatures/));

    fireEvent.click(screen.getByLabelText(/Allow unsafe signatures/));
    await waitFor(() => screen.getByTestId('dangerous-confirm'));
    fireEvent.click(screen.getByRole('button', { name: 'Leave it off' }));

    expect(setDappPolicy).not.toHaveBeenCalled();
  });

  it('asks separately for unreadable contracts', async () => {
    openSettings();
    await waitFor(() =>
      screen.getByLabelText(/Allow contracts Klimper cannot read/),
    );

    fireEvent.click(
      screen.getByLabelText(/Allow contracts Klimper cannot read/),
    );
    const dialog = await waitFor(() => screen.getByTestId('dangerous-confirm'));
    expect(dialog.textContent).toMatch(
      /Nobody can tell you what such a contract does/,
    );

    fireEvent.click(
      screen.getByRole('button', { name: 'I understand, turn it on' }),
    );
    await waitFor(() =>
      expect(setDappPolicy).toHaveBeenCalledWith('sess-1', {
        allowUnknownContracts: true,
      }),
    );
  });

  it('turns one off without a dialog, because that is the safe direction', async () => {
    openSettings({ ...fresh, allowAggSigUnsafe: true });
    await waitFor(() => screen.getByLabelText(/Allow unsafe signatures/));

    fireEvent.click(screen.getByLabelText(/Allow unsafe signatures/));

    await waitFor(() =>
      expect(setDappPolicy).toHaveBeenCalledWith('sess-1', {
        allowAggSigUnsafe: false,
      }),
    );
    expect(screen.queryByTestId('dangerous-confirm')).toBeNull();
  });
});

describe('limits', () => {
  it('reads an existing limit back in the asset’s own units', async () => {
    openSettings({
      ...fresh,
      limits: {
        xch: { perRequest: 1_500_000_000_000, perDay: 5_000_000_000_000 },
      },
    });
    await waitFor(() =>
      expect(screen.getByLabelText('Per request')).toHaveProperty(
        'value',
        '1.5',
      ),
    );
    expect(screen.getByLabelText('Per day')).toHaveProperty('value', '5');
  });

  it('writes what the user typed as whole mojos', async () => {
    openSettings();
    await waitFor(() => screen.getByLabelText('Per request'));

    fireEvent.change(screen.getByLabelText('Per request'), {
      target: { value: '0.25' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() =>
      expect(setDappPolicy).toHaveBeenCalledWith('sess-1', {
        limits: { xch: { perRequest: 250_000_000_000 } },
        methods: [...DEFAULT_METHODS],
      }),
    );
  });

  it('refuses a limit it cannot store exactly instead of rounding it', async () => {
    openSettings();
    await waitFor(() => screen.getByLabelText('Per day'));

    fireEvent.change(screen.getByLabelText('Per day'), {
      target: { value: '1000000' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() =>
      expect(
        screen.getByText(/too large for Klimper to store exactly/),
      ).toBeDefined(),
    );
    expect(setDappPolicy).not.toHaveBeenCalled();
  });

  it('refuses something that is not an amount', async () => {
    openSettings();
    await waitFor(() => screen.getByLabelText('Per day'));

    fireEvent.change(screen.getByLabelText('Per day'), {
      target: { value: 'as much as you like' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() =>
      expect(screen.getByText(/has to be a plain amount/)).toBeDefined(),
    );
    expect(setDappPolicy).not.toHaveBeenCalled();
  });
});

describe('parseLimit', () => {
  it('reads an empty field as no limit', () => {
    expect(parseLimit('', 12)).toEqual({ kind: 'none' });
    expect(parseLimit('   ', 12)).toEqual({ kind: 'none' });
  });

  it('converts to the smallest unit', () => {
    expect(parseLimit('1.5', 12)).toEqual({
      kind: 'amount',
      mojos: 1_500_000_000_000,
    });
    expect(parseLimit('10', 3)).toEqual({ kind: 'amount', mojos: 10_000 });
    expect(parseLimit('0', 12)).toEqual({ kind: 'amount', mojos: 0 });
  });

  it('refuses more precision than the asset has', () => {
    expect(parseLimit('1.0005', 3)).toEqual({
      kind: 'invalid',
      reason: 'notANumber',
    });
  });

  it('refuses a negative limit and nonsense', () => {
    expect(parseLimit('-5', 12).kind).toBe('invalid');
    expect(parseLimit('lots', 12).kind).toBe('invalid');
  });

  it('refuses number syntax that is not a plain amount', () => {
    for (const text of ['1e3', '0x10', '.5', '1,5', '+1', 'Infinity']) {
      expect(parseLimit(text, 12), text).toEqual({
        kind: 'invalid',
        reason: 'notANumber',
      });
    }
    expect(parseLimit('1.', 12)).toEqual({ kind: 'amount', mojos: 1e12 });
  });

  it('refuses an amount that a JSON number cannot carry exactly', () => {
    // StoredLimit is a u128 in Rust but crosses as a JSON number, so above 2^53 the
    // value the plugin stores would not be the value the user typed.
    expect(parseLimit('9007200', 12)).toEqual({
      kind: 'invalid',
      reason: 'tooLarge',
    });
  });
});
