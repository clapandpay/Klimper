// SPDX-License-Identifier: AGPL-3.0-or-later
// Klimper ↔ the card (tauri-plugin-klimper-card): the whole command surface the webview has.
//
// Every call to Gnosis Pay is made in Rust with the user's own session. The webview never
// holds a token, never sees a key, and receives only what the screens show. Nothing here is
// stored: the card's state is read fresh from the plugin each time a screen needs it.
import { invoke } from '@tauri-apps/api/core';

const call = <T>(command: string, args?: Record<string, unknown>) =>
  invoke<T>(`plugin:klimper-card|${command}`, args);

export interface CardStatus {
  /** This build was made with a card registration (docs/card.md). */
  configured: boolean;
  environment: 'sandbox' | 'production' | 'custom' | null;
  /** This wallet can have a card (it has recovery words). */
  available: boolean;
  unavailable: 'noWallet' | 'noRecoveryWords' | null;
  /** The card address (EIP-55): the owner of the Gnosis Pay account. */
  address: string | null;
  signedIn: boolean;
}

export type CardErrorKind =
  | 'noWallet'
  | 'noRecoveryWords'
  | 'key'
  | 'notConfigured'
  | 'invalid'
  | 'signedOut'
  | 'refusedMessage'
  | 'network'
  | 'api';

/** What a failed command rejects with. `code` is Gnosis Pay's short error code. */
export interface CardError {
  kind: CardErrorKind;
  message: string;
  code: string | null;
}

export function isCardError(value: unknown): value is CardError {
  return (
    typeof value === 'object' &&
    value !== null &&
    'kind' in value &&
    'message' in value
  );
}

export function cardErrorMessage(error: unknown): string {
  if (isCardError(error)) return error.message;
  return error instanceof Error ? error.message : String(error);
}

export const card = {
  status: () => call<CardStatus>('card_status'),
  signIn: () => call<null>('card_sign_in'),
  signOut: () => call<null>('card_sign_out'),
};

export const CARD_ROUTE = '/card';
