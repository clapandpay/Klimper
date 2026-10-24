import './cardMock';
// SPDX-License-Identifier: AGPL-3.0-or-later
// A visual preview of the card screens (milestone gnosis-card-mvp) in a plain browser, with
// the card plugin replaced by mockIPC (`cardMock.ts`). Development only, never bundled:
//   npx vite --config vite.preview.config.ts   then open /preview-card.html?state=signin
//   (states listed in cardMock.ts; ?dark=1 for the dark theme; &view=total for the wallet's
//   total on the Assets page with the card in it, TASK-158).
import '@/lib/klimperTheme';
import { discoverThemes, resolveThemeImage } from '@/lib/themes';
import { i18n } from '@lingui/core';
import { I18nProvider } from '@lingui/react';
import { createRoot } from 'react-dom/client';
import {
  createHashRouter,
  createRoutesFromElements,
  Route,
  RouterProvider,
} from 'react-router-dom';
import { ThemeProvider } from 'theme-o-rama';
import '../setup.ts';
import CardScreen from '@/klimper/card/CardScreen';
import { WalletTotal } from '@/klimper/WalletTotal';
import { fetchPrices } from '@/klimper/price';

i18n.loadAndActivate({ locale: 'en-US', messages: {} });

if (new URLSearchParams(location.search).get('dark')) {
  localStorage.setItem('theme', 'dark');
}

const totalView = new URLSearchParams(location.search).get('view') === 'total';
if (totalView) {
  // Made-up prices, as nodexch would answer them: 1 XCH = 10 USD = 9 EUR.
  await fetchPrices(false, {
    fetch: async () =>
      new Response(JSON.stringify({ xch: { usd: 10, eur: 9 }, cats: {} })),
    now: Date.now,
  });
}

const router = createHashRouter(
  createRoutesFromElements(
    <Route
      path='*'
      element={
        totalView ? (
          <div className='mx-auto max-w-2xl p-4'>
            <WalletTotal usd={1234.56} />
          </div>
        ) : (
          <CardScreen />
        )
      }
    />,
  ),
);

const root = document.getElementById('root');
if (root === null) throw new Error('preview: no #root element');

createRoot(root).render(
  <ThemeProvider
    discoverThemes={discoverThemes}
    imageResolver={resolveThemeImage}
  >
    <I18nProvider i18n={i18n}>
      <RouterProvider router={router} />
    </I18nProvider>
  </ThemeProvider>,
);
