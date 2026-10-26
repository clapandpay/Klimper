// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The Scan tab: it is only a camera, and where each kind of code it reads goes.

import { act, cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

vi.mock('@/lib/features', () => ({
  xchonnectEnabled: true,
  offersEnabled: true,
  partialOffersEnabled: false,
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
vi.mock('./xchonnect/xchonnectLinkHandoff', () => ({
  offerXchonnectLink: (raw: string) => offerXchonnectLink(raw),
}));

const setReturnValue = vi.fn();
vi.mock('@/state', () => ({
  useNavigationStore: () => ({ setReturnValue }),
}));

// The camera itself is jsQR over getUserMedia; here it is a stand-in that hands codes and
// problems to the page the way `QrCamera` does.
let onCode: ((text: string) => void) | null = null;
let onProblem: ((problem: string) => void) | null = null;
let supported = true;
vi.mock('./QrCamera', () => ({
  cameraSupported: () => supported,
  QrCamera: (props: {
    onCode: (text: string) => void;
    onProblem: (problem: string) => void;
  }) => {
    onCode = props.onCode;
    onProblem = props.onProblem;
    return <div data-testid='qr-camera' />;
  },
  QrCameraOff: ({ message }: { message: string }) => (
    <div data-testid='camera-off'>{message}</div>
  ),
}));

const { i18n } = await import('@lingui/core');
const { I18nProvider } = await import('@lingui/react');
const { MemoryRouter, Route, Routes } = await import('react-router-dom');
const { render } = await import('@testing-library/react');
const Scanner = (await import('./Scanner')).default;

function renderScanner() {
  i18n.loadAndActivate({ locale: 'en-US', messages: {} });
  return render(
    <I18nProvider i18n={i18n}>
      <MemoryRouter initialEntries={['/scanner']}>
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
  supported = true;
  onCode = null;
  onProblem = null;
  vi.clearAllMocks();
});
afterEach(cleanup);

const PAIR = 'xchonnect:v1?r=wss%3A%2F%2Frelay.example&s=c2VjcmV0';

it('hands an Xchonnect code to the link handoff and never shows it', () => {
  renderScanner();
  act(() => onCode?.(PAIR));
  expect(offerXchonnectLink).toHaveBeenCalledWith(PAIR);
  expect(document.body.textContent).not.toContain('c2VjcmV0');
});

it('opens Send with a scanned address filled in', async () => {
  const address =
    'xch1dtfukqqka3ftqtdlhmc5spc5vd44h7ejrtnjcewxlueam5yrnnqqyczg8t';
  renderScanner();
  act(() => onCode?.(address));
  expect(setReturnValue).toHaveBeenCalledWith('/wallet/send/xch', {
    status: 'success',
    data: address,
  });
  expect(await screen.findByTestId('send')).toBeTruthy();
});

it('shows text it does not know, so it can be copied', async () => {
  renderScanner();
  act(() => onCode?.('hello'));
  expect(await screen.findByTestId('scan-unknown')).toBeTruthy();
  expect(screen.getByText('hello')).toBeTruthy();
  expect(offerXchonnectLink).not.toHaveBeenCalled();
});

it('is only a camera, open as soon as the tab is shown', () => {
  renderScanner();
  expect(screen.getByTestId('qr-camera')).toBeTruthy();
  expect(screen.queryByRole('textbox')).toBeNull();
  expect(screen.queryByText(/paste/i)).toBeNull();
  expect(screen.queryByText(/picture/i)).toBeNull();
});

it('scans again after a code it does not know', async () => {
  renderScanner();
  act(() => onCode?.('hello'));
  expect(screen.queryByTestId('qr-camera')).toBeNull();
  fireEvent.click(await screen.findByText('Scan another code'));
  expect(screen.getByTestId('qr-camera')).toBeTruthy();
  expect(screen.queryByTestId('scan-unknown')).toBeNull();
});

it('says when the camera may not be used, and offers to try again', () => {
  renderScanner();
  act(() => onProblem?.('denied'));
  expect(screen.getByTestId('camera-off').textContent).toContain(
    'may not use the camera',
  );
  fireEvent.click(screen.getByText('Try again'));
  expect(screen.getByTestId('qr-camera')).toBeTruthy();
});

it('says when the device has no camera to share', () => {
  supported = false;
  renderScanner();
  expect(screen.queryByTestId('qr-camera')).toBeNull();
  expect(screen.getByTestId('camera-off')).toBeTruthy();
});
