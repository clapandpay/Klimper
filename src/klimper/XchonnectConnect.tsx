// SPDX-License-Identifier: AGPL-3.0-or-later
// Connecting a dApp: the code, the verified domain, the six digits (clapandpay TASK-069,
// TASK-072, TASK-076, TASK-085). Replaces the placeholder `XchonnectPair.tsx`.
//
// The three things this screen exists to get right:
//
//   1. the verified domain is shown in both forms when they differ, with every homograph
//      warning — `XchonnectDomain` is the only way a domain is rendered;
//   2. a failure is a dead end. The `failed` stage renders no control that continues:
//      not behind a confirmation, not behind a disclosure. `XchonnectConnect.test.tsx`
//      asserts that, because a rule nobody tests is a rule that regresses;
//   3. the six digits are large, and the session does not exist until the user says they
//      match.
//
// A pairing code is a secret (spec 6.2): it is never written into the app URL, never
// logged, and never put in an error message. `xchonnectLink.ts` keeps that rule.

import Container from '@/components/Container';
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
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { readText } from '@tauri-apps/plugin-clipboard-manager';
import {
  AlertTriangle,
  Camera,
  ClipboardPaste,
  Image as ImageIcon,
  ShieldCheck,
} from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  CameraProblem,
  cameraProblemMessage,
  cameraSupported,
  QrCamera,
  QrCameraIdle,
  QrCameraOff,
} from './QrCamera';
import { XchonnectDomain } from './XchonnectDomain';
import { ConnectSteps, Fact, Hint, SasDigits, SiteCoin } from './XchonnectUi';
import {
  onXchonnectEvent,
  PairingEvent,
  PairingPreview,
  PairingSas,
  xchonnect,
  XCHONNECT_EVENTS,
} from './xchonnect';
import { LinkProblem, parseXchonnectLink } from './xchonnectLink';
import {
  subscribeXchonnectLink,
  takeXchonnectLink,
  XCHONNECT_ROUTES,
} from './xchonnectLinkHandoff';

type Stage = 'enter' | 'checking' | 'verified' | 'sas' | 'failed';

/** The desktop camera: not asked for until the user taps, released as soon as it reads. */
type CameraState = 'off' | 'on' | CameraProblem;

function problemMessage(problem: LinkProblem): string {
  switch (problem) {
    case 'empty':
      return t`There was nothing to read.`;
    case 'tooLong':
      return t`That is too long to be a pairing code.`;
    case 'otherHost':
      return t`That link does not belong to Klimper.`;
    case 'unknownPath':
      return t`That Klimper link is not a pairing or request link.`;
    case 'noParameters':
      return t`That pairing link is missing its code.`;
    case 'unknownScheme':
    default:
      return t`That is not an Xchonnect pairing code.`;
  }
}

function secondsLeft(deadline: number): number {
  return Math.max(0, deadline - Math.floor(Date.now() / 1000));
}

