// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 the Klimper authors
//
// "Use my Clap and Pay money": brings the wallet that was made on clapandpay.com (with a passkey,
// when somebody opened a Green Envelope) into Klimper. A passkey belongs to a website, so the
// face or finger check happens on that page. Klimper makes a one-time key and a nonce, opens the
// page, and the page answers with a clapandpay.com/klimper/return link that the phone hands to
// this app alone (handoff version 2, TASK-129, src/lib/clapandpayHandoff.ts): the wallet's 24
// words, sealed to that key, opened once and only within two minutes. From there it is Sage's
// ordinary mnemonic import: nothing in the engine or the app bridge is touched, and Sage's own
// screens show and export the words. Sage's "Biometric Authentication" setting guards spending,
// and it is off until turned on, so the import ends by offering it (TASK-126 audit).
import Container from '@/components/Container';
import Header from '@/components/Header';
import SafeAreaView from '@/components/SafeAreaView';
import { Alert, AlertDescription } from '@/components/ui/alert';
import { Button } from '@/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { LoadingButton } from '@/components/ui/loading-button';
import { useWallet } from '@/contexts/WalletContext';
import { useBiometric } from '@/hooks/useBiometric';
import { useErrors } from '@/hooks/useErrors';
import {
  createHandoffRequest,
  HandoffError,
  HandoffPayload,
  HandoffRequest,
} from '@/lib/clapandpayHandoff';
import { awaitClapAndPayReturn } from './clapandpayReturn';
import { clapAndPayOrigin } from '@/lib/features';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { commands } from '@/bindings';
import { clearState, fetchState } from '@/state';

// The page this build talks to: envelope.clapandpay.com, or a Green Envelope dev server such as
// localhost:5173 when VITE_CLAPANDPAY_ORIGIN says so (docs/clapandpay-connect.md).
const pageHost = new URL(clapAndPayOrigin).host;

function problemText(error: unknown): string | null {
  if (!(error instanceof HandoffError)) return null;
  if (error.code === 'expired' || error.code === 'used') {
    return t`That took longer than two minutes. Start again.`;
  }
  if (error.code === 'wrong_receiver') {
    return t`That answer was made for another request. Start again.`;
  }
  return t`That is not an answer from ${pageHost}. Start again.`;
}

