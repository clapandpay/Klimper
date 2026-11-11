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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { t } from '@lingui/core/macro';
import { Plural, Trans } from '@lingui/react/macro';
import {
  AlertTriangle,
  BellOff,
  CheckCircle2,
  KeyRound,
  Plus,
  Settings2,
  Trash2,
  Unplug,
  Wallet,
} from 'lucide-react';
import { useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { Hint } from '../Hint';
import { allowsOptions } from './optionMethods';
import { identityOfSession, XchonnectDomain } from './XchonnectDomain';
import { SessionSummary, xchonnect } from './xchonnect';
import { XCHONNECT_ROUTES } from './xchonnectLinkHandoff';
import { useXchonnectApprovals } from './useXchonnectApprovals';
import { useXchonnectPush } from './useXchonnectPush';
import { useXchonnectSessions } from './useXchonnectSessions';
import { LivePill, SiteCoin } from './XchonnectUi';

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

/** One line of trouble on a row: what is wrong in a few words, the why behind the dot. */
function RowStatus({
  testId,
  tone,
  title,
  children,
}: {
  readonly testId: string;
  readonly tone: 'danger' | 'note';
  readonly title: React.ReactNode;
  readonly children: React.ReactNode;
}) {
  return (
    <div
      className={`flex items-center gap-1.5 text-xs ${tone === 'danger' ? 'text-destructive' : 'text-foreground'}`}
      role={tone === 'danger' ? 'alert' : 'status'}
      data-testid={testId}
    >
      <AlertTriangle className='h-3.5 w-3.5 shrink-0' aria-hidden='true' />
      <span className='font-medium'>{title}</span>
      <Hint>{children}</Hint>
    </div>
  );
}

/** What a website may do, for the permissions dot: its methods and its two switches. */
function Permissions({ session }: { readonly session: SessionSummary }) {
  return (
    <div className='space-y-2' data-testid='permissions'>
      <div>
        <div className='xc-label'>
          <Trans>May call</Trans>
        </div>
        <div className='font-mono text-xs break-words'>
          {session.methods.join(', ')}
        </div>
      </div>
      <div>
        {session.allReceiveKeys ? (
          <Trans>Sees your receive addresses.</Trans>
        ) : (
          <Trans>Sees one address of its own.</Trans>
        )}{' '}
        {allowsOptions(session.methods) ? (
          <Trans>May ask for option contracts and loans.</Trans>
        ) : (
          <Trans>No option contracts or loans.</Trans>
        )}
      </div>
      <div>
        <Trans>Every request still asks you first.</Trans>
      </div>
    </div>
  );
}

/** A small square button whose name is its label, shown on hover. */
function RowAction({
  label,
  onClick,
  disabled = false,
  children,
}: {
  readonly label: string;
  readonly onClick: () => void;
  readonly disabled?: boolean;
  readonly children: React.ReactNode;
}) {
  return (
    <Button
      variant='ghost'
      size='icon'
      className='h-8 w-8'
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </Button>
  );
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
  readonly onEnd: (session: SessionSummary) => void;
  readonly onRotate: (id: string) => void;
}) {
  const navigate = useNavigate();
  const limits = limitCount(session);
  const connected = new Date(session.createdAt * 1000).toLocaleDateString();
  const sessionChain = session.chainId;
  const epoch = session.epoch;
  const methodCount = session.methods.length;
  // Renewing needs the message loop to carry the offer and the dApp to answer it.
  const usable =
    session.sasConfirmed && !session.needsRepair && !session.revoked;
  const canRotate = usable && session.connected && !session.rotating;
  return (
    <li
      data-testid='dapp-row'
      className={`flex items-start gap-3 px-3 py-2.5 ${fresh ? 'xc-fresh rounded-lg' : ''}`}
    >
      <SiteCoin domain={session.domainUnicode} size='sm' />
      <div className='min-w-0 flex-1 space-y-1'>
        <div className='flex items-start gap-2'>
          <div className='min-w-0 flex-1 text-sm'>
            <XchonnectDomain
              identity={identityOfSession(session)}
              size='inline'
              showName
            />
          </div>
          <LivePill live={session.connected} compact />
        </div>

        <div className='flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground'>
          <span>{session.chainId}</span>
          <span aria-hidden='true'>·</span>
          <span className='inline-flex items-center gap-1'>
            <Plural
              value={methodCount}
              one='# permission'
              other='# permissions'
            />
            <Hint label={t`What this website may do`}>
              <Permissions session={session} />
            </Hint>
          </span>
          <span aria-hidden='true'>·</span>
          <span>
            {limits === 0 ? (
              <Trans>no limits</Trans>
            ) : (
              <Plural value={limits} one='# limit' other='# limits' />
            )}
          </span>
          <span aria-hidden='true'>·</span>
          <span>
            <Trans>since {connected}</Trans>
          </span>
          <span aria-hidden='true'>·</span>
          {/*
            Either side may renew the keys (spec 9.2.1); the epoch counts how often
            that has happened. "Being renewed" lasts until the website has answered
            and the old mailbox has been read to the end.
          */}
          <span>
            {session.rotating ? (
              <Trans>keys being renewed</Trans>
            ) : (
              <Trans>keys renewed {epoch} times</Trans>
            )}
          </span>
        </div>

        {session.revoked && (
          <RowStatus
            testId='revoked'
            tone='danger'
            title={<Trans>Klimper ended this connection</Trans>}
          >
            <Trans>
              The website withdrew the key this connection was made with, which
              is what a website does when that key may have been stolen. Nothing
              can be asked through it any more. Connect again if you still use
              the website.
            </Trans>
          </RowStatus>
        )}

        {walletChainId !== null &&
          session.chainId !== walletChainId &&
          !session.revoked && (
            <RowStatus
              testId='other-network'
              tone='note'
              title={<Trans>Made on {sessionChain}</Trans>}
            >
              <Trans>
                Klimper is on {walletChainId} now. The website sees which
                network Klimper is on, and can ask nothing else until you switch
                back. To use it on {walletChainId}, connect again.
              </Trans>
            </RowStatus>
          )}

        {session.needsRepair && !session.revoked && (
          <RowStatus
            testId='needs-repair'
            tone='danger'
            title={<Trans>This connection cannot be used</Trans>}
          >
            <Trans>
              Its stored state was lost or restored from a backup. Klimper will
              not reuse it. Connect to the website again.
            </Trans>
          </RowStatus>
        )}

        {!session.sasConfirmed && !session.needsRepair && !session.revoked && (
          <RowStatus
            testId='sas-unconfirmed'
            tone='danger'
            title={<Trans>Not confirmed</Trans>}
          >
            <Trans>
              The six digits were never confirmed, so this connection cannot be
              used.
            </Trans>
          </RowStatus>
        )}

        {(session.allowAggSigUnsafe || session.allowUnknownContracts) && (
          <RowStatus
            testId='dangerous-switch-warning'
            tone='danger'
            title={
              session.allowAggSigUnsafe && session.allowUnknownContracts ? (
                <Trans>
                  Unsafe signatures and unreadable contracts allowed
                </Trans>
              ) : session.allowAggSigUnsafe ? (
                <Trans>Unsafe signatures allowed</Trans>
              ) : (
                <Trans>Unreadable contracts allowed</Trans>
              )
            }
          >
            {session.allowAggSigUnsafe && session.allowUnknownContracts ? (
              <Trans>
                You allowed this website unsafe signatures and contracts Klimper
                cannot read.
              </Trans>
            ) : session.allowAggSigUnsafe ? (
              <Trans>You allowed this website unsafe signatures.</Trans>
            ) : (
              <Trans>
                You allowed this website contracts Klimper cannot read.
              </Trans>
            )}
          </RowStatus>
        )}
      </div>

      <div className='flex shrink-0 items-center'>
        {!session.revoked && (
          <RowAction
            label={t`What it may do`}
            onClick={() =>
              navigate(`${XCHONNECT_ROUTES.dapp}/${session.sessionId}`)
            }
          >
            <Settings2 className='h-4 w-4' aria-hidden='true' />
          </RowAction>
        )}
        {usable && (
          <RowAction
            label={t`Renew keys`}
            disabled={!canRotate}
            onClick={() => onRotate(session.sessionId)}
          >
            <KeyRound className='h-4 w-4' aria-hidden='true' />
          </RowAction>
        )}
        <RowAction
          label={session.revoked ? t`Remove` : t`End connection`}
          onClick={() => onEnd(session)}
        >
          {session.revoked ? (
            <Trash2 className='h-4 w-4' aria-hidden='true' />
          ) : (
            <Unplug className='h-4 w-4' aria-hidden='true' />
          )}
        </RowAction>
      </div>
    </li>
  );
}