export default function XchonnectConnect() {
  const navigate = useNavigate();
  const [stage, setStage] = useState<Stage>('enter');
  const [code, setCode] = useState('');
  const [preview, setPreview] = useState<PairingPreview | null>(null);
  const keyCount = preview?.grantedKeyCount ?? 0;
  const [sas, setSas] = useState<PairingSas | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [remaining, setRemaining] = useState(0);
  const [camera, setCamera] = useState<CameraState>('off');
  const fileInput = useRef<HTMLInputElement>(null);

  /** Give up on this attempt. There is no path from here back into the flow. */
  const fail = useCallback((message: string) => {
    setFailure(message);
    setStage('failed');
    setPreview(null);
    setSas(null);
    setCode('');
  }, []);

  const check = useCallback(
    async (raw: string) => {
      const link = parseXchonnectLink(raw);
      if (link.kind === 'notALink') {
        // Not a code at all: say so and stay on the entry stage, where the user can read
        // a different one. Nothing was created anywhere.
        setFailure(problemMessage(link.problem));
        setStage('enter');
        return;
      }
      if (link.kind === 'request') {
        navigate(XCHONNECT_ROUTES.request);
        return;
      }
      setBusy(true);
      setFailure(null);
      setStage('checking');
      try {
        const result = await xchonnect.inspectPairingUri(link.uri);
        setPreview(result);
        setStage('verified');
      } catch (error) {
        fail(String(error));
      } finally {
        setBusy(false);
      }
    },
    [fail, navigate],
  );

  // A link the operating system handed us, or one that arrived while this screen was open.
  useEffect(() => {
    const link = takeXchonnectLink();
    if (link?.kind === 'pair') void check(link.uri);
    if (link?.kind === 'notALink') setFailure(problemMessage(link.problem));
    return subscribeXchonnectLink((incoming) => {
      if (incoming.kind === 'pair') void check(incoming.uri);
      if (incoming.kind === 'notALink') {
        setFailure(problemMessage(incoming.problem));
      }
    });
  }, [check]);

  // The dApp's `session.confirm`, and every way the attempt can die.
  useEffect(() => {
    const unlisten = onXchonnectEvent<PairingEvent>(
      XCHONNECT_EVENTS.pairing,
      (event) => {
        if (event.state === 'failed') {
          fail(event.error ?? t`The website never answered.`);
          return;
        }
        setSas((current) =>
          current ? { ...current, dappConfirmed: true } : current,
        );
      },
    );
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [fail]);

  // The five-minute deadline, so "waiting" is never indefinite.
  useEffect(() => {
    if (sas === null) return;
    setRemaining(secondsLeft(sas.deadline));
    const timer = setInterval(
      () => setRemaining(secondsLeft(sas.deadline)),
      1000,
    );
    return () => clearInterval(timer);
  }, [sas]);

  const onScanResult = useCallback(
    (text: string) => {
      setCamera('off');
      void check(text);
    },
    [check],
  );
  const { handleScanOrPaste, handleScanImage, isMobile } =
    useScannerOrClipboard(onScanResult);

  const pair = async () => {
    if (preview === null) return;
    setBusy(true);
    try {
      setSas(await xchonnect.approvePairing(preview.pairingId));
      setStage('sas');
    } catch (error) {
      fail(String(error));
    } finally {
      setBusy(false);
    }
  };

  const cancel = async () => {
    if (preview !== null) {
      try {
        await xchonnect.cancelPairing(preview.pairingId);
      } catch {
        // Cancelling is the safe direction: nothing to report.
      }
    }
    navigate(XCHONNECT_ROUTES.dapps);
  };

  /** The phone's native scanner, or the computer's camera inside the page. */
  const scan = () => {
    setFailure(null);
    if (isMobile) {
      void handleScanOrPaste();
      return;
    }
    setCamera(cameraSupported() ? 'on' : 'unsupported');
  };

  const paste = async () => {
    setFailure(null);
    try {
      const text = await readText();
      if (text) onScanResult(text);
      else setFailure(t`The clipboard is empty.`);
    } catch {
      setFailure(t`Klimper could not read the clipboard.`);
    }
  };

  const decide = async (matches: boolean) => {
    if (preview === null) return;
    setBusy(true);
    try {
      const session = await xchonnect.confirmSas(preview.pairingId, matches);
      // The list opens on the new connection, so the user sees what they just made.
      navigate(XCHONNECT_ROUTES.dapps, {
        state: matches && session ? { connected: session.sessionId } : null,
      });
    } catch (error) {
      fail(String(error));
    } finally {
      setBusy(false);
    }
  };

  const startOver = () => {
    setFailure(null);
    setStage('enter');
  };

  const step =
    stage === 'enter' ? 'scan' : stage === 'sas' ? 'compare' : 'check';

  return (
    <Layout>
      <Header
        title={t`Connect a website`}
        back={() => navigate(XCHONNECT_ROUTES.dapps)}
      />
      <Container className='max-w-xl space-y-4'>
        {stage !== 'failed' && <ConnectSteps current={step} />}

        {/*
          The dead end. It is rendered instead of the flow, never beside it, so there is
          no control on screen that would carry the attempt forward.
        */}
        {stage === 'failed' && (
          <Card className='border-destructive' data-testid='pairing-failed'>
            <CardHeader>
              <CardTitle className='text-lg text-destructive flex items-center gap-2'>
                <AlertTriangle className='h-5 w-5' aria-hidden='true' />
                <Trans>Connection stopped</Trans>
              </CardTitle>
              <CardDescription className='break-words'>
                {failure}
              </CardDescription>
            </CardHeader>
            <CardContent className='space-y-4 text-sm'>
              <p>
                <Trans>
                  There is no way to continue past this. Ask the website for a
                  new code.
                </Trans>
              </p>
              <div className='flex flex-wrap gap-2'>
                <Button variant='outline' onClick={startOver}>
                  <Trans>Read another code</Trans>
                </Button>
                <Button variant='outline' onClick={() => void cancel()}>
                  <Trans>Cancel</Trans>
                </Button>
              </div>
            </CardContent>
          </Card>
        )}

        {stage === 'enter' && (
          <Card>
            <CardHeader>
              <CardTitle className='text-xl flex items-center gap-1'>
                <Trans>Scan the website&apos;s code</Trans>
                <Hint>
                  <Trans>
                    Press connect on the website. It shows a QR code: point the
                    camera at it, or paste the code under it.
                  </Trans>
                </Hint>
              </CardTitle>
            </CardHeader>
            <CardContent className='space-y-5'>
              {camera === 'on' ? (
                <QrCamera onCode={onScanResult} onProblem={setCamera} />
              ) : camera !== 'off' ? (
                <QrCameraOff message={cameraProblemMessage(camera)} />
              ) : (
                <QrCameraIdle />
              )}

              {failure !== null && (
                <p
                  className='text-sm text-destructive text-center'
                  role='alert'
                >
                  {failure}
                </p>
              )}

              <div className='flex flex-wrap justify-center gap-2'>
                {camera === 'on' ? (
                  <Button variant='outline' onClick={() => setCamera('off')}>
                    <Trans>Stop</Trans>
                  </Button>
                ) : (
                  <Button onClick={scan}>
                    <Camera className='h-4 w-4 mr-2' aria-hidden='true' />
                    <Trans>Scan</Trans>
                  </Button>
                )}
                <Button variant='secondary' onClick={() => void paste()}>
                  <ClipboardPaste className='h-4 w-4 mr-2' aria-hidden='true' />
                  <Trans>Paste</Trans>
                </Button>
                <Button
                  variant='outline'
                  onClick={() => fileInput.current?.click()}
                >
                  <ImageIcon className='h-4 w-4 mr-2' aria-hidden='true' />
                  <Trans>Picture</Trans>
                </Button>
                <input
                  ref={fileInput}
                  type='file'
                  accept='image/*'
                  className='hidden'
                  aria-label={t`Picture of the pairing code`}
                  onChange={(event) => {
                    const file = event.target.files?.[0];
                    event.target.value = '';
                    if (file) void handleScanImage(file);
                  }}
                />
              </div>

              <details className='border-t border-border pt-3'>
                <summary className='cursor-pointer text-sm text-muted-foreground'>
                  <Trans>Type the code</Trans>
                </summary>
                <div className='space-y-2 pt-3'>
                  <Textarea
                    aria-label={t`Pairing code`}
                    rows={3}
                    value={code}
                    onChange={(event) => setCode(event.target.value)}
                    className='font-mono text-xs'
                    placeholder='xchonnect:v1?…'
                  />
                  <Button
                    variant={code.trim() === '' ? 'outline' : 'default'}
                    onClick={() => void check(code)}
                    disabled={busy || code.trim() === ''}
                  >
                    <Trans>Check</Trans>
                  </Button>
                </div>
              </details>
            </CardContent>
          </Card>
        )}

        {stage === 'checking' && (
          <Card>
            <CardHeader>
              <CardTitle className='text-xl flex items-center gap-1'>
                <Trans>Checking the website…</Trans>
                <Hint>
                  <Trans>
                    Klimper is asking the domain to prove it signed this code.
                  </Trans>
                </Hint>
              </CardTitle>
            </CardHeader>
          </Card>
        )}

        {stage === 'verified' && preview !== null && (
          <Card>
            <CardHeader className='space-y-4'>
              <div className='flex items-center gap-1.5 text-sm font-medium xc-verified'>
                <ShieldCheck className='h-4 w-4' aria-hidden='true' />
                <Trans>Domain verified</Trans>
              </div>
              <div className='flex items-start gap-4'>
                <SiteCoin domain={preview.domainUnicode} size='lg' />
                <div className='min-w-0 flex-1'>
                  <XchonnectDomain
                    identity={{
                      domainUnicode: preview.domainUnicode,
                      domainAscii: preview.domainAscii,
                      domainDiffers: preview.domainDiffers,
                      warnings: preview.warnings,
                      dappName: preview.dappName,
                    }}
                  />
                </div>
              </div>
              <CardDescription>
                <Trans>
                  Only the domain is verified — the name is what it claims.
                </Trans>
              </CardDescription>
            </CardHeader>
            <CardContent className='space-y-5 text-sm'>
              <dl className='grid gap-4 rounded-2xl border border-border p-4 sm:grid-cols-2'>
                <Fact label={<Trans>Network</Trans>}>{preview.chainId}</Fact>
                <Fact label={<Trans>It may ask for</Trans>}>
                  {preview.grantedMethods.join(', ')}
                </Fact>
                <Fact label={<Trans>Addresses it will see</Trans>}>
                  {keyCount > 0 ? (
                    <span className='inline-flex items-center gap-1'>
                      <Trans>{keyCount} receive addresses</Trans>
                      <Hint>
                        <Trans>So it can show your balances.</Trans>
                      </Hint>
                    </span>
                  ) : (
                    t`none`
                  )}
                </Fact>
                <Fact label={<Trans>Relay</Trans>} mono>
                  {preview.relay}
                </Fact>
              </dl>
              {preview.developerMode && (
                <p
                  className='flex items-start gap-2 text-destructive'
                  role='alert'
                >
                  <AlertTriangle
                    className='h-4 w-4 mt-0.5 shrink-0'
                    aria-hidden='true'
                  />
                  <Trans>
                    Developer mode: this code uses a local relay without
                    encryption in transit. Never for real money.
                  </Trans>
                </p>
              )}
              <div className='flex flex-wrap items-center gap-2'>
                <Button onClick={() => void pair()} disabled={busy}>
                  <Trans>Connect</Trans>
                </Button>
                <Button
                  variant='outline'
                  onClick={() => void cancel()}
                  disabled={busy}
                >
                  <Trans>Cancel</Trans>
                </Button>
                <Hint>
                  <Trans>
                    Nothing is signed automatically. Every request still asks
                    you, and you can change what this website may do at any
                    time.
                  </Trans>
                </Hint>
              </div>
            </CardContent>
          </Card>
        )}

        {stage === 'sas' && sas !== null && (
          <Card>
            <CardHeader>
              <CardTitle className='text-xl flex items-center gap-1'>
                <Trans>Same digits on the website?</Trans>
              </CardTitle>
              <CardDescription>
                <Trans>If they are different, somebody is in the middle.</Trans>
              </CardDescription>
            </CardHeader>
            <CardContent className='space-y-5'>
              <SasDigits sas={sas.sas} />
              {sas.dappConfirmed ? (
                <div className='flex flex-wrap justify-center gap-2'>
                  <Button onClick={() => void decide(true)} disabled={busy}>
                    <Trans>They match</Trans>
                  </Button>
                  <Button
                    variant='destructive'
                    onClick={() => void decide(false)}
                    disabled={busy}
                  >
                    <Trans>They are different</Trans>
                  </Button>
                </div>
              ) : (
                <p className='text-sm text-muted-foreground text-center'>
                  <Trans>Waiting for the website… {remaining}s</Trans>
                </p>
              )}
            </CardContent>
          </Card>
        )}
      </Container>
    </Layout>
  );
}
