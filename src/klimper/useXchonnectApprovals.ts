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

import { xchonnectEnabled } from '@/lib/features';
import { useCallback, useEffect, useState } from 'react';
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
  /** Answer one approval. The queue drops it either way. */
  readonly answer: (id: string, approved: boolean) => Promise<void>;
  /** Withdraw one approval; the website is told it was cancelled. */
  readonly cancel: (id: string) => Promise<void>;
  readonly reload: () => Promise<void>;
}

export function useXchonnectApprovals(): ApprovalQueue {
  const [approvals, setApprovals] = useState<Approval[]>([]);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    if (!xchonnectEnabled) return;
    try {
      setApprovals(await xchonnect.pendingApprovals());
    } catch (problem) {
      setError(String(problem));
    }
  }, []);

  useEffect(() => {
    void reload();
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
    ];
    return () => {
      for (const subscription of subscriptions) {
        void subscription.then((stop) => stop());
      }
    };
  }, [reload]);

  const answer = useCallback(async (id: string, approved: boolean) => {
    try {
      await xchonnect.resolveApproval(id, approved);
    } catch (problem) {
      // The approval timed out or the session ended while the screen was open. Either
      // way it is no longer waiting, and both of those are refusals.
      setError(String(problem));
    }
    setApprovals((current) => current.filter((item) => item.id !== id));
  }, []);

  const cancel = useCallback(async (id: string) => {
    try {
      await xchonnect.cancelApproval(id);
    } catch (problem) {
      // Already answered, timed out or ended: it is no longer waiting either way.
      setError(String(problem));
    }
    setApprovals((current) => current.filter((item) => item.id !== id));
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
