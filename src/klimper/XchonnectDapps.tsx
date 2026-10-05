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
import { AlertTriangle, Plus, Settings2 } from 'lucide-react';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { identityOfSession, XchonnectDomain } from './XchonnectDomain';
import { SessionSummary, xchonnect } from './xchonnect';
import { XCHONNECT_ROUTES } from './xchonnectLinkHandoff';
import { useXchonnectApprovals } from './useXchonnectApprovals';
import { useXchonnectSessions } from './useXchonnectSessions';

function limitCount(session: SessionSummary): number {
  return Object.values(session.limits).filter(
    (limit) =>
      (limit.perRequest ?? null) !== null || (limit.perDay ?? null) !== null,
  ).length;
}

function Dapp({
  session,
  onEnd,
}: {
  readonly session: SessionSummary;
  readonly onEnd: (id: string) => void;
}) {
  const navigate = useNavigate();
  const limits = limitCount(session);
  const connected = new Date(session.createdAt * 1000).toLocaleDateString();
  const epoch = session.epoch;
  return (
    <Card data-testid='dapp-card'>
      <CardHeader>
        <CardTitle className='text-base'>
          <XchonnectDomain
            identity={identityOfSession(session)}
            size='inline'
            showName
          />
        </CardTitle>
      </CardHeader>
      <CardContent className='space-y-3 text-sm'>
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

        <dl className='grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-muted-foreground'>
          <dt>
            <Trans>Connected</Trans>
          </dt>
          <dd className='text-foreground'>{connected}</dd>
          <dt>
            <Trans>Network</Trans>
          </dt>
          <dd className='text-foreground'>{session.chainId}</dd>
          <dt>
            <Trans>May call</Trans>
          </dt>
          <dd className='text-foreground break-all'>
            {session.methods.join(', ')}
          </dd>
          <dt>
            <Trans>Limits</Trans>
          </dt>
          <dd className='text-foreground'>
            {limits === 0 ? (
              <Trans>none set</Trans>
            ) : (
              <Trans>{limits} set</Trans>
            )}
          </dd>
          <dt>
            <Trans>Connection keys</Trans>
          </dt>
          <dd className='text-foreground'>
            {/*
              Rotation is driven by the dApp and accepted automatically by the session
              loop; the epoch is how the user sees it happen. A user-initiated rotation
              needs a plugin command that does not exist yet (TASK-084 AC #3).
            */}
            <Trans>renewed {epoch} times</Trans>
          </dd>
          <dt>
            <Trans>Right now</Trans>
          </dt>
          <dd className='text-foreground'>
            {session.connected ? (
              <Trans>listening for requests</Trans>
            ) : (
              <Trans>not listening</Trans>
            )}
          </dd>
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

  return (
    <>
      <Header title={t`Connected websites`}>
        <Button size='sm' onClick={() => navigate(XCHONNECT_ROUTES.connect)}>
          <Plus className='h-4 w-4 mr-2' aria-hidden='true' />
          <Trans>Connect a website</Trans>
        </Button>
      </Header>
      <Container className='max-w-2xl space-y-4'>
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
    </>
  );
}
