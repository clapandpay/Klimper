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

export interface Term {
  type: string;
  version: string;
  name: string;
  url: string | null;
  accepted: boolean;
}

export interface Question {
  id: string;
  question: string;
  answers: string[];
}

/** The one thing to do next in Gnosis Pay's onboarding. */
export type Step =
  | { step: 'register' }
  | { step: 'acceptTerms'; terms: Term[] }
  | { step: 'waitingKyc' }
  /** `url` is an https link to Sumsub, or null when Gnosis Pay sent none (or another). */
  | { step: 'completeKyc'; url: string | null; resubmission: boolean }
  | { step: 'kycReview' }
  | { step: 'sourceOfFunds'; questions: Question[] }
  | { step: 'createAccount' }
  | { step: 'waitingAccount' }
  | { step: 'verifyPhone' }
  | { step: 'completed' }
  | { step: 'rejected' }
  | { step: 'unknown'; status: string };

/** Steps where Gnosis Pay is working and the screen polls. */
export const WAITING_STEPS: ReadonlySet<Step['step']> = new Set([
  'waitingKyc',
  'completeKyc',
  'kycReview',
  'waitingAccount',
  'unknown',
]);

export interface CardSummary {
  id: string;
  status:
    | 'provisioning'
    | 'pending_activation'
    | 'active'
    | 'blocked'
    | 'canceled'
    | string;
  kind: 'virtual' | 'plastic' | string;
  last4: string | null;
  brand: string | null;
  expires: string | null;
}

export interface Account {
  safeAddress: string | null;
  ownerAddress: string | null;
  status: string;
  currency: string;
  /** Decimal strings in `currency`. */
  spendable: string;
  pending: string;
  cards: CardSummary[];
}

export interface Payment {
  id: string;
  kind: string;
  createdAt: string | null;
  status: string;
  merchant: string | null;
  amount: string | null;
  currency: string | null;
}

export interface Statement {
  items: Payment[];
  nextCursor: string | null;
}

export const card = {
  status: () => call<CardStatus>('card_status'),
  signIn: () => call<null>('card_sign_in'),
  signOut: () => call<null>('card_sign_out'),
  onboarding: () => call<Step>('card_onboarding'),
  /** Resolves `false` when this card programme needs no e-mail code. */
  sendEmailCode: (email: string) =>
    call<boolean>('card_send_email_code', { email }),
  register: (email: string, code: string | null) =>
    call<null>('card_register', { email, code }),
  acceptTerms: (terms: Term[]) => call<null>('card_accept_terms', { terms }),
  answerSourceOfFunds: (answers: { question: string; answer: string }[]) =>
    call<null>('card_answer_source_of_funds', { answers }),
  createAccount: () => call<null>('card_create_account'),
  sendPhoneCode: (phone: string) =>
    call<null>('card_send_phone_code', { phone }),
  verifyPhone: (code: string) => call<null>('card_verify_phone', { code }),
  orderCard: (name: string) => call<null>('card_order_card', { name }),
  account: () => call<Account>('card_account'),
  transactions: (cursor: string | null = null) =>
    call<Statement>('card_transactions', { cursor }),
  setFrozen: (cardId: string, frozen: boolean) =>
    call<null>('card_set_frozen', { cardId, frozen }),
};

export const CARD_ROUTE = '/card';
