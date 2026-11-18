// SPDX-License-Identifier: AGPL-3.0-or-later
// The version Settings, About shows: Klimper's and the Sage release under it (version.ts),
// where Sage shows the Tauri app version, which is Sage's.

import { useEffect, useState } from 'react';
import { getVersion } from './version';

/** Klimper's version; `fallback` until it is read. */
export function AppVersion({ fallback }: { readonly fallback: string | null }) {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    void getVersion().then((v) => live && setVersion(v));
    return () => {
      live = false;
    };
  }, []);
  return <>{version ?? fallback}</>;
}
