// SPDX-License-Identifier: AGPL-3.0-or-later
// The approval overlay (clapandpay TASK-080): a request that arrives is shown at once, over
// whatever screen is open, in the same large view as the requests page.
//
// Rendered once, next to the app (`App.tsx`), outside the router: it has to work on every
// screen, including the ones that are not Klimper's own. It also does the two things that
// make a request arrive at all:
//
//   * it resumes every stored session's message loop (`listSessions`) when the app starts
//     and whenever it comes back to the front. Before, only some Xchonnect screens did,
//     so after a restart nothing fetched requests until the user happened to open one;
//   * on the desktop it brings the window to the front when a request arrives, because a
//     request nobody sees is refused after five minutes.
//
// "Later" only hides the overlay: the request keeps waiting, a pill in the corner says so
// and counts down, and the requests page lists it. A new request opens the overlay again.
// The overlay never shows on the requests page itself, which renders the same view.

import { xchonnectEnabled } from '@/lib/features';
import { t } from '@lingui/core/macro';
import { Trans } from '@lingui/react/macro';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { getCurrentWindow, UserAttentionType } from '@tauri-apps/api/window';
import { platform } from '@tauri-apps/plugin-os';
import { Bell } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ExternalLink } from 'lucide-react';
import { toast } from 'react-toastify';
import { mempoolTxUrl } from './explorer';
import { useTicker } from './useTicker';
import {
  onXchonnectEvent,
  SentEvent,
  xchonnect,
  XCHONNECT_EVENTS,
} from './xchonnect';
import { clock, XchonnectApprovalDetail } from './XchonnectApprovalDetail';
import { goToXchonnectRoute, XCHONNECT_ROUTES } from './xchonnectLinkHandoff';
import { useXchonnectApprovals } from './useXchonnectApprovals';
import './xchonnect.css';

/** The app's route, from the hash router's URL. */
function currentRoute(): string {
  return window.location.hash.replace(/^#/, '') || '/';
}

function useCurrentRoute(): string {
  const [route, setRoute] = useState(currentRoute);
  useEffect(() => {
    const update = () => setRoute(currentRoute());
    window.addEventListener('hashchange', update);
    return () => window.removeEventListener('hashchange', update);
  }, []);
  return route;
}

function isDesktop(): boolean {
  try {
    const os = platform();
    return os !== 'ios' && os !== 'android';
  } catch {
    return false;
  }
}

/** Bring the window to the front; at least make the dock icon ask for attention. */
async function bringToFront() {
  if (!isDesktop()) return;
  const window = getCurrentWindow();
  try {
    await window.unminimize();
    await window.show();
    await window.setFocus();
  } catch {
    await window
      .requestUserAttention(UserAttentionType.Critical)
      .catch(() => undefined);
  }
}

/** Resume every stored session's message loop; quiet if no wallet is open yet. */
function resumeSessions() {
  if (!xchonnectEnabled) return;
  void xchonnect.listSessions().catch(() => undefined);
}

function WaitingPill({
  count,
  expiresAt,
  onOpen,
}: {
  readonly count: number;
  readonly expiresAt: number;
  readonly onOpen: () => void;
}) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  const time = clock(Math.max(0, Math.floor(expiresAt - now / 1000)));
  return (
    <button
      type='button'
      className='xc-waiting-pill'
      onClick={onOpen}
      data-testid='approval-waiting-pill'
    >
      <Bell className='h-4 w-4' aria-hidden='true' />
      {count === 1 ? (
        <Trans>1 request waiting · {time}</Trans>
      ) : (
        <Trans>
          {count} requests waiting · {time}
        </Trans>
      )}
    </button>
  );
}

/** "Sent to the network", with where to look the transaction up. */
function SentToast({
  event,
  testnet,
}: {
  readonly event: SentEvent;
  readonly testnet: boolean;
}) {
  const href = mempoolTxUrl(event.transactionId, testnet);
  const dapp = event.dapp;
  return (
    <div className='space-y-1' data-testid='sent-toast'>
      <div>
        <Trans>Klimper sent the transaction for {dapp}.</Trans>
      </div>
      {href && (
        <button
          type='button'
          className='inline-flex items-center gap-1 text-sm underline underline-offset-2'
          onClick={() => void openUrl(href).catch(() => undefined)}
          data-testid='sent-explorer-link'
        >
          <Trans>View on mempoolxch.space</Trans>
          <ExternalLink className='h-3 w-3' aria-hidden='true' />
        </button>
      )}
    </div>
  );
}

