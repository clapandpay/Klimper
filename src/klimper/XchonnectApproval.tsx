// SPDX-License-Identifier: AGPL-3.0-or-later
// The approval screen (clapandpay TASK-080, TASK-085). Replaces the approval half of the
// placeholder `XchonnectSessions.tsx`.
//
// This screen is the product's safety promise, so it is worth being explicit about what it
// does and does not render:
//
//   * it renders a `PromptView`, which can only be built by `readPrompt` from the
//     wallet's own local simulation. The dApp's description of its request has no path
//     here, and neither does the name it gave itself — only the verified domain is shown;
//   * `conditional_received` is a line of its own with its own sentence, never part of a
//     total (`XchonnectPromptView`);
//   * an unrecognised puzzle needs a second, separate confirmation that shows the puzzle
//     hash: approving a request and approving "I accept a contract nobody can read" are
//     two different decisions;
//   * a `kind: 'signature'` approval is answered on its own. There is deliberately no
//     control that answers more than one approval, so a plan with three signatures asks
//     three times (spec 11.1 item 4, the desktop form of it — see `docs/xchonnect.md`);
//   * doing nothing is a refusal. The approval bridge in Rust treats five minutes of
//     silence, a closed webview and a session that ended as "do not sign".

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
import { Trans } from '@lingui/react/macro';
import { KeyRound } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { XchonnectPromptView } from './XchonnectPromptView';
import { Approval, xchonnect } from './xchonnect';
import { needsSecondConfirmation, readPrompt } from './xchonnectPrompt';
import {
  subscribeXchonnectLink,
  takeXchonnectLink,
  XCHONNECT_ROUTES,
} from './xchonnectLinkHandoff';
import { useXchonnectApprovals } from './useXchonnectApprovals';
import { returnToDapp } from './xchonnectReturn';

function minutesLeft(expiresAt: number): number {
  return Math.max(0, Math.ceil((expiresAt - Date.now() / 1000) / 60));
}

/** The verified domain, and only that: this line sits next to amounts. */
function AsksYouToSign({ dapp }: { readonly dapp: string }) {
  return <Trans>{dapp} asks you to sign</Trans>;
}

function RefusedIn({ minutes }: { readonly minutes: number }) {
  return <Trans>refused by itself in {minutes} minutes</Trans>;
}

/** One signature out of an approved plan. Its own decision, every time. */
function SignatureApproval({
  approval,
  onAnswer,
}: {
  readonly approval: Approval & { kind: 'signature' };
  readonly onAnswer: (approved: boolean) => void;
}) {
  const index = approval.index;
  const total = approval.total;
  return (
    <CardContent className='space-y-4' data-testid='signature-approval'>
      <p className='text-sm text-muted-foreground'>
        <Trans>
          You approved the request. Each signature it needs is authorised on its
          own.
        </Trans>
      </p>
      <div className='flex items-start gap-2'>
        <KeyRound className='h-4 w-4 mt-1 shrink-0' aria-hidden='true' />
        <div>
          <div className='text-sm text-muted-foreground'>
            <Trans>Signing key</Trans>
          </div>
          <div
            className='font-mono text-xs break-all'
            data-testid='signature-key'
          >
            {approval.publicKey}
          </div>
        </div>
      </div>
      <div className='flex gap-2'>
        <Button onClick={() => onAnswer(true)}>
          <Trans>
            Authorise signature {index} of {total}
          </Trans>
        </Button>
        <Button variant='outline' onClick={() => onAnswer(false)}>
          <Trans>Reject</Trans>
        </Button>
      </div>
    </CardContent>
  );
}

