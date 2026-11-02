// SPDX-License-Identifier: AGPL-3.0-or-later
// The queue of things waiting for the user (clapandpay TASK-080).
//
// Two sources, and both are needed:
//
//   * the `approval` / `approval-resolved` events, for an approval that arrives while the
//     app is open;
//   * `pendingApprovals()`, because the webview can reload (a dev-server refresh, a
//     crash, a theme change) while a request is parked on the approval bridge in Rust.
//     Without the rebuild the request would wait out its five minutes and be refused with
//     the user sitting in front of it.
//
// Order is arrival order, and only the first one is ever rendered: a queue the user can
// answer out of order is a queue where the wrong thing gets approved.
//
// Phones (clapandpay TASK-125.04): a request can reach the wallet while the app is in the
// background or the phone is locked, when the webview hears no event. The queue is read again
// whenever the app comes to the front, and once more a moment later, for a request the
// message loop fetches right as the app resumes.
//
// A fee choice (`chooseFee`) withdraws the approval on screen (`replaced: true`) and the
// wallet rebuilds the send: the approval stays in its place until the new one for the same
// request arrives and takes it, so the screen neither closes nor reads as an answer.

import { useBiometric } from '@/hooks/useBiometric';
import { xchonnectEnabled } from '@/lib/features';
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  Approval,
  onXchonnectEvent,
  xchonnect,
  XCHONNECT_EVENTS,
} from './xchonnect';

export interface ApprovalQueue {
  readonly approvals: readonly Approval[];
  /** The one being decided. */
  readonly current: Approval | null;
  readonly error: string | null;
  /**
   * Answer one approval. The queue drops it either way; `false` means the wallet no longer
   * had it (timed out, withdrawn, session ended), so nothing was signed.
   */
  readonly answer: (id: string, approved: boolean) => Promise<Answered>;
  /** Withdraw one approval; the website is told it was cancelled. `false` as for `answer`. */
  readonly cancel: (id: string) => Promise<boolean>;
  readonly reload: () => Promise<void>;
}

/** The second look after coming to the front. */
const RELOAD_AGAIN_MS = 1500;

/** How long a replaced approval waits for its rebuilt send before it is dropped. */
const REPLACE_WAIT_MS = 30_000;

/** How an answer ended: sent, no longer waiting, or not confirmed by Face ID. */
export type Answered = 'answered' | 'gone' | 'unconfirmed';

export function useXchonnectApprovals(): ApprovalQueue {
  const [approvals, setApprovals] = useState<Approval[]>([]);
  const [error, setError] = useState<string | null>(null);

  /** Approvals withdrawn for a fee choice, by id, while their rebuilt send is on its way. */
  const replacing = useRef(new Map<string, ReturnType<typeof setTimeout>>());

  const reload = useCallback(async () => {
    if (!xchonnectEnabled) return;
    try {
      const pending = await xchonnect.pendingApprovals();
      // A replaced one stays until its successor is listed (or it waited too long).
      setApprovals((current) => [
        ...current.filter(
          (item) =>
            replacing.current.has(item.id) &&
            !pending.some((next) => next.requestId === item.requestId),
        ),
        ...pending,
      ]);
    } catch (problem) {
      setError(String(problem));
    }
  }, []);

  useEffect(() => {
    void reload();
    let later: ReturnType<typeof setTimeout> | undefined;
    const onFront = () => {
      if (document.visibilityState !== 'visible') return;
      void reload();
      clearTimeout(later);
      later = setTimeout(() => void reload(), RELOAD_AGAIN_MS);
    };
    document.addEventListener('visibilitychange', onFront);
    window.addEventListener('focus', onFront);
    const waits = replacing.current;
    const stopWaiting = (id: string) => {
      clearTimeout(waits.get(id));
      waits.delete(id);
    };
    const drop = (id: string) => {
      stopWaiting(id);
      setApprovals((current) => current.filter((item) => item.id !== id));
    };
    const subscriptions = [
      onXchonnectEvent<Approval>(XCHONNECT_EVENTS.approval, (approval) =>
        setApprovals((current) => {
          if (current.some((item) => item.id === approval.id)) return current;
          // The rebuilt send of a replaced approval takes its place in the queue.
          const replaced = current.findIndex(
            (item) =>
              waits.has(item.id) && item.requestId === approval.requestId,
          );
          // Its wait timer is left to run out: dropping an id no longer queued is a no-op.
          if (replaced === -1) return [...current, approval];
          return current.map((item, index) =>
            index === replaced ? approval : item,
          );
        }),
      ),
      onXchonnectEvent<{ id: string; replaced?: boolean }>(
        XCHONNECT_EVENTS.approvalResolved,
        (payload) => {
          if (payload.replaced === true) {
            stopWaiting(payload.id);
            waits.set(
              payload.id,
              setTimeout(() => drop(payload.id), REPLACE_WAIT_MS),
            );
            return;
          }
          drop(payload.id);
        },
      ),
    ];
    return () => {
      clearTimeout(later);
      for (const timer of waits.values()) clearTimeout(timer);
      waits.clear();
      document.removeEventListener('visibilitychange', onFront);
      window.removeEventListener('focus', onFront);
      for (const subscription of subscriptions) {
        void subscription.then((stop) => stop());
      }
    };
  }, [reload]);

  const { promptIfEnabled } = useBiometric();

  /**
   * `'unconfirmed'`: an approval that Sage's "Require biometrics" did not confirm. Nothing was
   * sent, and the request is still waiting, so the person can try again. Every signing path
   * of Sage asks the same way (`ConfirmationDialog`, WalletConnect, offers); without it the
   * slider alone would sign (TASK-126 audit).
   */
  const answer = useCallback(
    async (id: string, approved: boolean): Promise<Answered> => {
      if (approved && !(await promptIfEnabled())) return 'unconfirmed';
      let taken = true;
      try {
        await xchonnect.resolveApproval(id, approved);
      } catch (problem) {
        // The approval timed out, was withdrawn or the session ended while the screen was
        // open. It is no longer waiting, and nothing was signed.
        setError(String(problem));
        taken = false;
      }
      setApprovals((current) => current.filter((item) => item.id !== id));
      return taken ? 'answered' : 'gone';
    },
    [promptIfEnabled],
  );

  const cancel = useCallback(async (id: string) => {
    let taken = true;
    try {
      await xchonnect.cancelApproval(id);
    } catch (problem) {
      // Already answered, timed out or ended: it is no longer waiting either way.
      setError(String(problem));
      taken = false;
    }
    setApprovals((current) => current.filter((item) => item.id !== id));
    return taken;
  }, []);

  return {
    approvals,
    current: approvals[0] ?? null,
    error,
    answer,
    cancel,
    reload,
  };
}
