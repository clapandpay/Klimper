// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// VITE_ENABLE_CARD off (TASK-159, TASK-164): the wallet's total never calls the card plugin, so
// nothing reaches a card issuer, and the /card route sends the user back to the wallet.

import { cleanup, render, renderHook, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, expect, it, vi } from 'vitest';

vi.mock('@/lib/features', async (original) => ({
  ...(await original<typeof import('@/lib/features')>()),
  cardEnabled: false,
}));
const api = vi.hoisted(() => ({ status: vi.fn(), account: vi.fn() }));
vi.mock('./card', async (original) => ({
  ...(await original<typeof import('./card')>()),
  card: api,
}));
vi.mock('./CardScreen', () => ({
  default: () => <div data-testid='card-screen' />,
}));

const { useCardBalance } = await import('./useCardBalance');
const { cardRoutes } = await import('./CardRoutes');

afterEach(cleanup);

it('never asks the card plugin while the card is off', async () => {
  const { result } = renderHook(() => useCardBalance());
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(result.current).toEqual({ state: 'none' });
  expect(api.status).not.toHaveBeenCalled();
  expect(api.account).not.toHaveBeenCalled();
});

it('has no card screen while the card is off', async () => {
  const view = render(
    <MemoryRouter initialEntries={['/card']}>
      <Routes>
        {cardRoutes}
        <Route path='*' element={<div data-testid='elsewhere' />} />
      </Routes>
    </MemoryRouter>,
  );
  await waitFor(() => expect(view.getByTestId('elsewhere')).toBeTruthy());
  expect(view.queryByTestId('card-screen')).toBeNull();
});
