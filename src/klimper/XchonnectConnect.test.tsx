// @vitest-environment jsdom
// SPDX-License-Identifier: AGPL-3.0-or-later
// The pairing screen's two promises (clapandpay TASK-069, TASK-072, TASK-076):
//
//   1. a failure is a dead end — no control on screen carries the attempt forward;
//   2. the verified domain is shown in both forms when they differ, with every warning.
//
// Both are rules the integration guide's MUST checklist states, and a rule nobody tests is
// a rule that regresses.

import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PairingPreview } from './xchonnect';

// The app chrome needs the wallet, theme and safe-area contexts; none of that is what
// these tests are about.
vi.mock('@/components/Header', () => ({
  default: ({ title }: { title: string }) => <h1>{title}</h1>,
}));

const inspectPairingUri = vi.fn();
const approvePairing = vi.fn();
const cancelPairing = vi.fn();
const confirmSas = vi.fn();
let pairingHandler: ((event: unknown) => void) | null = null;

vi.mock('./xchonnect', () => ({
  xchonnect: {
    inspectPairingUri: (uri: string) => inspectPairingUri(uri),
    approvePairing: (id: string) => approvePairing(id),
    cancelPairing: (id: string) => cancelPairing(id),
    confirmSas: (id: string, matches: boolean) => confirmSas(id, matches),
  },
  XCHONNECT_EVENTS: { pairing: 'pairing' },
  onXchonnectEvent: (_event: string, handler: (payload: unknown) => void) => {
    pairingHandler = handler;
    return Promise.resolve(() => undefined);
  },
}));

// The scanner path: the test drives the callback the hook would call with scanned text.
let onScanResult: ((text: string) => void) | null = null;
vi.mock('@/hooks/useScannerOrClipboard', () => ({
  useScannerOrClipboard: (handler: (text: string) => void) => {
    onScanResult = handler;
    return {
      handleScanOrPaste: vi.fn(),
      handleScanImage: vi.fn(),
      isMobile: false,
    };
  },
}));

const { renderScreen } = await import('./testHarness');
const XchonnectConnect = (await import('./XchonnectConnect')).default;

const preview: PairingPreview = {
  pairingId: 'p1',
  domainUnicode: 'pengui.xyz',
  domainAscii: 'pengui.xyz',
  domainDiffers: false,
  warnings: [],
  dappName: 'Pengui',
  relay: 'https://relay.example/',
  expiresAt: 1_899_999_999,
  developerMode: false,
  grantedMethods: ['chainId', 'connect', 'getPublicKeys', 'signCoinSpends'],
  grantedKey: '0xabc',
  chainId: 'mainnet',
};

const CODE = 'xchonnect:v1?r=https%3A%2F%2Frelay.example%2F&d=pengui.xyz';

