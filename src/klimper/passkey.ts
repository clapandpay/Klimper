// SPDX-License-Identifier: AGPL-3.0-or-later
// Klimper passkey login (tauri-plugin-klimper-passkey). The passkey's PRF output stays in
// Rust; the webview only asks for status, enrolment, unlock, lock and removal.
import { invoke } from '@tauri-apps/api/core';

export interface PasskeyStatus {
  supported: boolean;
  enrolled: boolean;
  locked: boolean;
}

const call = <T>(command: string) =>
  invoke<T>(`plugin:klimper-passkey|${command}`);

export const passkey = {
  status: () => call<PasskeyStatus>('status'),
  enroll: () => call<null>('enroll'),
  unlock: () => call<null>('unlock'),
  lock: () => call<null>('lock'),
  remove: () => call<null>('remove'),
  /** Passkey lost: throw away the keys on this phone (restore from recovery words). */
  erase: () => call<null>('erase'),
};

// Lets the settings screen tell the gate that enrolment or removal changed the status.
export const PASSKEY_CHANGED = 'klimper-passkey-changed';
export const notifyPasskeyChanged = () =>
  window.dispatchEvent(new Event(PASSKEY_CHANGED));
