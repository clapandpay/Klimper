// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 the Klimper authors
//
// "Use my Clap and Pay money": brings the wallet that was made on clapandpay.com (with a passkey,
// when somebody opened a Green Envelope) into Klimper. A passkey belongs to a website, so the
// face or finger check happens on that page; the page answers with one line of text that only
// this app instance can read (src/lib/clapandpayHandoff.ts). From there it is Sage's ordinary
// key import: nothing in the engine or the app bridge is touched.
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
import { useErrors } from '@/hooks/useErrors';
import { useNetwork } from '@/hooks/useNetwork';
import {
  createHandoffRequest,
  HandoffError,
  HandoffRequest,
} from '@/lib/clapandpayHandoff';
import { clapAndPayOrigin } from '@/lib/features';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { readText } from '@tauri-apps/plugin-clipboard-manager';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { commands } from '../bindings';
import { fetchState } from '../state';

function problemText(error: unknown): string | null {
  if (!(error instanceof HandoffError)) return null;
  if (error.code === 'wrong_receiver') {
    return t`That answer was made for another request. Open the page again from here.`;
  }
  return t`That is not an answer from clapandpay.com. Copy it there and paste again.`;
}

export default function ClapAndPayConnect() {
  const navigate = useNavigate();
  const { addError } = useErrors();
  const { setWallet } = useWallet();
  const { network } = useNetwork();

  // Lives in memory only: closing this screen ends the request, and its answer becomes useless.
  const [request, setRequest] = useState<HandoffRequest | null>(null);
  const [opened, setOpened] = useState(false);
  const [pending, setPending] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    createHandoffRequest(clapAndPayOrigin)
      .then((made) => current && setRequest(made))
      .catch(addError);
    return () => {
      current = false;
    };
  }, [addError]);

  const openPage = () => {
    if (!request) return;
    openUrl(request.url)
      .then(() => setOpened(true))
      .catch(addError);
  };

  const paste = async () => {
    if (!request) return;
    setPending(true);
    setProblem(null);
    try {
      const payload = await request.open((await readText()) ?? '');
      const theirs = payload.network;
      if (network !== null && network !== 'unknown' && theirs !== network) {
        setProblem(
          t`That money is on ${theirs}, and Klimper is set to ${network}. Change the network in the settings first.`,
        );
        return;
      }
      await commands.importKey({
        name: 'Clap and Pay',
        key: payload.masterSk,
        derivation_index: 5000,
        hardened: false,
        unhardened: true,
        emoji: '🟢',
      });
      await fetchState();
      const data = await commands.getKey({});
      setWallet(data.key);
      navigate('/wallet');
    } catch (error) {
      const text = problemText(error);
      if (text === null) addError(error as Parameters<typeof addError>[0]);
      else setProblem(text);
    } finally {
      setPending(false);
    }
  };

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
            <div>
              <div className='text-sm text-muted-foreground'>
                <Trans>The page will show this number too</Trans>
              </div>
              <div
                className='font-mono text-4xl font-bold tracking-widest mt-1'
                aria-live='polite'
              >
                {request
                  ? `${request.checkCode.slice(0, 3)} ${request.checkCode.slice(3)}`
                  : '··· ···'}
              </div>
            </div>

            <div className='space-y-3'>
              <Button
                className='w-full'
                variant={opened ? 'outline' : 'default'}
                disabled={!request}
                onClick={openPage}
              >
                <Trans>1. Open clapandpay.com</Trans>
              </Button>
              <LoadingButton
                className='w-full'
                variant={opened ? 'default' : 'outline'}
                disabled={!request}
                loading={pending}
                loadingText={t`Bringing it in…`}
                onClick={paste}
              >
                <Trans>2. Paste what the page gave you</Trans>
              </LoadingButton>
            </div>

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