export default function XchonnectApproval() {
  const navigate = useNavigate();
  const { approvals, current, error, answer, reload } = useXchonnectApprovals();
  const [confirming, setConfirming] = useState(false);
  /** A `/req` link arrived with nothing pending: say so, neutrally. */
  const [cameFromLink, setCameFromLink] = useState(false);
  /** The domain of the last request answered here, for the "you can go back" state. */
  const [answered, setAnswered] = useState<string | null>(null);

  // A same-device request hint. The hint itself is not acted on: `listSessions` resumes
  // every stored session's message loop, which is what actually fetches the mailbox, and
  // only an authenticated envelope can produce an approval.
  useEffect(() => {
    const act = (kind: string | undefined) => {
      if (kind !== 'request') return;
      setCameFromLink(true);
      void xchonnect
        .listSessions()
        .then(reload)
        .catch(() => undefined);
    };
    act(takeXchonnectLink()?.kind);
    return subscribeXchonnectLink((link) => act(link.kind));
  }, [reload]);

  const view = useMemo(
    () =>
      current !== null && current.kind === 'request'
        ? readPrompt(current.prompt)
        : null,
    [current],
  );

  useEffect(() => setConfirming(false), [current?.id]);

  const resolve = (approved: boolean) => {
    if (current === null) return;
    const dapp = current.dapp;
    void answer(current.id, approved).then(() =>
      // Integration guide 5.4: with no usable return URL the user is left here with a
      // clear "you can go back now" state rather than a guessed navigation. The guard
      // that would open one lives in `xchonnectReturn.ts`.
      returnToDapp(undefined, dapp).then((outcome) => {
        if (outcome !== 'opened') setAnswered(dapp);
      }),
    );
    setConfirming(false);
  };

  const onApprove = () => {
    if (view !== null && needsSecondConfirmation(view)) {
      setConfirming(true);
      return;
    }
    resolve(true);
  };

  const waiting = approvals.length - 1;

  return (
    <Layout>
      <Header
        title={t`Approve a request`}
        back={() => navigate(XCHONNECT_ROUTES.dapps)}
      />
      <Container className='max-w-xl space-y-4'>
        {error !== null && (
          <Card className='border-destructive'>
            <CardHeader>
              <CardDescription className='break-words text-destructive'>
                {error}
              </CardDescription>
            </CardHeader>
          </Card>
        )}

        {current === null && answered !== null && (
          <Card data-testid='go-back-notice'>
            <CardHeader>
              <CardTitle className='text-base'>
                <Trans>Answered</Trans>
              </CardTitle>
              <CardDescription>
                <Trans>
                  {answered} has your answer. You can go back to it now.
                </Trans>
              </CardDescription>
            </CardHeader>
          </Card>
        )}

        {current === null && (
          <Card data-testid='nothing-waiting'>
            <CardHeader>
              <CardTitle className='text-base'>
                <Trans>Nothing is waiting for you</Trans>
              </CardTitle>
              <CardDescription>
                {cameFromLink ? (
                  <Trans>
                    A website asked for your signature, but nothing has arrived
                    yet. Klimper is checking its mailbox; if nothing appears,
                    ask the website to try again.
                  </Trans>
                ) : (
                  <Trans>
                    When a connected website asks you to sign something, it
                    appears here.
                  </Trans>
                )}
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Button
                variant='outline'
                onClick={() => navigate(XCHONNECT_ROUTES.dapps)}
              >
                <Trans>Connected websites</Trans>
              </Button>
            </CardContent>
          </Card>
        )}

        {current !== null && (
          <Card className='border-primary'>
            <CardHeader>
              <CardTitle className='text-base break-all'>
                {/*
                  The verified domain, and nothing the website chose to call itself:
                  this header is next to amounts, and a claimed name next to an amount
                  is a claim about the amount.
                */}
                <AsksYouToSign dapp={current.dapp} />
              </CardTitle>
              <CardDescription>
                {current.method} ·{' '}
                <RefusedIn minutes={minutesLeft(current.expiresAt)} />
                {waiting > 0 && (
                  <>
                    {' · '}
                    <span data-testid='queue-count'>
                      <Trans>{waiting} more waiting</Trans>
                    </span>
                  </>
                )}
              </CardDescription>
            </CardHeader>

            {current.kind === 'signature' ? (
              <SignatureApproval approval={current} onAnswer={resolve} />
            ) : (
              <CardContent className='space-y-4'>
                {view !== null && <XchonnectPromptView view={view} />}
                <p className='text-xs text-muted-foreground'>
                  <Trans>
                    If you do nothing, this request is refused. Approving asks
                    you again for every signature it needs.
                  </Trans>
                </p>
                <div className='flex gap-2'>
                  <Button
                    onClick={onApprove}
                    disabled={view?.kind === 'unreadable'}
                  >
                    <Trans>Approve</Trans>
                  </Button>
                  <Button variant='outline' onClick={() => resolve(false)}>
                    <Trans>Reject</Trans>
                  </Button>
                </div>
              </CardContent>
            )}
          </Card>
        )}

        {/*
          The second confirmation. Separate decision, separate words, and the puzzle hash
          in full: there is nothing else anybody can tell the user about this contract.
        */}
        <Dialog open={confirming} onOpenChange={setConfirming}>
          <DialogContent data-testid='unknown-contract-confirm'>
            <DialogHeader>
              <DialogTitle className='text-destructive'>
                <Trans>This spend uses a contract Klimper cannot read</Trans>
              </DialogTitle>
              <DialogDescription>
                <Trans>
                  Nobody can tell you what it does with your coins. Approve only
                  if you know this contract from somewhere other than this
                  website.
                </Trans>
              </DialogDescription>
            </DialogHeader>
            {view?.kind === 'spend' &&
              view.unknownPuzzles.map((hash) => (
                <div
                  key={hash}
                  className='font-mono text-xs break-all bg-muted p-2 rounded'
                >
                  {hash}
                </div>
              ))}
            <DialogFooter>
              <Button variant='outline' onClick={() => setConfirming(false)}>
                <Trans>Go back</Trans>
              </Button>
              <Button variant='destructive' onClick={() => resolve(true)}>
                <Trans>I accept the unknown contract</Trans>
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </Container>
    </Layout>
  );
}
