// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The Scan tab: where each kind of code goes, and that a phone opens the camera once per
// tap of the tab, not again when it comes back from the camera.

import { cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

vi.mock('@/lib/features', () => ({
  xchonnectEnabled: true,
  offersEnabled: true,
  scanEnabled: true,
}));
vi.mock('@/components/Layout', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('@/components/Header', () => ({
  default: ({ title }: { title: string }) => <h1>{title}</h1>,
}));
vi.mock('@/components/CopyBox', () => ({
  CopyBox: ({ value }: { value: string }) => <output>{value}</output>,
}));

const offerXchonnectLink = vi.fn();
vi.mock('./xchonnectLinkHandoff', () => ({
  offerXchonnectLink: (raw: string) => offerXchonnectLink(raw),
  XCHONNECT_ROUTES: { dapps: '/xchonnect' },
}));

const setReturnValue = vi.fn();
vi.mock('@/state', () => ({
  useNavigationStore: () => ({ setReturnValue }),
}));

let onScanResult: ((text: string) => void) | null = null;
const handleScanOrPaste = vi.fn();
let isMobile = false;
vi.mock('@/hooks/useScannerOrClipboard', () => ({
  useScannerOrClipboard: (handler: (text: string) => void) => {
    onScanResult = handler;
    return { handleScanOrPaste, handleScanImage: vi.fn(), isMobile };
  },
}));

const { i18n } = await import('@lingui/core');
const { I18nProvider } = await import('@lingui/react');
const { MemoryRouter, Route, Routes } = await import('react-router-dom');
const { render } = await import('@testing-library/react');
const Scanner = (await import('./Scanner')).default;

function renderScanner(state: unknown = null) {
  i18n.loadAndActivate({ locale: 'en-US', messages: {} });
  return render(
    <I18nProvider i18n={i18n}>
      <MemoryRouter initialEntries={[{ pathname: '/scanner', state }]}>
        <Routes>
          <Route path='/scanner' element={<Scanner />} />
          <Route path='/wallet/send/xch' element={<div data-testid='send' />} />
          <Route
            path='/offers/view/:offer'
            element={<div data-testid='offer' />}
          />
        </Routes>
      </MemoryRouter>
    </I18nProvider>,
  );
}

beforeEach(() => {
  isMobile = false;
  onScanResult = null;
  vi.clearAllMocks();
});
afterEach(cleanup);

const PAIR = 'xchonnect:v1?r=wss%3A%2F%2Frelay.example&s=c2VjcmV0';

it('hands an Xchonnect code to the link handoff and never shows it', () => {
  renderScanner();
  onScanResult?.(PAIR);
  expect(offerXchonnectLink).toHaveBeenCalledWith(PAIR);
  expect(document.body.textContent).not.toContain('c2VjcmV0');
});

it('takes a pasted code the same way', () => {
  renderScanner();
  fireEvent.change(screen.getByLabelText('Code'), { target: { value: PAIR } });
  fireEvent.click(screen.getByText('Continue'));
  expect(offerXchonnectLink).toHaveBeenCalledWith(PAIR);
});

it('opens Send with a scanned address filled in', async () => {
  const address =
    'xch1dtfukqqka3ftqtdlhmc5spc5vd44h7ejrtnjcewxlueam5yrnnqqyczg8t';
  renderScanner();
  onScanResult?.(address);
  expect(setReturnValue).toHaveBeenCalledWith('/wallet/send/xch', {
    status: 'success',
    data: address,
  });
  expect(await screen.findByTestId('send')).toBeTruthy();
});

it('shows text it does not know, so it can be copied', async () => {
  renderScanner();
  onScanResult?.('hello');
  expect(await screen.findByTestId('scan-unknown')).toBeTruthy();
  expect(screen.getByText('hello')).toBeTruthy();
  expect(offerXchonnectLink).not.toHaveBeenCalled();
});

it('opens the camera once when the tab is tapped on a phone', () => {
  isMobile = true;
  renderScanner({ start: true });
  expect(handleScanOrPaste).toHaveBeenCalledTimes(1);
});

it('does not open the camera when the page is reached any other way', () => {
  isMobile = true;
  renderScanner();
  expect(handleScanOrPaste).not.toHaveBeenCalled();
});

it('does not read the clipboard on its own on a computer', () => {
  renderScanner({ start: true });
  expect(handleScanOrPaste).not.toHaveBeenCalled();
});
