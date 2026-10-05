// SPDX-License-Identifier: AGPL-3.0-or-later
// Minimal pairing screen for Xchonnect (clapandpay TASK-075, TASK-076).
//
// Deliberately plain: enough to complete a real pairing and prove the flow end to end.
// The polished screens, the QR entry point and the deep links are TASK-072, TASK-080 and
// TASK-085; this file is meant to be replaced, not extended.
//
// What must survive any redesign:
//   * the verified domain is shown in both forms when they differ, with every warning;
//   * there is no "continue anyway" on a verification failure — the only control is Cancel;
//   * the six digits are large, and the session is unusable until the user confirms them.
import Container from '@/components/Container';
import Header from '@/components/Header';
import { Button } from '@/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { Textarea } from '@/components/ui/textarea';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { AlertTriangle } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  onXchonnectEvent,
  PairingEvent,
  PairingPreview,
  PairingSas,
  xchonnect,
  XCHONNECT_EVENTS,
} from './xchonnect';

type Stage = 'enter' | 'verified' | 'sas' | 'done';

export default function XchonnectPair() {
  const navigate = useNavigate();
  const [stage, setStage] = useState<Stage>('enter');
  const [uri, setUri] = useState('');
  const [preview, setPreview] = useState<PairingPreview | null>(null);
  const [sas, setSas] = useState<PairingSas | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The dApp's `session.confirm` arrives asynchronously; so does a timeout.
  useEffect(() => {
    const unlisten = onXchonnectEvent<PairingEvent>(
      XCHONNECT_EVENTS.pairing,
      (event) => {
        if (event.state === 'failed') {
          setError(event.error ?? t`Pairing failed.`);
          setStage('enter');
          setSas(null);
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
  }, []);

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const check = () =>
    run(async () => {
      const result = await xchonnect.inspectPairingUri(uri.trim());
      setPreview(result);
      setStage('verified');
    });

  const pair = () =>
    run(async () => {
      if (!preview) return;
      setSas(await xchonnect.approvePairing(preview.pairingId));
      setStage('sas');
    });

  const cancel = () =>
    run(async () => {
      if (preview) await xchonnect.cancelPairing(preview.pairingId);
      setPreview(null);
      setSas(null);
      setStage('enter');
    });

  const decide = (matches: boolean) =>
    run(async () => {
      if (!preview) return;
      await xchonnect.confirmSas(preview.pairingId, matches);
      setStage('done');
      navigate('/xchonnect');
    });

  return (
    <>
      <Header title={t`Connect a dApp`} back={() => navigate('/xchonnect')} />
      <Container className='max-w-xl space-y-4'>
        {error && (
          <Card className='border-destructive'>
            <CardHeader>
              <CardTitle className='text-destructive text-base'>
                <Trans>Pairing stopped</Trans>
              </CardTitle>
              <CardDescription className='break-words'>{error}</CardDescription>
            </CardHeader>
            <CardContent className='text-sm text-muted-foreground'>
              <Trans>
                There is no way to continue past this. Ask the website for a new
                code.
              </Trans>
            </CardContent>
          </Card>
        )}

        {stage === 'enter' && (
          <Card>
            <CardHeader>
              <CardTitle className='text-base'>
                <Trans>Paste the pairing code</Trans>
              </CardTitle>
              <CardDescription>
                <Trans>
                  The code under the dApp&apos;s QR image, starting with
                  xchonnect:v1.
                </Trans>
              </CardDescription>
            </CardHeader>
            <CardContent className='space-y-3'>
              <Textarea
                aria-label={t`Pairing code`}
                rows={5}
                value={uri}
                onChange={(event) => setUri(event.target.value)}
                className='font-mono text-xs'
              />
              <Button onClick={check} disabled={busy || uri.trim() === ''}>
                <Trans>Check this dApp</Trans>
              </Button>
            </CardContent>
          </Card>
        )}

        {stage === 'verified' && preview && (
          <Card>
            <CardHeader>
              <CardTitle className='text-base'>
                <Trans>Verified website</Trans>
              </CardTitle>
              <CardDescription>
                <Trans>
                  This domain proved it owns the key that signed the code. Only
                  the domain is verified — the name below is what it claims.
                </Trans>
              </CardDescription>
            </CardHeader>
            <CardContent className='space-y-3 text-sm'>
              <div className='text-xl font-semibold break-all'>
                {preview.domainUnicode}
              </div>
              {preview.domainDiffers && (
                <div className='text-muted-foreground break-all'>
                  <Trans>ASCII form</Trans>:{' '}
                  <span className='font-mono'>{preview.domainAscii}</span>
                </div>
              )}
              {preview.warnings.map((warning) => (
                <div
                  key={warning}
                  className='flex items-start gap-2 text-destructive'
                >
                  <AlertTriangle className='h-4 w-4 mt-0.5 shrink-0' />
                  <span>
                    {warning === 'NonAscii' ? (
                      <Trans>
                        This domain uses letters outside the English alphabet.
                        Check it character by character.
                      </Trans>
                    ) : warning === 'MixedScripts' ? (
                      <Trans>
                        This domain mixes alphabets, which is how look-alike
                        domains are built. Check it carefully.
                      </Trans>
                    ) : (
                      warning
                    )}
                  </span>
                </div>
              ))}
              <dl className='grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-muted-foreground'>
                <dt>
                  <Trans>Name it claims</Trans>
                </dt>
                <dd className='text-foreground break-all'>
                  {preview.dappName}
                </dd>
                <dt>
                  <Trans>Network</Trans>
                </dt>
                <dd className='text-foreground'>{preview.chainId}</dd>
                <dt>
                  <Trans>Relay</Trans>
                </dt>
                <dd className='text-foreground break-all font-mono text-xs'>
                  {preview.relay}
                </dd>
                <dt>
                  <Trans>It may ask for</Trans>
                </dt>
                <dd className='text-foreground'>
                  {preview.grantedMethods.join(', ')}
                </dd>
                <dt>
                  <Trans>Address shared</Trans>
                </dt>
                <dd className='text-foreground font-mono text-xs break-all'>
                  {preview.grantedKey ?? t`none`}
                </dd>
              </dl>
              {preview.developerMode && (
                <div className='text-destructive'>
                  <Trans>
                    Developer mode: this code uses a plain-HTTP local relay.
                    Never for real money.
                  </Trans>
                </div>
              )}
              <p className='text-muted-foreground'>
                <Trans>
                  Nothing is signed automatically. Every request still asks you.
                </Trans>
              </p>
              <div className='flex gap-2'>
                <Button onClick={pair} disabled={busy}>
                  <Trans>Pair with this dApp</Trans>
                </Button>
                <Button variant='outline' onClick={cancel} disabled={busy}>
                  <Trans>Cancel</Trans>
                </Button>
              </div>
            </CardContent>
          </Card>
        )}

        {stage === 'sas' && sas && (
          <Card>
            <CardHeader>
              <CardTitle className='text-base'>
                <Trans>Compare these six digits</Trans>
              </CardTitle>
              <CardDescription>
                <Trans>
                  The dApp shows the same six digits. If they are different,
                  somebody is in the middle.
                </Trans>
              </CardDescription>
            </CardHeader>
            <CardContent className='space-y-4'>
              <div className='text-5xl font-mono tracking-[0.3em] text-center py-4'>
                {sas.sas}
              </div>
              {sas.dappConfirmed ? (
                <div className='flex gap-2'>
                  <Button onClick={() => decide(true)} disabled={busy}>
                    <Trans>They match</Trans>
                  </Button>
                  <Button
                    variant='destructive'
                    onClick={() => decide(false)}
                    disabled={busy}
                  >
                    <Trans>They are different</Trans>
                  </Button>
                </div>
              ) : (
                <p className='text-sm text-muted-foreground text-center'>
                  <Trans>Waiting for the dApp…</Trans>
                </p>
              )}
            </CardContent>
          </Card>
        )}
      </Container>
    </>
  );
}
