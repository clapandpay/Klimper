// SPDX-License-Identifier: AGPL-3.0-or-later
// The one view of a request that waits for the user (clapandpay TASK-080).
//
// The overlay that opens over any screen and the detail pane of the requests page both
// render this component and nothing else, so a request looks the same, and is answered
// the same way, wherever the user meets it. What it renders, and what it refuses to:
//
//   * a `PromptView`, which can only be built by `readPrompt` from the wallet's own local
//     simulation. The dApp's description of its request has no path here, and neither does
//     the name it gave itself — only the verified domain is shown. A declared intent is
//     shown only as the claims the wallet verified;
//   * `conditional_received` is a line of its own with its own sentence, never part of a
//     total (`XchonnectPromptView`);
//   * an unrecognised puzzle needs a second, separate confirmation that shows the puzzle
//     hash: approving a request and approving "I accept a contract nobody can read" are
//     two different decisions;
//   * one decision per request (xchonnect-wiki 04): approving signs every signature the
//     request needs, after one unlock, and — for a request Klimper broadcasts — sends it;
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
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import { Clock, X } from 'lucide-react';
import { ReactNode, useEffect, useMemo, useRef, useState } from 'react';
import { Approval } from './xchonnect';
import { promptHeadline, XchonnectPromptView } from './XchonnectPromptView';
import { needsSecondConfirmation, readPrompt } from './xchonnectPrompt';
import { SlideToApprove } from './SlideToApprove';
import { Hint, SiteCoin } from './XchonnectUi';
import { useTicker } from './useTicker';

/** How long the buttons stay off after one request replaces another on screen. */
const SWAP_SETTLE_MS = 700;

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

/** "4:31" until the request is refused by itself, urgent in the last minute. */
export function Countdown({ expiresAt }: { readonly expiresAt: number }) {
  const left = useSecondsLeft(expiresAt);
  const time = clock(left);
  return (
    <span
      className='xc-countdown'
      data-urgent={left <= 60}
      data-testid='approval-countdown'
      title={t`Refused by itself in ${time}`}
    >
      <Clock className='h-3.5 w-3.5 shrink-0' aria-hidden='true' />
      <span className='sr-only'>
        <Trans>refused by itself in</Trans>{' '}
      </span>
      {time}
    </span>
  );
}

/** What a waiting approval is, in a few words and with no amount in it. */
export function approvalHeadline(approval: Approval): ReactNode {
  return promptHeadline(readPrompt(approval.prompt));
}

interface ApprovalDetailProps {
  readonly approval: Approval;
  /** How many other approvals are waiting; shown, never answerable from here. */
  readonly waiting: number;
  /** Answer this approval. Resolves once the wallet has the answer. */
  readonly onAnswer: (approved: boolean) => void;
  /** Withdraw the request instead of answering it; the website is told it was cancelled. */
  readonly onCancel?: () => void;
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
  onCancel,
  aside,
  footer,
  titleId,
}: ApprovalDetailProps) {
  const [confirming, setConfirming] = useState(false);
  // Puts the slider back after the second confirmation was left without approving.
  const [slides, setSlides] = useState(0);
  // One answer per request: after a press, every button waits for the next request.
  const [busy, setBusy] = useState(false);
  // A request that takes the place of the one on screen (the website withdrew the first,
  // or it timed out) keeps the buttons off for a moment, so a tap meant for the first
  // cannot land on the second.
  const [settling, setSettling] = useState(false);
  const shownId = useRef(approval.id);
  const ticker = useTicker();
  const view = useMemo(
    () => readPrompt(approval.prompt, { xchLabel: ticker }),
    [approval, ticker],
  );

  useEffect(() => {
    setConfirming(false);
    setBusy(false);
    if (shownId.current === approval.id) return;
    shownId.current = approval.id;
    setSettling(true);
    const timer = setTimeout(() => setSettling(false), SWAP_SETTLE_MS);
    return () => clearTimeout(timer);
  }, [approval.id]);

  // Off from the very render that shows a new request, before any effect has run: a slide
  // or tap meant for the request before cannot land on this one.
  const swapped = shownId.current !== approval.id;
  // Out of time: the wallet refuses it now (the bridge checks the deadline too).
  const expired = useSecondsLeft(approval.expiresAt) <= 0;
  const disabled = busy || settling || swapped || expired;

  /** One answer per request. */
  const once = (act: () => void) => {
    if (disabled) return;
    setConfirming(false);
    setBusy(true);
    act();
  };
  const answer = (approved: boolean) => once(() => onAnswer(approved));

  const onApprove = () => {
    if (needsSecondConfirmation(view)) {
      setConfirming(true);
      return;
    }
    answer(true);
  };

  const broadcasts = view.kind === 'spend' && view.broadcasts;
  const approveLabel = broadcasts ? t`Approve and send` : t`Approve`;

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
            {approval.dapp}
            <span className='sr-only'>
              {' '}
              <Trans>asks you to sign</Trans>
            </span>
          </h2>
          <div className='flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground'>
            <span className='xc-chip' title={approval.method}>
              {approvalHeadline(approval)}
            </span>
            <Countdown expiresAt={approval.expiresAt} />
            {waiting > 0 && (
              <span className='text-xs' data-testid='queue-count'>
                <Trans>+{waiting} waiting</Trans>
              </span>
            )}
            <Hint>
              {broadcasts ? (
                <Trans>
                  If you do nothing, this request is refused when the time runs
                  out. Approving signs it and Klimper sends it to the network.
                </Trans>
              ) : (
                <Trans>
                  If you do nothing, this request is refused when the time runs
                  out.
                </Trans>
              )}
            </Hint>
          </div>
        </div>
        {aside}
      </header>

      <div className='xc-approval-body'>
        <XchonnectPromptView view={view} />
      </div>

      <footer className='xc-approval-actions'>
        <SlideToApprove
          label={approveLabel}
          hint={
            broadcasts ? (
              <Trans>Slide to approve and send</Trans>
            ) : (
              <Trans>Slide to approve</Trans>
            )
          }
          onConfirm={onApprove}
          disabled={disabled || view.kind === 'unreadable'}
          resetKey={`${approval.id}:${slides}`}
          testId='approval-slide'
        />
        <div className='flex gap-2'>
          <Button
            variant='outline'
            className='flex-1 rounded-full'
            onClick={() => answer(false)}
            disabled={disabled}
          >
            <X className='h-4 w-4 mr-1.5' aria-hidden='true' />
            <Trans>Reject</Trans>
          </Button>
          {onCancel && (
            <Button
              variant='ghost'
              className='rounded-full'
              disabled={disabled}
              onClick={() => once(onCancel)}
              title={t`Withdraw it without answering. The website is told it was cancelled.`}
              data-testid='approval-cancel'
            >
              <Trans>Withdraw</Trans>
            </Button>
          )}
        </div>
      </footer>
      {footer}

      {/*
        The second confirmation. Separate decision, separate words, and the puzzle hash in
        full: there is nothing else anybody can tell the user about this contract.
      */}
      <Dialog
        open={confirming}
        onOpenChange={(open) => {
          setConfirming(open);
          if (!open) setSlides((n) => n + 1);
        }}
      >
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
          {view.kind === 'spend' &&
            view.unknownPuzzles.map((hash) => (
              <div
                key={hash}
                className='font-mono text-xs break-all bg-muted p-2 rounded'
              >
                {hash}
              </div>
            ))}
          <DialogFooter>
            <Button
              variant='outline'
              onClick={() => {
                setConfirming(false);
                setSlides((n) => n + 1);
              }}
            >
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
