import './mock';
// SPDX-License-Identifier: AGPL-3.0-or-later
// A visual preview of the approval overlay and the requests page in a plain browser, with
// the Tauri backend replaced by mockIPC (`mock.ts`). Development only, never bundled:
//   npx vite --config vite.preview.config.ts   then open /preview-approvals.html#/wallet
//   (?n=1 one request, ?dark=1 dark theme) or /preview-phone.html for phone sizes.
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

i18n.loadAndActivate({ locale: 'en-US', messages: {} });

const params = new URLSearchParams(location.search);
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
