// SPDX-License-Identifier: AGPL-3.0-or-later
// The one view of a request that waits for the user (clapandpay TASK-080).
//
// The overlay that opens over any screen and the detail pane of the requests page both
// render this component and nothing else, so a request looks the same, and is answered
// the same way, wherever the user meets it. What it renders, and what it refuses to:
//
//   * a `PromptView`, which can only be built by `readPrompt` from the wallet's own local
//     simulation. The dApp's description of its request has no path here, and neither does
//     the name it gave itself — only the verified domain is shown;
//   * `conditional_received` is a line of its own with its own sentence, never part of a
//     total (`XchonnectPromptView`);
//   * an unrecognised puzzle needs a second, separate confirmation that shows the puzzle
//     hash: approving a request and approving "I accept a contract nobody can read" are
//     two different decisions;
//   * a `kind: 'signature'` approval is answered on its own. There is deliberately no
//     control that answers more than one approval, so a plan with three signatures asks
//     three times (spec 11.1 item 4, the desktop form of it — see `docs/xchonnect.md`);
//   * Approve exists only here. A list may show what is waiting, but nothing is approved
//     without this whole view on screen;
//   * doing nothing is a refusal. The approval bridge in Rust treats five minutes of
//     silence, a closed webview and a session that ended as "do not sign", and the
//     countdown says when that happens.

import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Trans } from '@lingui/react/macro';
import { Check, Clock, KeyRound, X } from 'lucide-react';
import { ReactNode, useEffect, useMemo, useState } from 'react';
import { Approval } from './xchonnect';
import { promptHeadline, XchonnectPromptView } from './XchonnectPromptView';
import { needsSecondConfirmation, readPrompt } from './xchonnectPrompt';
import { SiteCoin } from './XchonnectUi';
import { useTicker } from './useTicker';

/** Seconds until the approval bridge refuses the request by itself. */
function secondsLeft(expiresAt: number): number {
  return Math.max(0, Math.floor(expiresAt - Date.now() / 1000));
}

