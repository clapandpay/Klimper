// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// Never a black screen (clapandpay TASK-105): a screen, the overlay or the app itself
// throwing while it renders leaves a screen with the way back to the wallet.

import { i18n } from '@lingui/core';
import { I18nProvider } from '@lingui/react';
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import { useState } from 'react';
import {
  createMemoryRouter,
  createRoutesFromElements,
  Route,
  RouterProvider,
} from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/features', () => ({
  klimperCoinsEnabled: false,
  nftMintingEnabled: true,
  nftsEnabled: true,
  offersEnabled: true,
  partialOffersEnabled: false,
  optionsEnabled: true,
  profilesEnabled: true,
  swapEnabled: true,
  themeGalleryEnabled: true,
  tokenIssuingEnabled: true,
  xchonnectEnabled: true,
}));
// A data router's navigation trips over jsdom's AbortSignal, so the way back is checked
// at the call it makes.
const navigate = vi.hoisted(() => vi.fn());
vi.mock('react-router-dom', async (original) => ({
  ...(await original<typeof import('react-router-dom')>()),
  useNavigate: () => navigate,
}));
vi.mock('./Scanner', () => ({ scannerRoutes: null }));
vi.mock('./xchonnect/XchonnectRoutes', () => ({ xchonnectRoutes: null }));
const overlayThrows = vi.hoisted(() => ({ value: false }));
vi.mock('./xchonnect/XchonnectApprovalHost', () => ({
  XchonnectApprovalHost: () => {
    if (overlayThrows.value) throw new Error('overlay broke');
    return <div data-testid='overlay' />;
  },
}));

const { AppRecovery } = await import('./KlimperErrorBoundary');
const { withKlimper } = await import('./routes');
const { KlimperRoot } = await import('./KlimperRoot');

i18n.loadAndActivate({ locale: 'en-US', messages: {} });

function Broken(): never {
  throw new Error('screen broke');
}

beforeEach(() => {
  // React logs every caught error; the boundary logs it once more.
  vi.spyOn(console, 'error').mockImplementation(() => undefined);
  overlayThrows.value = false;
  window.location.hash = '';
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('a screen that throws', () => {
  it('shows the recovery screen, and the way back goes to the wallet', () => {
    const router = createMemoryRouter(
      withKlimper(
        createRoutesFromElements(
          <>
            <Route path='/wallet' element={<div data-testid='wallet' />} />
            <Route path='/broken' element={<Broken />} />
          </>,
        ),
      ),
      { initialEntries: ['/broken'] },
    );
    render(
      <I18nProvider i18n={i18n}>
        <RouterProvider router={router} />
      </I18nProvider>,
    );

    expect(screen.getByTestId('recovery-screen')).toBeDefined();
    expect(screen.getByTestId('recovery-error').textContent).toBe(
      'screen broke',
    );
    fireEvent.click(screen.getByTestId('recovery-wallet'));
    expect(navigate).toHaveBeenCalledWith('/wallet', { replace: true });
  });
});

describe('the app throwing outside the router', () => {
  it('shows the recovery screen, and the way back renders the app again on the wallet', () => {
    let fail = true;
    function App() {
      if (fail) throw new Error('provider broke');
      return <div data-testid='app' />;
    }
    render(
      <AppRecovery>
        <App />
      </AppRecovery>,
    );

    expect(screen.getByTestId('recovery-error').textContent).toBe(
      'provider broke',
    );
    fail = false;
    fireEvent.click(screen.getByTestId('recovery-wallet'));
    expect(screen.getByTestId('app')).toBeDefined();
    expect(window.location.hash).toBe('#/wallet');
  });
});

describe('the approval overlay throwing', () => {
  it('keeps the screen underneath, points to the requests page, and tries again on the next route', async () => {
    overlayThrows.value = true;
    function Page() {
      const [clicks, setClicks] = useState(0);
      return (
        <button
          type='button'
          data-testid='page'
          onClick={() => setClicks(clicks + 1)}
        >
          {clicks}
        </button>
      );
    }
    render(
      <I18nProvider i18n={i18n}>
        <KlimperRoot />
        <Page />
      </I18nProvider>,
    );

    expect(screen.queryByTestId('recovery-screen')).toBeNull();
    fireEvent.click(screen.getByTestId('page'));
    expect(screen.getByTestId('page').textContent).toBe('1');

    fireEvent.click(screen.getByTestId('approval-overlay-failed'));
    await waitFor(() =>
      expect(window.location.hash).toBe('#/xchonnect/request'),
    );

    overlayThrows.value = false;
    window.location.hash = '#/wallet';
    window.dispatchEvent(new HashChangeEvent('hashchange'));
    await waitFor(() => expect(screen.getByTestId('overlay')).toBeDefined());
  });
});
