// SPDX-License-Identifier: AGPL-3.0-or-later
// Rendering one Xchonnect screen on its own, for the component tests.
//
// Only two things are supplied: the i18n provider the `Trans` macro compiles against (with
// an empty catalogue, so every assertion reads the English source string), and a router.
// Everything else a screen needs is mocked in the test that needs it, which keeps each
// test about the rule it is checking.

import { i18n } from '@lingui/core';
import { I18nProvider } from '@lingui/react';
import { render, RenderResult } from '@testing-library/react';
import { ReactNode } from 'react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';

i18n.loadAndActivate({ locale: 'en-US', messages: {} });

export function renderScreen(
  screen: ReactNode,
  {
    path = '/',
    route = '/',
    state = null,
  }: { path?: string; route?: string; state?: unknown } = {},
): RenderResult {
  return render(
    <I18nProvider i18n={i18n}>
      <MemoryRouter initialEntries={[{ pathname: route, state }]}>
        <Routes>
          <Route path={path} element={screen} />
          <Route path='*' element={<div data-testid='elsewhere' />} />
        </Routes>
      </MemoryRouter>
    </I18nProvider>,
  );
}