export function XchonnectApprovalHost() {
  const route = useCurrentRoute();
  const testnet = useTicker().toUpperCase().startsWith('T');
  const testnetRef = useRef(testnet);
  testnetRef.current = testnet;

  // A transaction Klimper broadcast: say so, and where to look it up.
  useEffect(() => {
    if (!xchonnectEnabled) return;
    const subscription = onXchonnectEvent<SentEvent>(
      XCHONNECT_EVENTS.sent,
      (event) => {
        toast.success(
          <SentToast event={event} testnet={testnetRef.current} />,
          { autoClose: 12_000 },
        );
      },
    );
    return () => {
      void subscription.then((stop) => stop());
    };
  }, []);
  const { approvals, answer, cancel } = useXchonnectApprovals();
  /** Ids the user put off with "Later"; a new arrival opens the overlay again. */
  const [later, setLater] = useState<ReadonlySet<string>>(new Set());
  const seen = useRef<Set<string>>(new Set());

  useEffect(() => {
    resumeSessions();
    const onVisible = () => {
      if (document.visibilityState === 'visible') resumeSessions();
    };
    document.addEventListener('visibilitychange', onVisible);
    window.addEventListener('focus', resumeSessions);
    return () => {
      document.removeEventListener('visibilitychange', onVisible);
      window.removeEventListener('focus', resumeSessions);
    };
  }, []);

  // Something new arrived: show it, and fetch the user if they are elsewhere.
  useEffect(() => {
    const fresh = approvals.filter((item) => !seen.current.has(item.id));
    for (const item of fresh) seen.current.add(item.id);
    if (fresh.length > 0) {
      setLater(new Set());
      if (!document.hasFocus()) void bringToFront();
    }
  }, [approvals]);

  if (!xchonnectEnabled) return null;

  const onRequestsPage = route.startsWith(XCHONNECT_ROUTES.request);
  const current = approvals[0] ?? null;
  if (current === null || onRequestsPage) return null;

  const open = !later.has(current.id);
  const waiting = approvals.length - 1;
  const total = approvals.length;

  const putOff = () => setLater(new Set(approvals.map((item) => item.id)));

  const onAnswer = (approved: boolean) => {
    const dapp = current.dapp;
    void answer(current.id, approved).then(() => {
      if (approvals.length > 1) return;
      if (approved) {
        toast.success(t`${dapp} has your approval.`);
      } else {
        toast.info(t`You rejected the request from ${dapp}.`);
      }
    });
  };

  const onCancel = () => {
    const dapp = current.dapp;
    void cancel(current.id).then(() => {
      if (approvals.length > 1) return;
      toast.info(t`You cancelled the request from ${dapp}.`);
    });
  };

  const seeAll = () => {
    putOff();
    goToXchonnectRoute(`${XCHONNECT_ROUTES.request}/${current.id}`);
  };

  return (
    <>
      <DialogPrimitive.Root
        open={open}
        onOpenChange={(next) => !next && putOff()}
      >
        <DialogPrimitive.Portal>
          <DialogPrimitive.Overlay
            className='xc-overlay-backdrop'
            data-testid='approval-overlay-backdrop'
          />
          <DialogPrimitive.Content
            className='xc-overlay'
            data-testid='approval-overlay'
            aria-describedby={undefined}
            onPointerDownOutside={(event) => event.preventDefault()}
          >
            <DialogPrimitive.Title className='sr-only'>
              <Trans>A website asks you to sign</Trans>
            </DialogPrimitive.Title>
            <XchonnectApprovalDetail
              approval={current}
              waiting={waiting}
              onAnswer={onAnswer}
              onCancel={onCancel}
              aside={
                <button
                  type='button'
                  className='xc-later'
                  onClick={putOff}
                  data-testid='approval-later'
                >
                  <Trans>Later</Trans>
                </button>
              }
              footer={
                <button
                  type='button'
                  className='xc-see-all'
                  onClick={seeAll}
                  data-testid='approval-see-all'
                >
                  {waiting > 0 ? (
                    <Trans>See all {total} requests</Trans>
                  ) : (
                    <Trans>Open in requests</Trans>
                  )}
                </button>
              }
            />
          </DialogPrimitive.Content>
        </DialogPrimitive.Portal>
      </DialogPrimitive.Root>

      {!open && (
        <WaitingPill
          count={approvals.length}
          expiresAt={current.expiresAt}
          onOpen={() => setLater(new Set())}
        />
      )}
    </>
  );
}
