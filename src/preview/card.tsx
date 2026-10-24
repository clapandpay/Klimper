import './cardMock';
// SPDX-License-Identifier: AGPL-3.0-or-later
// A visual preview of the card screens (milestone gnosis-card-mvp) in a plain browser, with
// the card plugin replaced by mockIPC (`cardMock.ts`). Development only, never bundled:
//   npx vite --config vite.preview.config.ts   then open /preview-card.html?state=signin
//   (states listed in cardMock.ts; ?dark=1 for the dark theme).
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

i18n.loadAndActivate({ locale: 'en-US', messages: {} });

if (new URLSearchParams(location.search).get('dark')) {
  localStorage.setItem('theme', 'dark');
}

const router = createHashRouter(
  createRoutesFromElements(<Route path='*' element={<CardScreen />} />),
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
