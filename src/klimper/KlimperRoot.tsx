// SPDX-License-Identifier: AGPL-3.0-or-later
// Everything Klimper renders next to Sage's app, in one element (docs/UPSTREAM.md,
// "Hook points"): `src/App.tsx` mounts `<KlimperRoot />` once and knows nothing else.
//
// Importing this module also applies Klimper's look (`klimperTheme` seeds the default theme
// before theme-o-rama reads it), so App.tsx needs no separate import for it.

import '@/lib/klimperTheme';
import { t } from '@lingui/core/macro';
import { lazy, Suspense, useEffect, useState } from 'react';
import { klimperSoundEnabled } from '@/lib/features';
import { KlimperErrorBoundary } from './KlimperErrorBoundary';
import { PasskeyGate } from './PasskeyGate';
import { XchonnectApprovalHost } from './XchonnectApprovalHost';
import { listenForXchonnectLinks } from './xchonnectDeepLink';
import { goToXchonnectRoute, XCHONNECT_ROUTES } from './xchonnectLinkHandoff';

export { klimperRouter } from './routes';

// The Klimper sound (decision-009): its chunk is loaded only when the flag is on, so a build
// without it carries nothing of it, and a build with it pays for the chunk once.
const KlimperSound = klimperSoundEnabled
  ? lazy(() => import('./sound/KlimperSound'))
  : null;

function useHash(): string {
  const [hash, setHash] = useState(() => window.location.hash);
  useEffect(() => {
    const update = () => setHash(window.location.hash);
    window.addEventListener('hashchange', update);
    return () => window.removeEventListener('hashchange', update);
  }, []);
  return hash;
}

/**
 * The overlay failed to render: the request is still waiting and the requests page shows
 * it, so point there and leave the screen underneath usable (TASK-105).
 */
function OverlayFailed() {
  return (
    <button
      type='button'
      className='xc-waiting-pill'
      onClick={() => goToXchonnectRoute(XCHONNECT_ROUTES.request)}
      data-testid='approval-overlay-failed'
    >
      {t`A request could not be shown. Open requests`}
    </button>
  );
}

/** Links the OS hands the app (`xchonnect:` scheme, clapandpay.com universal links). */
function useDeepLinks() {
  useEffect(() => {
    let stop: (() => void) | null = null;
    let unmounted = false;
    void listenForXchonnectLinks().then((unlisten) => {
      if (unmounted) unlisten();
      else stop = unlisten;
    });
    return () => {
      unmounted = true;
      stop?.();
    };
  }, []);
}

/**
 * The passkey gate over the app, and the approval overlay over every screen. The overlay
 * has a boundary of its own (`KlimperErrorBoundary`), cleared on the next route; the gate
 * has none, so that a failed gate never leaves the wallet open.
 */
export function KlimperRoot() {
  const hash = useHash();
  useDeepLinks();
  return (
    <>
      <PasskeyGate />
      {KlimperSound && (
        <Suspense fallback={null}>
          <KlimperSound />
        </Suspense>
      )}
      <KlimperErrorBoundary
        resetKey={hash}
        renderFallback={() => <OverlayFailed />}
      >
        <XchonnectApprovalHost />
      </KlimperErrorBoundary>
    </>
  );
}