/** Seconds as `m:ss`. */
export function clock(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${String(seconds % 60).padStart(2, '0')}`;
}

/** Re-renders every second while mounted. */
function useSecondsLeft(expiresAt: number): number {
  const [left, setLeft] = useState(() => secondsLeft(expiresAt));
  useEffect(() => {
    setLeft(secondsLeft(expiresAt));
    const timer = window.setInterval(
      () => setLeft(secondsLeft(expiresAt)),
      1000,
    );
    return () => window.clearInterval(timer);
  }, [expiresAt]);
  return left;
}

/** "Refused by itself in 4:31", urgent in the last minute. */
export function Countdown({ expiresAt }: { readonly expiresAt: number }) {
  const left = useSecondsLeft(expiresAt);
  const time = clock(left);
  return (
    <span
      className='xc-countdown'
      data-urgent={left <= 60}
      data-testid='approval-countdown'
    >
      <Clock className='h-3.5 w-3.5 shrink-0' aria-hidden='true' />
      <Trans>refused by itself in {time}</Trans>
    </span>
  );
}

/** What a waiting approval is, in a few words and with no amount in it. */
export function approvalHeadline(approval: Approval): ReactNode {
  if (approval.kind === 'signature') {
    const index = approval.index;
    const total = approval.total;
    return (
      <Trans>
        Signature {index} of {total}
      </Trans>
    );
  }
  return promptHeadline(readPrompt(approval.prompt));
}

function AuthoriseSignature({
  index,
  total,
}: {
  readonly index: number;
  readonly total: number;
}) {
  return (
    <Trans>
      Authorise signature {index} of {total}
    </Trans>
  );
}

/** The verified domain, and only that: this line sits next to amounts. */
function AsksYouToSign({ dapp }: { readonly dapp: string }) {
  return <Trans>{dapp} asks you to sign</Trans>;
}

/** One signature out of an approved plan. Its own decision, every time. */
function SignatureBody({
  approval,
}: {
  readonly approval: Approval & { kind: 'signature' };
}) {
  return (
    <div className='space-y-4' data-testid='signature-approval'>
      <p className='text-sm text-muted-foreground'>
        <Trans>
          You approved the request. Each signature it needs is authorised on its
          own.
        </Trans>
      </p>
      <div className='xc-verdict flex items-start gap-3'>
        <KeyRound className='h-5 w-5 mt-0.5 shrink-0' aria-hidden='true' />
        <div className='min-w-0'>
          <div className='xc-label'>
            <Trans>Signing key</Trans>
          </div>
          <div
            className='font-mono text-xs break-all mt-1'
            data-testid='signature-key'
          >
            {approval.publicKey}
          </div>
        </div>
      </div>
    </div>
  );
}

export interface ApprovalDetailProps {
  readonly approval: Approval;
  /** How many other approvals are waiting; shown, never answerable from here. */
  readonly waiting: number;
  /** Answer this approval. Resolves once the wallet has the answer. */
  readonly onAnswer: (approved: boolean) => void;
  /** Extra controls for the header's right edge (the overlay's "Later"). */
  readonly aside?: ReactNode;
  /** Shown under the actions (the overlay's link to every request). */
  readonly footer?: ReactNode;
  /** The overlay names its title for screen readers through this id. */
  readonly titleId?: string;
}

export function XchonnectApprovalDetail({
  approval,
  waiting,
  onAnswer,
  aside,
  footer,
  titleId,
}: ApprovalDetailProps) {
  const [confirming, setConfirming] = useState(false);
  const ticker = useTicker();
  const view = useMemo(
    () =>
      approval.kind === 'request'
        ? readPrompt(approval.prompt, { xchLabel: ticker })
        : null,
    [approval, ticker],
  );

  useEffect(() => setConfirming(false), [approval.id]);

  const answer = (approved: boolean) => {
    setConfirming(false);
    onAnswer(approved);
  };

  const onApprove = () => {
    if (view !== null && needsSecondConfirmation(view)) {
      setConfirming(true);
      return;
    }
    answer(true);
  };

  const signature = approval.kind === 'signature';

  return (
    <article className='xc-approval' data-testid='approval-detail'>
      <header className='xc-approval-head'>
        <SiteCoin domain={approval.dapp} size='lg' />
        <div className='min-w-0 flex-1 space-y-1.5'>
          <h2
            id={titleId}
            className='xc-approval-title break-all'
            data-testid='approval-title'
          >
            {/*
              The verified domain, and nothing the website chose to call itself: this
              header is next to amounts, and a claimed name next to an amount is a claim
              about the amount.
            */}
            <AsksYouToSign dapp={approval.dapp} />
          </h2>
          <div className='flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground'>
            <span className='xc-chip'>{approvalHeadline(approval)}</span>
            <span className='font-mono text-xs'>{approval.method}</span>
            <Countdown expiresAt={approval.expiresAt} />
            {waiting > 0 && (
              <span className='text-xs' data-testid='queue-count'>
                <Trans>{waiting} more waiting</Trans>
              </span>
            )}
          </div>
        </div>
        {aside}
      </header>

      <div className='xc-approval-body'>
        {signature ? (
          <SignatureBody approval={approval} />
        ) : (
          view !== null && <XchonnectPromptView view={view} />
        )}
      </div>

      <footer className='xc-approval-actions'>
        <p className='text-xs text-muted-foreground sm:max-w-[16rem]'>
          {signature ? (
            <Trans>If you do nothing, this signature is refused.</Trans>
          ) : view?.kind === 'message' ? (
            <Trans>If you do nothing, this request is refused.</Trans>
          ) : (
            <Trans>
              If you do nothing, this request is refused. Approving asks you
              again for every signature it needs.
            </Trans>
          )}
        </p>
        <div className='flex gap-2 sm:ml-auto'>
          <Button
            variant='outline'
            size='lg'
            className='flex-1 sm:flex-none'
            onClick={() => answer(false)}
          >
            <X className='h-4 w-4 mr-1.5' aria-hidden='true' />
            <Trans>Reject</Trans>
          </Button>
          {approval.kind === 'signature' ? (
            <Button
              size='lg'
              className='flex-1 sm:flex-none'
              onClick={() => answer(true)}
            >
              <AuthoriseSignature
                index={approval.index}
                total={approval.total}
              />
            </Button>
          ) : (
            <Button
              size='lg'
              className='flex-1 sm:flex-none'
              onClick={onApprove}
              disabled={view?.kind === 'unreadable'}
            >
              <Check className='h-4 w-4 mr-1.5' aria-hidden='true' />
              <Trans>Approve</Trans>
            </Button>
          )}
        </div>
      </footer>
      {footer}

      {/*
        The second confirmation. Separate decision, separate words, and the puzzle hash in
        full: there is nothing else anybody can tell the user about this contract.
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
            <Button variant='destructive' onClick={() => answer(true)}>
              <Trans>I accept the unknown contract</Trans>
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </article>
  );
}
