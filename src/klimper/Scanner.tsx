// SPDX-License-Identifier: AGPL-3.0-or-later
// The Scan tab: one place to point the camera at any code.
//
// A website's Xchonnect code goes to the connect screen, an offer to the offer, an address
// to Send with the address filled in; anything else is shown so it can be copied. Which
// is which is decided in `scanTarget.ts`.
//
// On a phone the camera opens as soon as the tab is chosen. The tab passes `start` in the
// route state; the page clears it before it opens the camera, so coming back from the
// camera — scanned or cancelled — lands on this page without opening it again. Sage's
// `/scan` page does the scanning and hands the result back through the navigation store,
// exactly as it does for Send.

import Container from '@/components/Container';
import { CopyBox } from '@/components/CopyBox';
import Header from '@/components/Header';
import Layout from '@/components/Layout';
import { Button } from '@/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { Textarea } from '@/components/ui/textarea';
import { useScannerOrClipboard } from '@/hooks/useScannerOrClipboard';
import { featureElement } from '@/lib/featureElement';
import { offersEnabled, scanEnabled, xchonnectEnabled } from '@/lib/features';
import { useNavigationStore } from '@/state';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import {
  ClipboardPaste,
  Globe,
  Image as ImageIcon,
  ScanLine,
} from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Route, useLocation, useNavigate } from 'react-router-dom';
import { classifyScan } from './scanTarget';
import { offerXchonnectLink, XCHONNECT_ROUTES } from './xchonnectLinkHandoff';

export const SCANNER_ROUTE = '/scanner';
/** Where Send for the native coin lives; the scanned address is handed to it there. */
const SEND_XCH_ROUTE = '/wallet/send/xch';

/** Route state that asks the page to open the camera straight away. */
export interface ScannerState {
  start?: boolean;
}

/** One element in `App.tsx`, like `xchonnectRoutes`. */
export const scannerRoutes = (
  <Route
    path={SCANNER_ROUTE}
    element={featureElement(scanEnabled, <Scanner />)}
  />
);

export default function Scanner() {
  const navigate = useNavigate();
  const location = useLocation();
  const { setReturnValue } = useNavigationStore();
  const fileInput = useRef<HTMLInputElement>(null);
  const [code, setCode] = useState('');
  const [unknown, setUnknown] = useState<string | null>(null);

  const onScanResult = useCallback(
    (raw: string) => {
      const target = classifyScan(raw, {
        xchonnect: xchonnectEnabled,
        offers: offersEnabled,
      });
      switch (target.kind) {
        case 'xchonnect':
          setCode('');
          // The single entry point for every link: it keeps the code in memory and
          // navigates, so the secret never reaches the app URL.
          offerXchonnectLink(raw);
          return;
        case 'offer':
          navigate(`/offers/view/${encodeURIComponent(target.offer)}`);
          return;
        case 'address':
          setReturnValue(SEND_XCH_ROUTE, {
            status: 'success',
            data: target.address,
          });
          navigate(SEND_XCH_ROUTE);
          return;
        case 'unknown':
          setCode('');
          setUnknown(target.text);
          return;
      }
    },
    [navigate, setReturnValue],
  );

  const { handleScanOrPaste, handleScanImage, isMobile } =
    useScannerOrClipboard(onScanResult);

  const autoStart = (location.state as ScannerState | null)?.start === true;
  useEffect(() => {
    if (!autoStart) return;
    navigate(SCANNER_ROUTE, { replace: true, state: null });
    if (isMobile) void handleScanOrPaste();
    // Runs once per tap of the tab; `handleScanOrPaste` is a new function every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoStart]);

  return (
    <Layout>
      <Header title={t`Scan`} />
      <Container className='max-w-xl space-y-4'>
        <Card>
          <CardHeader>
            <CardTitle className='text-base'>
              <Trans>Scan a code</Trans>
            </CardTitle>
            <CardDescription>
              {xchonnectEnabled ? (
                <Trans>
                  A website&apos;s connect code, an offer or an address. Klimper
                  works out which it is.
                </Trans>
              ) : (
                <Trans>
                  An offer or an address. Klimper works out which it is.
                </Trans>
              )}
            </CardDescription>
          </CardHeader>
          <CardContent className='space-y-3'>
            <div className='flex flex-wrap gap-2'>
              <Button onClick={() => void handleScanOrPaste()}>
                {isMobile ? (
                  <>
                    <ScanLine className='h-4 w-4 mr-2' aria-hidden='true' />
                    <Trans>Open the camera</Trans>
                  </>
                ) : (
                  <>
                    <ClipboardPaste
                      className='h-4 w-4 mr-2'
                      aria-hidden='true'
                    />
                    <Trans>Paste from the clipboard</Trans>
                  </>
                )}
              </Button>
              <Button
                variant='outline'
                onClick={() => fileInput.current?.click()}
              >
                <ImageIcon className='h-4 w-4 mr-2' aria-hidden='true' />
                <Trans>Choose a picture of the code</Trans>
              </Button>
              <input
                ref={fileInput}
                type='file'
                accept='image/*'
                className='hidden'
                aria-label={t`Picture of the code`}
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  event.target.value = '';
                  if (file) void handleScanImage(file);
                }}
              />
            </div>
            <Textarea
              aria-label={t`Code`}
              rows={3}
              value={code}
              onChange={(event) => setCode(event.target.value)}
              className='font-mono text-xs'
              placeholder={t`Or paste a code here`}
            />
            <Button
              variant='outline'
              onClick={() => onScanResult(code)}
              disabled={code.trim() === ''}
            >
              <Trans>Continue</Trans>
            </Button>
          </CardContent>
        </Card>

        {unknown !== null && (
          <Card data-testid='scan-unknown'>
            <CardHeader>
              <CardTitle className='text-base'>
                <Trans>Klimper does not know this code</Trans>
              </CardTitle>
              <CardDescription>
                {unknown === '' ? (
                  <Trans>
                    It is not something this version of Klimper can open.
                  </Trans>
                ) : (
                  <Trans>
                    It is not a connect code, an offer or an address. Here is
                    what it says.
                  </Trans>
                )}
              </CardDescription>
            </CardHeader>
            {unknown !== '' && (
              <CardContent>
                <CopyBox title={t`Scanned text`} value={unknown} />
              </CardContent>
            )}
          </Card>
        )}

        {xchonnectEnabled && (
          <Button
            variant='ghost'
            className='w-full'
            onClick={() => navigate(XCHONNECT_ROUTES.dapps)}
          >
            <Globe className='h-4 w-4 mr-2' aria-hidden='true' />
            <Trans>Connected websites</Trans>
          </Button>
        )}
      </Container>
    </Layout>
  );
}
