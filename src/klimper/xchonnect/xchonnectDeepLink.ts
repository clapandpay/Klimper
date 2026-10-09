// SPDX-License-Identifier: AGPL-3.0-or-later
// The operating system hands Klimper a link (clapandpay TASK-072.01).
//
// On iOS the chain is: Safari (or a dApp's "open in wallet") → the scene delegate in
// gen/apple/Sources/sage-tauri/KlimperScene.m → tao's application delegate → Tauri's
// `RunEvent::Opened` → tauri-plugin-deep-link's `deep-link://new-url` event → here →
// `offerXchonnectLink`. Two forms arrive: the pairing URI itself (`xchonnect:v1?…`, a URL
// scheme in Info.ios.plist) and clapandpay.com universal links (`/pair`, `/req`, from the
// entitlements). Parsing, redaction and routing all belong to the handoff; this module only
// delivers, and never looks at a URL beyond its path (docs/xchonnect-links.md). One path is not
// Xchonnect's: clapandpay.com/klimper/return carries the answer of "Use my Clap and Pay money"
// (TASK-129) and goes to clapandpayReturn.ts.

import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link';
import { isReturnLink } from '@/lib/clapandpayHandoff';
import { clapAndPayConnectEnabled, xchonnectEnabled } from '@/lib/features';
import { offerClapAndPayReturn } from '../clapandpayReturn';
import { offerXchonnectLink } from './xchonnectLinkHandoff';

/** The unsubscribe of a listener that never started. */
function nothingToStop() {
  return undefined;
}

/** Replaceable for tests; the real plugin needs Tauri's IPC. */
export interface DeepLinkApi {
  onOpenUrl: typeof onOpenUrl;
  getCurrent: typeof getCurrent;
}

/**
 * Deliver every link the OS hands the app, from now on and the one the app may have been
 * started with. Resolves to the unsubscribe. Outside Tauri (a browser tab, a test without
 * a mock) the plugin has no IPC and the call fails: that is a no-op here, because a web
 * build has no deep links and nothing to tell the user.
 */
export async function listenForXchonnectLinks(
  api: DeepLinkApi = { onOpenUrl, getCurrent },
): Promise<() => void> {
  if (!xchonnectEnabled && !clapAndPayConnectEnabled) return nothingToStop;

  // Listen first, then read the launch URL, so nothing arriving in between is lost. A link
  // that arrives in that window would come through both paths once; `seen` keeps it to
  // one delivery, and is forgotten once the start-up read is done so that a user who taps
  // the same link again later is served again.
  let seen: Set<string> | null = new Set();
  const deliver = (urls: string[]) => {
    for (const url of urls) {
      if (seen?.has(url)) continue;
      seen?.add(url);
      if (isReturnLink(url)) {
        if (clapAndPayConnectEnabled) offerClapAndPayReturn(url);
      } else if (xchonnectEnabled) {
        offerXchonnectLink(url);
      }
    }
  };

  let unlisten: () => void = nothingToStop;
  try {
    unlisten = await api.onOpenUrl(deliver);
    const launch = await api.getCurrent();
    if (launch) deliver(launch);
  } catch {
    // No IPC: not a Tauri window.
  }
  seen = null;
  return unlisten;
}
