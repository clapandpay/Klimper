// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The entry point in Settings (clapandpay TASK-072, TASK-084).
//
// The flag is the whole security argument for shipping this before TASK-094 has reviewed
// it: a build that did not ask for Xchonnect must not call the plugin at all.

import { cleanup, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';

vi.mock('@/lib/features', () => ({ xchonnectEnabled: false }));

const listSessions = vi.fn().mockResolvedValue([]);
const status = vi.fn().mockResolvedValue({
  developerModeAllowed: false,
  fingerprint: 1,
  chainId: 'mainnet',
  sessionCount: 0,
  keyringService: 'com.clapandpay.klimper.xchonnect',
});

vi.mock('./xchonnect', () => ({
  xchonnect: { status: () => status(), listSessions: () => listSessions() },
  XCHONNECT_EVENTS: { sessions: 'sessions', session: 'session' },
  onXchonnectEvent: () => Promise.resolve(() => undefined),
}));

const { renderScreen } = await import('./testHarness');
const { XchonnectSettings } = await import('./XchonnectSettings');

afterEach(cleanup);

it('renders nothing and calls nothing when the feature is off', async () => {
  const { container } = renderScreen(<XchonnectSettings />);

  expect(container.textContent).toBe('');
  // Not even a status read: the flag is checked before any hook runs.
  await waitFor(() => expect(status).not.toHaveBeenCalled());
  expect(listSessions).not.toHaveBeenCalled();
  expect(screen.queryByText('Connected websites')).toBeNull();
});