function enterCode(code = CODE) {
  renderScreen(<XchonnectConnect />);
  fireEvent.change(screen.getByLabelText('Pairing code'), {
    target: { value: code },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Check this website' }));
}

beforeEach(() => {
  pairingHandler = null;
  onScanResult = null;
  vi.clearAllMocks();
});

afterEach(cleanup);

describe('a pairing failure is a dead end', () => {
  it('offers no way forward when the origin cannot be verified', async () => {
    inspectPairingUri.mockRejectedValue(
      'this website could not prove it owns its domain, so pairing was stopped: bad signature',
    );

    enterCode();

    await waitFor(() =>
      expect(screen.getByTestId('pairing-failed')).toBeDefined(),
    );
    expect(
      screen.getByText(/There is no way to continue past this/),
    ).toBeDefined();

    // Every control on the screen, by name. Only "read another code" and "cancel" exist,
    // and neither of them continues this attempt.
    const names = screen
      .getAllByRole('button')
      .map((button) => button.textContent ?? '');
    expect(names).toEqual(['Read another code', 'Cancel']);
    for (const name of names) {
      expect(name).not.toMatch(/continue|anyway|proceed|ignore|connect|trust/i);
    }
    // And nothing was created: no mailbox, no reply, no session.
    expect(approvePairing).not.toHaveBeenCalled();
  });

  it('offers no way forward when the dApp never answers', async () => {
    inspectPairingUri.mockResolvedValue(preview);
    approvePairing.mockResolvedValue({
      pairingId: 'p1',
      sas: '123456',
      dappConfirmed: false,
      deadline: Math.floor(Date.now() / 1000) + 300,
    });

    enterCode();
    await waitFor(() => screen.getByRole('button', { name: 'Connect' }));
    fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    await waitFor(() => expect(screen.getByTestId('sas-digits')).toBeDefined());

    // The five-minute timeout arrives as a failed pairing event.
    pairingHandler?.({
      pairingId: 'p1',
      state: 'failed',
      error: 'this pairing code was already used or has expired',
    });

    await waitFor(() =>
      expect(screen.getByTestId('pairing-failed')).toBeDefined(),
    );
    expect(screen.queryByTestId('sas-digits')).toBeNull();
    expect(
      screen.getAllByRole('button').map((button) => button.textContent),
    ).toEqual(['Read another code', 'Cancel']);
    expect(confirmSas).not.toHaveBeenCalled();
  });

  it('cannot be reached past by starting over with the same code', async () => {
    inspectPairingUri.mockRejectedValue('bad signature');
    enterCode();
    await waitFor(() => screen.getByTestId('pairing-failed'));

    fireEvent.click(screen.getByRole('button', { name: 'Read another code' }));

    // Back at the entry stage with an empty field: the refused code is gone, and the
    // user has to read a new one from the website.
    expect(screen.getByLabelText('Pairing code')).toHaveProperty('value', '');
    expect(
      screen.getByRole('button', { name: 'Check this website' }),
    ).toHaveProperty('disabled', true);
  });
});

describe('the verified domain', () => {
  it('shows both forms and every warning when they differ', async () => {
    inspectPairingUri.mockResolvedValue({
      ...preview,
      domainUnicode: 'pengúi.xyz',
      domainAscii: 'xn--pengi-4na.xyz',
      domainDiffers: true,
      warnings: ['NonAscii', 'MixedScripts'],
    });

    enterCode();

    await waitFor(() =>
      expect(screen.getByTestId('domain-unicode').textContent).toBe(
        'pengúi.xyz',
      ),
    );
    expect(screen.getByTestId('domain-ascii').textContent).toContain(
      'xn--pengi-4na.xyz',
    );
    expect(screen.getAllByTestId('domain-warning')).toHaveLength(2);
    expect(screen.getByText(/outside the English alphabet/)).toBeDefined();
    expect(screen.getByText(/mixes alphabets/)).toBeDefined();
  });

  it('shows a warning name it has no sentence for rather than dropping it', async () => {
    inspectPairingUri.mockResolvedValue({
      ...preview,
      warnings: ['SomethingNewUpstream'],
    });

    enterCode();

    await waitFor(() =>
      expect(screen.getAllByTestId('domain-warning')).toHaveLength(1),
    );
    expect(screen.getByText(/SomethingNewUpstream/)).toBeDefined();
  });

  it('labels the name as a claim', async () => {
    inspectPairingUri.mockResolvedValue({
      ...preview,
      dappName: 'Official Chia Wallet Support',
    });

    enterCode();

    await waitFor(() =>
      expect(screen.getByTestId('claimed-name').textContent).toBe(
        'Official Chia Wallet Support',
      ),
    );
    expect(screen.getByText(/It calls itself/)).toBeDefined();
  });
});

describe('entry points', () => {
  it('sends a scanned xchonnect code to origin verification', async () => {
    inspectPairingUri.mockResolvedValue(preview);
    renderScreen(<XchonnectConnect />);

    onScanResult?.(CODE);

    await waitFor(() => expect(inspectPairingUri).toHaveBeenCalledWith(CODE));
  });

  it('sends a scanned universal link to origin verification', async () => {
    inspectPairingUri.mockResolvedValue(preview);
    const url = 'https://clapandpay.com/pair#r=https%3A%2F%2Frelay.example%2F';
    renderScreen(<XchonnectConnect />);

    onScanResult?.(url);

    await waitFor(() => expect(inspectPairingUri).toHaveBeenCalledWith(url));
  });

  it('leaves other scanned content alone', async () => {
    renderScreen(<XchonnectConnect />);

    onScanResult?.('xch1qqqqlongaddress');

    await waitFor(() =>
      expect(screen.getByText(/not an Xchonnect pairing code/)).toBeDefined(),
    );
    expect(inspectPairingUri).not.toHaveBeenCalled();
  });
});
