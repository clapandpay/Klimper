// SPDX-License-Identifier: AGPL-3.0-or-later
// The card (milestone card-payment, docs/card.md): a payment card from the build's card
// issuer (Gnosis Pay first), on an account that the wallet's own recovery words control.
// The issuer comes from `card_status` and is handed down through `CardIssuerProvider`.
//
// This screen answers, in order: can this wallet have a card here, is it signed in, and
// then hands over to the signed-in part (`CardHome`). Signing in is a signature by the card
// key, so it asks for Face ID or Touch ID first when the wallet requires it, as Sage does
// for every signature.

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
import { useBiometric } from '@/hooks/useBiometric';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { AlertTriangle, CreditCard, KeyRound, Loader2 } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { card, CardIssuer, cardErrorMessage, CardStatus } from './card';
import { CardHome } from './CardHome';
import { CardIssuerProvider } from './issuer';

export default function CardScreen() {
  const { promptIfEnabled } = useBiometric();
  const [status, setStatus] = useState<CardStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await card.status());
      setError(null);
    } catch (e) {
      setError(cardErrorMessage(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const signIn = async () => {
    setBusy(true);
    setError(null);
    try {
      if (!(await promptIfEnabled())) return;
      await card.signIn();
    } catch (e) {
      setError(cardErrorMessage(e));
    } finally {
      setBusy(false);
      await refresh();
    }
  };

  const signOut = async () => {
    try {
      await card.signOut();
    } finally {
      await refresh();
    }
  };

  return (
    <Layout>
      <Header title={t`Card`} />
      <Container className='max-w-2xl space-y-4'>
        {error && (
          <div
            className='flex items-start gap-2 rounded-lg border border-destructive p-3 text-sm text-destructive'
            role='alert'
            data-testid='card-error'
          >
            <AlertTriangle
              className='mt-0.5 h-4 w-4 shrink-0'
              aria-hidden='true'
            />
            <span>{error}</span>
          </div>
        )}
        {status === null ? (
          !error && (
            <div className='flex justify-center p-8' data-testid='card-loading'>
              <Loader2
                className='h-6 w-6 animate-spin text-muted-foreground'
                aria-label={t`Loading`}
              />
            </div>
          )
        ) : !status.configured || !status.issuer ? (
          <NotInThisBuild />
        ) : !status.available ? (
          <NoCardKey reason={status.unavailable} />
        ) : !status.signedIn ? (
          <SignIn
            issuer={status.issuer}
            address={status.address ?? ''}
            busy={busy}
            onSignIn={signIn}
          />
        ) : (
          <CardIssuerProvider issuer={status.issuer}>
            <CardHome
              address={status.address ?? ''}
              onSignOut={signOut}
              onSignedOut={refresh}
            />
          </CardIssuerProvider>
        )}
      </Container>
    </Layout>
  );
}

function NotInThisBuild() {
  return (
    <Card data-testid='card-not-configured'>
      <CardHeader>
        <CardTitle className='text-base'>
          <Trans>No card in this build</Trans>
        </CardTitle>
        <CardDescription>
          <Trans>
            This copy of Klimper was built without a card provider. Nothing
            about your wallet was sent anywhere.
          </Trans>
        </CardDescription>
      </CardHeader>
    </Card>
  );
}

function NoCardKey({ reason }: { readonly reason: CardStatus['unavailable'] }) {
  return (
    <Card data-testid='card-unavailable'>
      <CardHeader>
        <CardTitle className='text-base'>
          {reason === 'noRecoveryWords' ? (
            <Trans>This wallet cannot have a card</Trans>
          ) : (
            <Trans>Open a wallet first</Trans>
          )}
        </CardTitle>
        {reason === 'noRecoveryWords' && (
          <CardDescription>
            <Trans>
              The card is controlled by a key made from the wallet’s recovery
              words. This wallet was imported without them.
            </Trans>
          </CardDescription>
        )}
      </CardHeader>
    </Card>
  );
}

function SignIn({
  issuer,
  address,
  busy,
  onSignIn,
}: {
  readonly issuer: CardIssuer;
  readonly address: string;
  readonly busy: boolean;
  readonly onSignIn: () => void;
}) {
  const { name, scheme, network } = issuer;
  const sandbox = issuer.environment !== 'production';
  return (
    <Card data-testid='card-sign-in'>
      <CardHeader>
        <div className='flex items-start gap-3'>
          <CreditCard className='mt-0.5 h-5 w-5 shrink-0' aria-hidden='true' />
          <div className='space-y-1'>
            <CardTitle className='text-base'>
              <Trans>Pay by card with your own money</Trans>
            </CardTitle>
            <CardDescription>
              <Trans>
                The card is a {scheme} card by {name}. Its money sits in an
                account on {network} that only your recovery words control.
                {name} checks who you are; Klimper never sees your documents,
                your card number or your payments on any server.
              </Trans>
            </CardDescription>
          </div>
        </div>
      </CardHeader>
      <CardContent className='space-y-4'>
        <div className='space-y-1'>
          <div className='text-xs text-muted-foreground'>
            <Trans>Card address</Trans>
          </div>
          <div
            className='break-all font-mono text-sm'
            data-testid='card-address'
          >
            {address}
          </div>
        </div>
        {sandbox && (
          <div
            className='rounded-md bg-muted p-2 text-xs text-muted-foreground'
            data-testid='card-sandbox'
          >
            <Trans>Test mode: {name}’s sandbox, no real money.</Trans>
          </div>
        )}
        <Button
          onClick={onSignIn}
          disabled={busy}
          data-testid='card-sign-in-button'
        >
          {busy ? (
            <Loader2 className='mr-2 h-4 w-4 animate-spin' aria-hidden='true' />
          ) : (
            <KeyRound className='mr-2 h-4 w-4' aria-hidden='true' />
          )}
          <Trans>Sign in to {name}</Trans>
        </Button>
      </CardContent>
    </Card>
  );
}
