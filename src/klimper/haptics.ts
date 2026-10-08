// SPDX-License-Identifier: AGPL-3.0-or-later
// The system haptics of a control (the slide to approve): a finger takes hold, passes a mark,
// finishes. Phone only, fire and forget; the phone's own haptics setting decides whether it
// is felt. The native side is the Klimper jingle plugin (`feel`).

import { invoke } from '@tauri-apps/api/core';
import { platform } from '@tauri-apps/plugin-os';

export type Feel = 'grab' | 'tick' | 'done';

let phone: boolean | null = null;

function onPhone(): boolean {
  if (phone === null) {
    try {
      const os = platform();
      phone = os === 'ios' || os === 'android';
    } catch {
      phone = false;
    }
  }
  return phone;
}

export function feel(kind: Feel): void {
  if (!onPhone()) return;
  void invoke('plugin:klimper-jingle|feel', { kind }).catch(() => undefined);
}
