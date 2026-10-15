import './mock';
// SPDX-License-Identifier: AGPL-3.0-or-later
// A visual preview of the approval overlay and the requests page in a plain browser, with
// the Tauri backend replaced by mockIPC (`mock.ts`). Development only, never bundled:
//   npx vite --config vite.preview.config.ts   then open /preview-approvals.html#/wallet
//   (?n=1 one request, ?dark=1 dark theme, ?trade=1|2|5|10 one request with that many
//   assets, ?usd=0 no prices) or /preview-phone.html for phone sizes.
import '@/lib/klimperTheme';
import { discoverThemes, resolveThemeImage } from '@/lib/themes';
import { i18n } from '@lingui/core';
import { I18nProvider } from '@lingui/react';
import { createRoot } from 'react-dom/client';
import {
  createHashRouter,
  Route,
  RouterProvider,
  createRoutesFromElements,
} from 'react-router-dom';
import { ThemeProvider } from 'theme-o-rama';
import '../setup.ts';
import { XchonnectApprovalHost } from '@/klimper/xchonnect/XchonnectApprovalHost';
import XchonnectApproval from '@/klimper/xchonnect/XchonnectApproval';
import XchonnectDappSettings from '@/klimper/xchonnect/XchonnectDappSettings';
import { fetchPrices } from '@/klimper/price';

i18n.loadAndActivate({ locale: 'en-US', messages: {} });

const params = new URLSearchParams(location.search);

// Prices as nodexch would answer, so the dollar figures show (`?usd=0` leaves them out).
if (params.get('usd') !== '0') {
  await fetchPrices(false, {
    fetch: () =>
      Promise.resolve(
        new Response(
          JSON.stringify({
            at: 0,
            xch: { usd: 12.4, eur: 11.5 },
            cats: {
              ['5b'.repeat(32)]: { xch: 0.0005, usd: 0.0062 },
              ['d1'.repeat(32)]: { xch: 0.21, usd: 2.6 },
              ['3c'.repeat(32)]: { xch: 0.0806, usd: 1 },
            },
          }),
        ),
      ),
    now: Date.now,
  });
}
if (params.get('dark')) localStorage.setItem('theme', 'dark');
else localStorage.setItem('theme', 'light');

function Wallet() {
  return (
    <div className='p-8 space-y-4'>
      <h1 className='text-3xl font-bold'>Wallet</h1>
      <div className='bg-card shadow-card rounded-[20px] p-6 h-40'>
        Balance 12.5 XCH
      </div>
      <div className='bg-card shadow-card rounded-[20px] p-6 h-64'>Tokens…</div>
    </div>
  );
}
const router = createHashRouter(
  createRoutesFromElements(
    <>
      <Route path='/' element={<Wallet />} />
      <Route path='/wallet' element={<Wallet />} />
      <Route
        path='/xchonnect/dapp/:sessionId'
        element={<XchonnectDappSettings />}
      />
      <Route path='/xchonnect/request' element={<XchonnectApproval />} />
      <Route
        path='/xchonnect/request/:approvalId'
        element={<XchonnectApproval />}
      />
    </>,
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
      <XchonnectApprovalHost />
      <RouterProvider router={router} />
    </I18nProvider>
  </ThemeProvider>,
);
