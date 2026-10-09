// SPDX-License-Identifier: AGPL-3.0-or-later
// Who can ask you to sign, and how to cut them off (clapandpay TASK-084). Replaces the
// list half of the placeholder `XchonnectSessions.tsx`.
//
// The screen answers three questions, in this order: is something waiting for me, who is
// connected, and how do I end it. Ending a connection needs no confirmation — it is the
// safe direction, and a confirmation dialog on the safe direction only teaches people to
// click through dialogs.

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
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import {
  AlertTriangle,
  BellOff,
  CheckCircle2,
  KeyRound,
  Plus,
  Settings2,
  Wallet,
} from 'lucide-react';
import { useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { identityOfSession, XchonnectDomain } from './XchonnectDomain';
import { SessionSummary, xchonnect } from './xchonnect';
import { XCHONNECT_ROUTES } from './xchonnectLinkHandoff';
import { useXchonnectApprovals } from './useXchonnectApprovals';
import { useXchonnectPush } from './useXchonnectPush';
import { useXchonnectSessions } from './useXchonnectSessions';
import { Fact, LivePill, SiteCoin } from './XchonnectUi';

/** Route state the connect screen leaves behind: the connection it just made. */
export interface DappsState {
  connected?: string;
}

function limitCount(session: SessionSummary): number {
  return Object.values(session.limits).filter(
    (limit) =>
      (limit.perRequest ?? null) !== null || (limit.perDay ?? null) !== null,
  ).length;
}

function Dapp({
  session,
  walletChainId,
  fresh,
  onEnd,
  onRotate,
}: {
  readonly session: SessionSummary;
  /** The network the wallet is on now; `null` while unknown. */
  readonly walletChainId: string | null;
  readonly fresh: boolean;
  readonly onEnd: (id: string) => void;
  readonly onRotate: (id: string) => void;
}) {
  const navigate = useNavigate();
  const limits = limitCount(session);
  const connected = new Date(session.createdAt * 1000).toLocaleDateString();
  const sessionChain = session.chainId;
  const epoch = session.epoch;
  // Renewing needs the message loop to carry the offer and the dApp to answer it.
  const usable =
    session.sasConfirmed && !session.needsRepair && !session.revoked;
  const canRotate = usable && session.connected && !session.rotating;
  return (
    <Card data-testid='dapp-card' className={fresh ? 'xc-fresh' : undefined}>
      <CardHeader>
        <div className='flex items-start gap-3'>
          <SiteCoin domain={session.domainUnicode} />
          <CardTitle className='min-w-0 flex-1 text-base'>
            <XchonnectDomain
              identity={identityOfSession(session)}
              size='inline'
              showName
            />
          </CardTitle>
          <LivePill live={session.connected} />
        </div>
      </CardHeader>
      <CardContent className='space-y-4 text-sm'>
        {session.revoked && (
          <div
            className='rounded-lg border border-destructive p-3 text-destructive'
            role='alert'
            data-testid='revoked'
          >
            <div className='flex items-start gap-2 font-semibold'>
              <AlertTriangle
                className='h-4 w-4 mt-0.5 shrink-0'
                aria-hidden='true'
              />
              <Trans>Klimper ended this connection</Trans>
            </div>
            <p className='pl-6 mt-1'>
              <Trans>
                The website withdrew the key this connection was made with,
                which is what a website does when that key may have been stolen.
                Nothing can be asked through it any more. Connect again if you
                still use the website.
              </Trans>
            </p>
          </div>
        )}

        {walletChainId !== null &&
          session.chainId !== walletChainId &&
          !session.revoked && (
            <div
              className='rounded-lg border p-3'
              role='status'
              data-testid='other-network'
            >
              <div className='flex items-start gap-2 font-semibold'>
                <AlertTriangle
                  className='h-4 w-4 mt-0.5 shrink-0'
                  aria-hidden='true'
                />
                <Trans>Made on {sessionChain}</Trans>
              </div>
              <p className='pl-6 mt-1'>
                <Trans>
                  Klimper is on {walletChainId} now. The website sees which
                  network Klimper is on, and can ask nothing else until you
                  switch back. To use it on {walletChainId}, connect again.
                </Trans>
              </p>
            </div>
          )}

        {session.needsRepair && !session.revoked && (
          <div
            className='rounded-lg border border-destructive p-3 text-destructive'
            role='alert'
            data-testid='needs-repair'
          >
            <div className='flex items-start gap-2 font-semibold'>
              <AlertTriangle
                className='h-4 w-4 mt-0.5 shrink-0'
                aria-hidden='true'
              />
              <Trans>This connection cannot be used</Trans>
            </div>
            <p className='pl-6 mt-1'>
              <Trans>
                Its stored state was lost or restored from a backup. Klimper
                will not reuse it. Connect to the website again.
              </Trans>
            </p>
          </div>
        )}

        {!session.sasConfirmed && !session.needsRepair && !session.revoked && (
          <div className='text-destructive' role='alert'>
            <Trans>
              The six digits were never confirmed, so this connection cannot be
              used.
            </Trans>
          </div>
        )}

        {(session.allowAggSigUnsafe || session.allowUnknownContracts) && (
          <div
            className='flex items-start gap-2 text-destructive'
            data-testid='dangerous-switch-warning'
          >
            <AlertTriangle
              className='h-4 w-4 mt-0.5 shrink-0'
              aria-hidden='true'
            />
            <span>
              {session.allowAggSigUnsafe && session.allowUnknownContracts ? (
                <Trans>
                  You allowed this website unsafe signatures and contracts
                  Klimper cannot read.
                </Trans>
              ) : session.allowAggSigUnsafe ? (
                <Trans>You allowed this website unsafe signatures.</Trans>
              ) : (
                <Trans>
                  You allowed this website contracts Klimper cannot read.
                </Trans>
              )}
            </span>
          </div>
        )}

        <dl className='grid gap-4 rounded-2xl border border-border p-4 sm:grid-cols-3'>
          <Fact label={<Trans>Connected</Trans>}>{connected}</Fact>
          <Fact label={<Trans>Network</Trans>}>{session.chainId}</Fact>
          <Fact label={<Trans>Limits</Trans>}>
            {limits === 0 ? (
              <Trans>none set</Trans>
            ) : (
              <Trans>{limits} set</Trans>
            )}
          </Fact>
          <div className='sm:col-span-2'>
            <Fact label={<Trans>May call</Trans>}>
              {session.methods.join(', ')}
            </Fact>
          </div>
          {/*
            Either side may renew the keys (spec 9.2.1); the epoch counts how often
            that has happened. "Being renewed" lasts until the website has answered
            and the old mailbox has been read to the end.
          */}
          <Fact label={<Trans>Connection keys</Trans>}>
            {session.rotating ? (
              <Trans>being renewed</Trans>
            ) : (
              <Trans>renewed {epoch} times</Trans>
            )}
          </Fact>
        </dl>

        <div className='flex flex-wrap gap-2'>
          {!session.revoked && (
            <Button
              variant='outline'
              size='sm'
              onClick={() =>
                navigate(`${XCHONNECT_ROUTES.dapp}/${session.sessionId}`)
              }
            >
              <Settings2 className='h-4 w-4 mr-2' aria-hidden='true' />
              <Trans>What it may do</Trans>
            </Button>
          )}
          {usable && (
            <Button
              variant='outline'
              size='sm'
              disabled={!canRotate}
              onClick={() => onRotate(session.sessionId)}
            >
              <KeyRound className='h-4 w-4 mr-2' aria-hidden='true' />
              <Trans>Renew keys</Trans>
            </Button>
          )}
          <Button
            variant='outline'
            size='sm'
            onClick={() => onEnd(session.sessionId)}
          >
            {session.revoked ? (
              <Trans>Remove</Trans>
            ) : (
              <Trans>End connection</Trans>
            )}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

export default function XchonnectDapps() {
  const navigate = useNavigate();
  const location = useLocation();
  const justConnected = (location.state as DappsState | null)?.connected;
  const { status, sessions, loading, error, refresh } = useXchonnectSessions();
  const { approvals } = useXchonnectApprovals();
  const [problem, setProblem] = useState<string | null>(null);

  const end = async (sessionId: string) => {
    try {
      await xchonnect.endSession(sessionId);
    } catch (failure) {
      setProblem(String(failure));
    }
    await refresh();
  };

  const rotate = async (sessionId: string) => {
    try {
      await xchonnect.rotateSessionKeys(sessionId);
      setProblem(null);
    } catch (failure) {
      setProblem(String(failure));
    }
    // The loop reports `rotating` and then `rotated`; each refreshes the list.
    await refresh();
  };

  const push = useXchonnectPush();
  const keyringService = status?.keyringService ?? '';
  const waiting = approvals.length;
  const fresh = sessions.find((session) => session.sessionId === justConnected);
  const freshDomain = fresh?.domainUnicode ?? '';

  return (
    <Layout>
      <Header title={t`Connected websites`}>
        <Button size='sm' onClick={() => navigate(XCHONNECT_ROUTES.connect)}>
          <Plus className='h-4 w-4 mr-2' aria-hidden='true' />
          <Trans>Connect a website</Trans>
        </Button>
      </Header>
      <Container className='max-w-2xl space-y-4'>
        {push.denied && (
          <div
            className='rounded-lg border border-destructive p-3 text-sm text-destructive'
            role='status'
            data-testid='push-denied'
          >
            <div className='flex items-start gap-2 font-semibold'>
              <BellOff className='h-4 w-4 mt-0.5 shrink-0' aria-hidden='true' />
              <Trans>Notifications are off</Trans>
            </div>
            <p className='pl-6 mt-1'>
              <Trans>
                Klimper only sees a request from a website while it is open. To
                be told when one arrives, turn notifications on for Klimper in
                your phone&apos;s settings. Notifications never show amounts or
                addresses.
              </Trans>
            </p>
          </div>
        )}
        {fresh !== undefined && (
          <Card className='xc-fresh' data-testid='connected-banner'>
            <CardHeader>
              <CardTitle className='flex items-center gap-2 text-lg'>
                <CheckCircle2 className='h-5 w-5' aria-hidden='true' />
                <Trans>Connected to {freshDomain}</Trans>
              </CardTitle>
              <CardDescription>
                <Trans>
                  Go back to the website and carry on there. When it asks for
                  something, Klimper shows you the request. Your wallet works as
                  usual in the meantime.
                </Trans>
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Button onClick={() => navigate('/wallet')}>
                <Wallet className='h-4 w-4 mr-2' aria-hidden='true' />
                <Trans>Back to my wallet</Trans>
              </Button>
            </CardContent>
          </Card>
        )}

        {waiting > 0 && (
          <Card className='border-primary' data-testid='waiting-banner'>
            <CardHeader>
              <CardTitle className='text-base'>
                <Trans>{waiting} requests are waiting for you</Trans>
              </CardTitle>
            </CardHeader>
            <CardContent>
              <Button onClick={() => navigate(XCHONNECT_ROUTES.request)}>
                <Trans>See what is asked</Trans>
              </Button>
            </CardContent>
          </Card>
        )}

        {(problem ?? error) !== null && (
          <Card className='border-destructive'>
            <CardHeader>
              <CardDescription className='break-words text-destructive'>
                {problem ?? error}
              </CardDescription>
            </CardHeader>
          </Card>
        )}

        {!loading && sessions.length === 0 && (
          <Card data-testid='empty-state'>
            <CardHeader>
              <CardTitle className='text-base'>
                <Trans>No websites are connected</Trans>
              </CardTitle>
              <CardDescription>
                <Trans>
                  A connected website can ask you to sign — and you decide every
                  time. Open the website, press its connect button, and scan or
                  paste the code it shows.
                </Trans>
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Button onClick={() => navigate(XCHONNECT_ROUTES.connect)}>
                <Trans>Connect a website</Trans>
              </Button>
            </CardContent>
          </Card>
        )}

        {sessions.map((session) => (
          <Dapp
            key={session.sessionId}
            session={session}
            walletChainId={status?.chainId ?? null}
            fresh={session.sessionId === justConnected}
            onEnd={(id) => void end(id)}
            onRotate={(id) => void rotate(id)}
          />
        ))}

        {status !== null && (
          <p className='text-xs text-muted-foreground'>
            <Trans>
              Connection keys live in the system keychain under {keyringService}
              , separate from the keys that sign your money.
            </Trans>
            {status.developerModeAllowed && (
              <>
                {' '}
                <Trans>
                  Development build: local relays without encryption in transit
                  are accepted.
                </Trans>
              </>
            )}
          </p>
        )}
      </Container>
    </Layout>
  );
}
