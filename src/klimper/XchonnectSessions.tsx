// SPDX-License-Identifier: AGPL-3.0-or-later
// Minimal connected-dApps and approval screen for Xchonnect (clapandpay TASK-077 to
// TASK-079). The polished screens are TASK-080, TASK-083 and TASK-084; this file exists to
// prove the flow end to end and is meant to be replaced.
//
// What must survive any redesign:
//   * only `prompt.summary` and `prompt.plan` are shown — the wallet's own simulation.
//     A dApp's claimed amounts and recipients are not rendered anywhere;
//   * `conditional_received` is labelled as "only if the counterparty completes", never
//     added to what the user receives;
//   * an `is_unsafe` signature is loud;
//   * every signature is confirmed separately.
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
import { AlertTriangle } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  Approval,
  onXchonnectEvent,
  SessionSummary,
  xchonnect,
  XCHONNECT_EVENTS,
  XchonnectStatus,
} from './xchonnect';

interface AssetDelta {
  asset: { type: string; asset_id?: string };
  sent: number | string;
  received: number | string;
  conditional_received: number | string;
  net: number | string;
}

interface Requirement {
  public_key: string;
  is_unsafe: boolean;
  opcode: number;
}

interface PromptShape {
  type?: string;
  summary?: {
    assets?: AssetDelta[];
    implied_fee?: number | null;
    reserve_fee?: number;
    unknown_puzzles?: string[];
    time_locks?: Record<string, unknown>;
  };
  plan?: { ours?: Requirement[]; foreign?: unknown[]; has_unsafe?: boolean };
  partial?: boolean;
  binding?: { all_bound?: boolean; bound_payments?: unknown[] };
  message_text?: string | null;
  message_hex?: string;
  public_key?: string;
}

const assetName = (asset: AssetDelta['asset']) =>
  asset.type === 'xch' ? 'XCH' : `CAT ${asset.asset_id?.slice(0, 12) ?? ''}…`;

function AssetRow({ delta }: { readonly delta: AssetDelta }) {
  const net = String(delta.net);
  const conditional = String(delta.conditional_received);
  return (
    <div className='border rounded p-2'>
      <div className='font-medium'>{assetName(delta.asset)}</div>
      <div
        className={
          Number(delta.net) < 0 ? 'text-destructive' : 'text-foreground'
        }
      >
        <Trans>Net effect</Trans>: {net}
      </div>
      {Number(delta.conditional_received) > 0 && (
        <div className='text-muted-foreground'>
          <Trans>Only if the counterparty completes: {conditional}</Trans>
        </div>
      )}
    </div>
  );
}

function PromptBody({ prompt }: { readonly prompt: PromptShape }) {
  if (prompt.type === 'sign_message') {
    return (
      <div className='space-y-2 text-sm'>
        <p>
          <Trans>This dApp wants you to sign a message with your key.</Trans>
        </p>
        <div className='font-mono text-xs break-all bg-muted p-2 rounded'>
          {prompt.message_text ?? prompt.message_hex}
        </div>
        <div className='text-muted-foreground font-mono text-xs break-all'>
          {prompt.public_key}
        </div>
      </div>
    );
  }

  const assets = prompt.summary?.assets ?? [];
  const unknown = prompt.summary?.unknown_puzzles ?? [];
  const ours = prompt.plan?.ours ?? [];

  return (
    <div className='space-y-3 text-sm'>
      <p className='text-muted-foreground'>
        <Trans>
          This is what Klimper worked out by running the spend itself, not what
          the dApp says it does.
        </Trans>
      </p>
      {assets.length === 0 && (
        <p>
          <Trans>No change to your assets.</Trans>
        </p>
      )}
      {assets.map((delta) => (
        <AssetRow key={assetName(delta.asset)} delta={delta} />
      ))}
      {prompt.partial && (
        <div className='border rounded p-2'>
          <Trans>
            This is one side of a multi-party spend. Your signature depends on
            receiving the counterparty&apos;s payment.
          </Trans>{' '}
          {prompt.binding?.all_bound ? (
            <Trans>Klimper checked that it is bound.</Trans>
          ) : (
            <span className='text-destructive'>
              <Trans>Not bound — this should have been refused.</Trans>
            </span>
          )}
        </div>
      )}
      {(prompt.summary?.reserve_fee ?? 0) > 0 && (
        <div>
          <Trans>Fee reserved</Trans>: {prompt.summary?.reserve_fee}
        </div>
      )}
      {unknown.length > 0 && (
        <div className='flex items-start gap-2 text-destructive'>
          <AlertTriangle className='h-4 w-4 mt-0.5 shrink-0' />
          <div>
            <Trans>Unknown contract</Trans>
            <div className='font-mono text-xs break-all'>
              {unknown.join(', ')}
            </div>
          </div>
        </div>
      )}
      {prompt.plan?.has_unsafe && (
        <div className='flex items-start gap-2 text-destructive font-semibold'>
          <AlertTriangle className='h-4 w-4 mt-0.5 shrink-0' />
          <Trans>
            One signature is unsafe: it would be valid in any spend, not just
            this one. Reject unless you know exactly why it is needed.
          </Trans>
        </div>
      )}
      <div className='text-muted-foreground'>
        <Trans>Signatures you would produce</Trans>: {ours.length}
      </div>
      <details>
        <summary className='cursor-pointer text-xs text-muted-foreground'>
          <Trans>Everything Klimper computed</Trans>
        </summary>
        <pre className='text-[10px] overflow-x-auto bg-muted p-2 rounded mt-1'>
          {JSON.stringify(prompt, null, 2)}
        </pre>
      </details>
    </div>
  );
}

