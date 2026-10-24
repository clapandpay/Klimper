// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The card details (TASK-153): only after biometrics, hidden again after a minute, when
// hidden by hand, and when Klimper goes to the background.

import {
  act,
  cleanup,
  fireEvent,
  screen,
  waitFor,
} from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const prompt = vi.hoisted(() => vi.fn());
vi.mock('@/hooks/useBiometric', () => ({
  useBiometric: () => ({ promptIfEnabled: prompt }),
}));
const api = vi.hoisted(() => ({ details: vi.fn() }));
vi.mock('./card', async (original) => ({
  ...(await original<typeof import('./card')>()),
  card: api,
}));

const { renderScreen } = await import('../testHarness');
const { CardDetails, DETAILS_VISIBLE_MS, groupNumber } =
  await import('./CardDetails');

const DETAILS = { number: '4000000000004242', cvv: '737', expires: '10/29' };

beforeEach(() => {
  vi.resetAllMocks();
  prompt.mockResolvedValue(true);
  api.details.mockResolvedValue(DETAILS);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it('groups the number in fours', () => {
  expect(groupNumber('4000000000004242')).toBe('4000 0000 0000 4242');
});

it('shows the details after biometrics, for the given card', async () => {
  renderScreen(<CardDetails cardId='card-1' />);
  fireEvent.click(screen.getByTestId('card-show-details'));
  expect((await screen.findByTestId('card-number')).textContent).toBe(
    '4000 0000 0000 4242',
  );
  expect(screen.getByTestId('card-cvv').textContent).toBe('737');
  expect(screen.getByTestId('card-expires').textContent).toBe('10/29');
  expect(api.details).toHaveBeenCalledWith('card-1');
  expect(prompt.mock.invocationCallOrder[0]).toBeLessThan(
    api.details.mock.invocationCallOrder[0],
  );
});

it('fetches nothing when biometrics are refused', async () => {
  prompt.mockResolvedValue(false);
  renderScreen(<CardDetails cardId='card-1' />);
  fireEvent.click(screen.getByTestId('card-show-details'));
  await waitFor(() => expect(prompt).toHaveBeenCalled());
  expect(api.details).not.toHaveBeenCalled();
});

it('hides them after a minute', async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  renderScreen(<CardDetails cardId='card-1' />);
  fireEvent.click(screen.getByTestId('card-show-details'));
  await screen.findByTestId('card-details');
  await act(() => vi.advanceTimersByTimeAsync(DETAILS_VISIBLE_MS));
  expect(screen.queryByTestId('card-details')).toBeNull();
});

it('hides them by hand and in the background', async () => {
  renderScreen(<CardDetails cardId='card-1' />);
  fireEvent.click(screen.getByTestId('card-show-details'));
  fireEvent.click(await screen.findByTestId('card-hide-details'));
  expect(screen.queryByTestId('card-details')).toBeNull();

  fireEvent.click(screen.getByTestId('card-show-details'));
  await screen.findByTestId('card-details');
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => 'hidden',
  });
  act(() => {
    document.dispatchEvent(new Event('visibilitychange'));
  });
  expect(screen.queryByTestId('card-details')).toBeNull();
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => 'visible',
  });
});

it('says why when Gnosis Pay asks for a signature it should not', async () => {
  api.details.mockRejectedValue({
    kind: 'invalid',
    message: 'Gnosis Pay asked to sign something other than showing the card',
    code: null,
  });
  renderScreen(<CardDetails cardId='card-1' />);
  fireEvent.click(screen.getByTestId('card-show-details'));
  expect((await screen.findByRole('alert')).textContent).toContain(
    'something other than showing the card',
  );
});
