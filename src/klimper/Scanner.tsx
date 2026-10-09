// SPDX-License-Identifier: AGPL-3.0-or-later
// The Scan tab: a camera, and nothing else.
//
// The camera opens as soon as the tab is shown. It is the webview's own camera, decoded
// frame by frame with the open-source `jsQR` (`QrCamera.tsx`), on a phone and on a
// computer alike: no native scanner, no closed-source decoder. A website's Xchonnect code
// goes to the connect screen, an offer to the offer, an address to Send with the address
// filled in; anything else is shown so it can be copied. Which is which is decided in
// `scanTarget.ts`.

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
import { featureElement } from '@/lib/featureElement';
import { offersEnabled, scanEnabled, xchonnectEnabled } from '@/lib/features';
import { useNavigationStore } from '@/state';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { ScanLine } from 'lucide-react';
import { useCallback, useState } from 'react';
import { Route, useNavigate } from 'react-router-dom';
import {
  CameraProblem,
  cameraSupported,
  QrCamera,
  QrCameraOff,
} from './QrCamera';
import { classifyScan } from './scanTarget';
import { offerXchonnectLink } from './xchonnect/xchonnectLinkHandoff';

export const SCANNER_ROUTE = '/scanner';
/** Where Send for the native coin lives; the scanned address is handed to it there. */
const SEND_XCH_ROUTE = '/wallet/send/xch';

/** One element in `App.tsx`, like `xchonnectRoutes`. */
export const scannerRoutes = (
  <Route
    path={SCANNER_ROUTE}
    element={featureElement(scanEnabled, <Scanner />)}
  />
);

/** `on` while the camera runs, `read` once a code was read and the camera is released. */
type CameraState = 'on' | 'read' | CameraProblem;

function openCamera(): CameraState {
  return cameraSupported() ? 'on' : 'unsupported';
}

function problemMessage(problem: CameraProblem): string {
  switch (problem) {
    case 'denied':
      return t`Klimper may not use the camera. Allow it in the system settings, then try again.`;
    case 'unsupported':
      return t`This device cannot share its camera with Klimper.`;
    case 'unavailable':
    default:
      return t`No camera could be opened.`;
  }
}

export default function Scanner() {
  const navigate = useNavigate();
  const { setReturnValue } = useNavigationStore();
  const [camera, setCamera] = useState<CameraState>(openCamera);
  const [unknown, setUnknown] = useState<string | null>(null);

  const onCode = useCallback(
    (raw: string) => {
      setCamera('read');
      const target = classifyScan(raw, {
        xchonnect: xchonnectEnabled,
        offers: offersEnabled,
      });
      switch (target.kind) {
        case 'xchonnect':
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
          setUnknown(target.text);
          return;
      }
    },
    [navigate, setReturnValue],
  );

  const scanAgain = () => {
    setUnknown(null);
    setCamera(openCamera());
  };

  return (
    <Layout>
      <Header title={t`Scan`} />
      <Container className='max-w-xl space-y-4'>
        {camera === 'on' ? (
          <QrCamera onCode={onCode} onProblem={setCamera} />
        ) : camera !== 'read' ? (
          <QrCameraOff message={problemMessage(camera)} />
        ) : null}

        {camera === 'on' && (
          <p className='text-center text-sm text-muted-foreground'>
            {xchonnectEnabled ? (
              <Trans>
                Point the camera at a website&apos;s connect code, an offer or
                an address.
              </Trans>
            ) : (
              <Trans>Point the camera at an offer or an address.</Trans>
            )}
          </p>
        )}

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

        {camera !== 'on' && (camera !== 'read' || unknown !== null) && (
          <div className='flex justify-center'>
            <Button onClick={scanAgain}>
              <ScanLine className='h-4 w-4 mr-2' aria-hidden='true' />
              {camera === 'read' ? (
                <Trans>Scan another code</Trans>
              ) : (
                <Trans>Try again</Trans>
              )}
            </Button>
          </div>
        )}
      </Container>
    </Layout>
  );
}
