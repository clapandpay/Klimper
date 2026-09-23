// SPDX-License-Identifier: AGPL-3.0-or-later
import {
  HandleError,
  ResolvedHandle,
  isHandleInput,
  resolveHandle,
} from '@/lib/handles';
import { useEffect, useState } from 'react';

export type HandleLookup =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'resolved'; resolved: ResolvedHandle }
  | { status: 'error'; error: HandleError };

const DEBOUNCE_MS = 350;

// Live lookup while the user types "@anna", so the real destination is on screen before the
// send button is pressed.
export function useHandle(input: string, prefix: string | null): HandleLookup {
  const [lookup, setLookup] = useState<HandleLookup>({ status: 'idle' });

  useEffect(() => {
    if (!prefix || !isHandleInput(input) || input.trim().length < 4) {
      setLookup({ status: 'idle' });
      return;
    }

    let cancelled = false;
    setLookup({ status: 'loading' });

    const timer = setTimeout(() => {
      resolveHandle(input, prefix)
        .then((resolved) => {
          if (!cancelled) setLookup({ status: 'resolved', resolved });
        })
        .catch((error) => {
          if (cancelled) return;
          setLookup({
            status: 'error',
            error:
              error instanceof HandleError
                ? error
                : new HandleError('unavailable'),
          });
        });
    }, DEBOUNCE_MS);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [input, prefix]);

  return lookup;
}