export default function ClapAndPayConnect() {
  const navigate = useNavigate();
  const { addError } = useErrors();
  const { setWallet } = useWallet();

  // Lives in memory only: closing this screen ends the request, and its answer becomes useless.
  const [request, setRequest] = useState<HandoffRequest | null>(null);
  const [opened, setOpened] = useState(false);
  const [pending, setPending] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  // Bumped by "Start again": a fresh key and nonce, the old request forgotten.
  const [attempt, setAttempt] = useState(0);
  // An answer for another network than Klimper's: kept, so one tap can switch and finish.
  const [elsewhere, setElsewhere] = useState<HandoffPayload | null>(null);
  const biometric = useBiometric();
  /** After the import: Sage's Face ID check is off, and the phone has Face ID. */
  const [offerBiometric, setOfferBiometric] = useState(false);
  const [enabling, setEnabling] = useState(false);

  useEffect(() => {
    let current = true;
    setRequest(null);
    setOpened(false);
    createHandoffRequest(clapAndPayOrigin)
      .then((made) => current && setRequest(made))
      .catch(addError);
    return () => {
      current = false;
    };
  }, [addError, attempt]);

  const openPage = () => {
    if (!request) return;
    openUrl(request.url)
      .then(() => setOpened(true))
      .catch(addError);
  };

  const bringIn = async (payload: HandoffPayload) => {
    await commands.importKey({
      name: 'Clap and Pay',
      key: payload.words,
      derivation_index: 5000,
      hardened: false,
      unhardened: true,
      emoji: '🟢',
    });
    await fetchState();
    const data = await commands.getKey({});
    setWallet(data.key);
    if (biometric.available && !biometric.enabled) {
      setOfferBiometric(true);
      return;
    }
    navigate('/wallet');
  };

  const turnOnBiometric = async () => {
    setEnabling(true);
    setProblem(null);
    try {
      await biometric.enableIfAvailable();
      navigate('/wallet');
    } catch (error) {
      setProblem(String(error));
    } finally {
      setEnabling(false);
    }
  };

  const fail = (error: unknown) => {
    const text = problemText(error);
    if (text === null) addError(error as Parameters<typeof addError>[0]);
    else setProblem(text);
  };

  // The page answered through the return link: open it, then the network, then the import.
  // The listener stays the same; what it does is this render's, through the ref.
  const onAnswer = useRef<(link: string) => void>(() => undefined);
  onAnswer.current = (link) => {
    if (!request || pending) return;
    void (async () => {
      setPending(true);
      setProblem(null);
      setElsewhere(null);
      try {
        const payload = await request.open(link);
        // Names, not kinds: the page says "testnet11", and so does Sage's network list.
        const current = (await commands.getNetwork({})).network.name;
        if (payload.network !== current) {
          setElsewhere(payload);
          return;
        }
        await bringIn(payload);
      } catch (error) {
        fail(error);
      } finally {
        setPending(false);
      }
    })();
  };
  useEffect(() => awaitClapAndPayReturn((link) => onAnswer.current(link)), []);

  // The same switch as the network setting, then the import it was waiting for.
  const switchAndBringIn = async () => {
    if (!elsewhere) return;
    setPending(true);
    setProblem(null);
    try {
      clearState();
      await commands.setNetwork({ name: elsewhere.network });
      await fetchState();
      await bringIn(elsewhere);
    } catch (error) {
      fail(error);
    } finally {
      // The words leave memory either way; start again to retry.
      setElsewhere(null);
      setPending(false);
    }
  };

  const theirNetwork = elsewhere?.network ?? '';

  if (offerBiometric) {
    return (
      <SafeAreaView>
        <Header title={t`Clap and Pay`} />
        <Container>
          <Card className='max-w-xl mx-auto'>
            <CardHeader className='text-center'>
              <CardTitle>
                <Trans>Ask for Face ID before paying</Trans>
              </CardTitle>
              <CardDescription>
                <Trans>
                  The money is in Klimper now. Right now anyone who has this
                  phone unlocked can spend it. With Face ID on, Klimper asks for
                  your face before every payment and before showing your
                  recovery words.
                </Trans>
              </CardDescription>
            </CardHeader>
            <CardContent className='space-y-3 text-center'>
              <LoadingButton
                className='w-full'
                loading={enabling}
                loadingText={t`Waiting for Face ID…`}
                onClick={turnOnBiometric}
              >
                <Trans>Turn on Face ID</Trans>
              </LoadingButton>
              <Button
                className='w-full'
                variant='ghost'
                disabled={enabling}
                onClick={() => navigate('/wallet')}
              >
                <Trans>Not now</Trans>
              </Button>
              {problem && (
                <Alert variant='destructive'>
                  <AlertDescription>{problem}</AlertDescription>
                </Alert>
              )}
            </CardContent>
          </Card>
        </Container>
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView>
      <Header title={t`Clap and Pay`} back={() => navigate('/')} />
      <Container>
        <Card className='max-w-xl mx-auto'>
          <CardHeader className='text-center'>
            <CardTitle>
              <Trans>Use my Clap and Pay money</Trans>
            </CardTitle>
            <CardDescription>
              <Trans>
                Opened a Green Envelope? Bring that money into Klimper with your
                face or fingerprint. Nothing to type.
              </Trans>
            </CardDescription>
          </CardHeader>
          <CardContent className='space-y-6 text-center'>
            <div className='space-y-3'>
              <Button
                className='w-full'
                variant={opened ? 'outline' : 'default'}
                disabled={!request || pending}
                onClick={openPage}
              >
                <Trans>Open {pageHost}</Trans>
              </Button>
              {opened && !problem && (
                <p className='text-sm text-muted-foreground' aria-live='polite'>
                  {pending ? (
                    <Trans>Bringing it in…</Trans>
                  ) : (
                    <Trans>
                      Say yes on the page with your face or fingerprint. It
                      brings you back here.
                    </Trans>
                  )}
                </p>
              )}
              {problem && (
                <Button
                  className='w-full'
                  variant='outline'
                  onClick={() => {
                    setProblem(null);
                    setAttempt((n) => n + 1);
                  }}
                >
                  <Trans>Start again</Trans>
                </Button>
              )}
            </div>

            {elsewhere && (
              <Alert>
                <AlertDescription className='space-y-3'>
                  <p>
                    <Trans>
                      That money is on {theirNetwork}. Klimper switches over to
                      bring it in; you can switch back in the settings.
                    </Trans>
                  </p>
                  <LoadingButton
                    className='w-full'
                    loading={pending}
                    loadingText={t`Bringing it in…`}
                    onClick={switchAndBringIn}
                  >
                    <Trans>Switch to {theirNetwork} and bring it in</Trans>
                  </LoadingButton>
                </AlertDescription>
              </Alert>
            )}

            {problem && (
              <Alert variant='destructive'>
                <AlertDescription>{problem}</AlertDescription>
              </Alert>
            )}

            <p className='text-sm text-muted-foreground'>
              <Trans>
                It then lives in Klimper on this device, as its own account with
                24 recovery words you can show and write down. If you lose the
                device, open {pageHost} again: your passkey brings the money
                back.
              </Trans>
            </p>
          </CardContent>
        </Card>
      </Container>
    </SafeAreaView>
  );
}
