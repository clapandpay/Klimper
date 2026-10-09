// SPDX-License-Identifier: AGPL-3.0-or-later
// Whether signing requests can reach the user while Klimper is closed (clapandpay TASK-087).
//
// Wake-ups need the notification permission. The first pairing asks for it; when the user
// says no, connections still work, but only while Klimper is open, and the screen has to say
// so instead of leaving a person waiting for a notification that cannot come.

import { xchonnectEnabled } from '@/lib/features';
import { useEffect, useState } from 'react';
import {
  onXchonnectEvent,
  PushPermission,
  xchonnect,
  XCHONNECT_EVENTS,
} from './xchonnect';

export interface PushState {
  /** Wake-ups are built into this app and the user refused notifications. */
  readonly denied: boolean;
}

export function useXchonnectPush(): PushState {
  const [enabled, setEnabled] = useState(false);
  const [permission, setPermission] = useState<PushPermission | null>(null);

  useEffect(() => {
    if (!xchonnectEnabled) return;
    let current = true;
    xchonnect
      .pushStatus()
      .then((status) => {
        if (!current) return;
        setEnabled(status.enabled);
        setPermission(status.permission);
      })
      // An older or desktop build without the command has no push to explain.
      .catch(() => undefined);
    const subscription = onXchonnectEvent<{ state: PushPermission }>(
      XCHONNECT_EVENTS.push,
      (event) => setPermission(event.state),
    );
    return () => {
      current = false;
      void subscription.then((stop) => stop());
    };
  }, []);

  return { denied: enabled && permission === 'denied' };
}
