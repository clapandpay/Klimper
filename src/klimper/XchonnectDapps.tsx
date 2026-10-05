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
  CheckCircle2,
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
  fresh,
  onEnd,
}: {
  readonly session: SessionSummary;
  readonly fresh: boolean;
  readonly onEnd: (id: string) => void;
}) {
  const navigate = useNavigate();
  const limits = limitCount(session);
  const connected = new Date(session.createdAt * 1000).toLocaleDateString();
  const epoch = session.epoch;
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
        {session.needsRepair && (
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

        {!session.sasConfirmed && !session.needsRepair && (
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
            Rotation is driven by the dApp and accepted automatically by the session
            loop; the epoch is how the user sees it happen. A user-initiated rotation
            needs a plugin command that does not exist yet (TASK-084 AC #3).
          */}
          <Fact label={<Trans>Connection keys</Trans>}>
            <Trans>renewed {epoch} times</Trans>
          </Fact>
        </dl>

        <div className='flex flex-wrap gap-2'>
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
          <Button
            variant='outline'
            size='sm'
            onClick={() => onEnd(session.sessionId)}
          >
            <Trans>End connection</Trans>
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
            fresh={session.sessionId === justConnected}
            onEnd={(id) => void end(id)}
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