function ApprovalTitle({ approval }: { readonly approval: Approval }) {
  if (approval.kind === 'signature') {
    const index = approval.index;
    const total = approval.total;
    return (
      <Trans>
        Authorise signature {index} of {total}
      </Trans>
    );
  }
  const dapp = approval.dapp;
  return <Trans>{dapp} asks you to approve</Trans>;
}

export default function XchonnectSessions() {
  const navigate = useNavigate();
  const [status, setStatus] = useState<XchonnectStatus | null>(null);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [approvals, setApprovals] = useState<Approval[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await xchonnect.status());
      setSessions(await xchonnect.listSessions());
      setApprovals(await xchonnect.pendingApprovals());
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const subscriptions = [
      onXchonnectEvent<Approval>(XCHONNECT_EVENTS.approval, (approval) =>
        setApprovals((current) =>
          current.some((item) => item.id === approval.id)
            ? current
            : [...current, approval],
        ),
      ),
      onXchonnectEvent<{ id: string }>(
        XCHONNECT_EVENTS.approvalResolved,
        (payload) =>
          setApprovals((current) =>
            current.filter((item) => item.id !== payload.id),
          ),
      ),
      onXchonnectEvent<unknown>(XCHONNECT_EVENTS.sessions, () => {
        void refresh();
      }),
      onXchonnectEvent<unknown>(XCHONNECT_EVENTS.session, () => {
        void refresh();
      }),
    ];
    return () => {
      for (const subscription of subscriptions) {
        void subscription.then((stop) => stop());
      }
    };
  }, [refresh]);

  const answer = async (id: string, approved: boolean) => {
    try {
      await xchonnect.resolveApproval(id, approved);
    } catch (err) {
      setError(String(err));
    }
    setApprovals((current) => current.filter((item) => item.id !== id));
  };

  const end = async (sessionId: string) => {
    try {
      await xchonnect.endSession(sessionId);
    } catch (err) {
      setError(String(err));
    }
    await refresh();
  };

  const keyringService = status?.keyringService ?? '';

  return (
    <>
      <Header title={t`Connected dApps`}>
        <Button size='sm' onClick={() => navigate('/xchonnect/pair')}>
          <Trans>Connect a dApp</Trans>
        </Button>
      </Header>
      <Container className='max-w-2xl space-y-4'>
        {error && (
          <Card className='border-destructive'>
            <CardHeader>
              <CardDescription className='break-words text-destructive'>
                {error}
              </CardDescription>
            </CardHeader>
          </Card>
        )}

        {approvals.map((approval) => (
          <Card key={approval.id} className='border-primary'>
            <CardHeader>
              <CardTitle className='text-base'>
                <ApprovalTitle approval={approval} />
              </CardTitle>
              <CardDescription>
                {approval.dapp} · {approval.method}
              </CardDescription>
            </CardHeader>
            <CardContent className='space-y-3'>
              {approval.kind === 'signature' ? (
                <div className='font-mono text-xs break-all'>
                  {approval.publicKey}
                </div>
              ) : (
                <PromptBody prompt={approval.prompt as PromptShape} />
              )}
              <div className='flex gap-2'>
                <Button onClick={() => void answer(approval.id, true)}>
                  <Trans>Approve</Trans>
                </Button>
                <Button
                  variant='outline'
                  onClick={() => void answer(approval.id, false)}
                >
                  <Trans>Reject</Trans>
                </Button>
              </div>
            </CardContent>
          </Card>
        ))}

        {sessions.length === 0 && approvals.length === 0 && (
          <Card>
            <CardHeader>
              <CardTitle className='text-base'>
                <Trans>No dApps connected</Trans>
              </CardTitle>
              <CardDescription>
                <Trans>
                  Scan a dApp&apos;s pairing code to let it ask you to sign.
                </Trans>
              </CardDescription>
            </CardHeader>
          </Card>
        )}

        {sessions.map((session) => (
          <Card key={session.sessionId}>
            <CardHeader>
              <CardTitle className='text-base break-all'>
                {session.domainUnicode}
              </CardTitle>
              <CardDescription className='space-y-1'>
                {session.domainDiffers && (
                  <div className='font-mono text-xs break-all'>
                    {session.dapp}
                  </div>
                )}
                <div>
                  {session.dappName} · {session.chainId}
                </div>
              </CardDescription>
            </CardHeader>
            <CardContent className='space-y-2 text-sm'>
              {session.warnings.map((warning) => (
                <div
                  key={warning}
                  className='flex items-center gap-2 text-destructive'
                >
                  <AlertTriangle className='h-4 w-4' />
                  {warning}
                </div>
              ))}
              {session.needsRepair && (
                <div className='text-destructive'>
                  <Trans>
                    Stored state was lost or restored from a backup. This
                    connection cannot be used; pair with the dApp again.
                  </Trans>
                </div>
              )}
              <div className='text-muted-foreground'>
                <Trans>Key epoch</Trans> {session.epoch} ·{' '}
                {session.connected ? (
                  <Trans>listening</Trans>
                ) : (
                  <Trans>not listening</Trans>
                )}
              </div>
              <div className='text-muted-foreground'>
                <Trans>May call</Trans>: {session.methods.join(', ')}
              </div>
              <Button
                variant='outline'
                size='sm'
                onClick={() => void end(session.sessionId)}
              >
                <Trans>End connection</Trans>
              </Button>
            </CardContent>
          </Card>
        ))}

        {status && (
          <p className='text-xs text-muted-foreground'>
            <Trans>
              Connection keys are kept in the system keychain under{' '}
              {keyringService}, separate from your signing keys.
            </Trans>
            {status.developerModeAllowed && (
              <>
                {' '}
                <Trans>
                  Development build: local HTTP relays are accepted.
                </Trans>
              </>
            )}
          </p>
        )}
      </Container>
    </>
  );
}
