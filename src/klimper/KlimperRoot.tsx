// SPDX-License-Identifier: AGPL-3.0-or-later
// Everything Klimper renders next to Sage's app, in one element (docs/UPSTREAM.md,
// "Hook points"): `src/App.tsx` mounts `<KlimperRoot />` once and knows nothing else.
//
// Importing this module also applies Klimper's look (`klimperTheme` seeds the default theme
// before theme-o-rama reads it), so App.tsx needs no separate import for it.

import '@/lib/klimperTheme';
import { PasskeyGate } from './PasskeyGate';
import { XchonnectApprovalHost } from './XchonnectApprovalHost';

export { klimperRouter } from './routes';

/** The passkey gate over the app, and the approval overlay over every screen. */
export function KlimperRoot() {
  return (
    <>
      <PasskeyGate />
      <XchonnectApprovalHost />
    </>
  );
}
