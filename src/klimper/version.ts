// SPDX-License-Identifier: AGPL-3.0-or-later
// The version Settings shows. Klimper numbers its own releases (scripts/set-version.sh writes
// version.json), while the Tauri app version is the Sage release the engine is built from:
// Sage's app system checks an app's minimum Sage version against it (docs/UPSTREAM.md).

import { getVersion as getSageVersion } from '@tauri-apps/api/app';
import klimper from './version.json';

export const KLIMPER_VERSION: string = klimper.version;

/** Klimper's version and the Sage release under it, e.g. `0.1.0 (Sage 0.13.1)`. */
export async function getVersion(): Promise<string> {
  try {
    return `${KLIMPER_VERSION} (Sage ${await getSageVersion()})`;
  } catch {
    return KLIMPER_VERSION;
  }
}
