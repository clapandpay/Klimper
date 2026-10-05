// SPDX-License-Identifier: AGPL-3.0-or-later
// The connected dApps of the open wallet (clapandpay TASK-083, TASK-084).
//
// `listSessions()` is not only a read: it resumes the message loop of every stored
// session, which is how connections come back after a restart or after the wallet was
// unlocked. So this hook is also what brings Xchonnect back to life, and anything that
// opens an Xchonnect screen calls it.

import { xchonnectEnabled } from '@/lib/features';
import { useCallback, useEffect, useState } from 'react';
import {
  onXchonnectEvent,
  SessionSummary,
  xchonnect,
  XCHONNECT_EVENTS,
  XchonnectStatus,
} from './xchonnect';

export interface SessionList {
  readonly status: XchonnectStatus | null;
  readonly sessions: readonly SessionSummary[];
  readonly loading: boolean;
  readonly error: string | null;
  readonly refresh: () => Promise<void>;
}

export function useXchonnectSessions(): SessionList {
  const [status, setStatus] = useState<XchonnectStatus | null>(null);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    // A build that did not ask for Xchonnect never calls the plugin at all.
    if (!xchonnectEnabled) {
      setLoading(false);
      return;
    }
    try {
      setStatus(await xchonnect.status());
      setSessions(await xchonnect.listSessions());
      setError(null);
    } catch (problem) {
      // No wallet open, or a locked keychain. Both are states, not faults.
      setError(String(problem));
      setSessions([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const subscriptions = [
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

  return { status, sessions, loading, error, refresh };
}