export default function XchonnectDapps() {
  const navigate = useNavigate();
  const location = useLocation();
  const justConnected = (location.state as DappsState | null)?.connected;
  const { status, sessions, loading, error, refresh } = useXchonnectSessions();
  const { approvals } = useXchonnectApprovals();
  const [problem, setProblem] = useState<string | null>(null);
  // Ending a connection is asked about first: its button is a small icon beside two others,
  // and the website has to be paired again afterwards. Removing an ended one is not.
  const [ending, setEnding] = useState<SessionSummary | null>(null);

  const endingDomain = ending?.domainUnicode ?? '';

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

        {sessions.length > 0 && (
          <Card>
            <ul className='divide-y divide-border' data-testid='dapp-list'>
              {sessions.map((session) => (
                <Dapp
                  key={session.sessionId}
                  session={session}
                  walletChainId={status?.chainId ?? null}
                  fresh={session.sessionId === justConnected}
                  onEnd={(target) =>
                    target.revoked
                      ? void end(target.sessionId)
                      : setEnding(target)
                  }
                  onRotate={(id) => void rotate(id)}
                />
              ))}
            </ul>
          </Card>
        )}

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

        <Dialog
          open={ending !== null}
          onOpenChange={(open) => !open && setEnding(null)}
        >
          <DialogContent data-testid='end-confirm'>
            <DialogHeader>
              <DialogTitle>
                <Trans>End this connection?</Trans>
              </DialogTitle>
              <DialogDescription>
                <Trans>
                  {endingDomain} can no longer ask you to sign. To use it again
                  you connect it anew.
                </Trans>
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant='outline' onClick={() => setEnding(null)}>
                <Trans>Keep it</Trans>
              </Button>
              <Button
                variant='destructive'
                onClick={() => {
                  const target = ending;
                  setEnding(null);
                  if (target) void end(target.sessionId);
                }}
              >
                <Trans>End connection</Trans>
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </Container>
    </Layout>
  );
}
