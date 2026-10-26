// SPDX-License-Identifier: AGPL-3.0-or-later
// Klimper ↔ the card (tauri-plugin-klimper-card): the whole command surface the webview has.
//
// Every call to the card issuer is made in Rust with the user's own session. The webview never
// holds a token, never sees a key, and receives only what the screens show. Nothing here is
// stored: the card's state is read fresh from the plugin each time a screen needs it.
import { invoke } from '@tauri-apps/api/core';

const call = <T>(command: string, args?: Record<string, unknown>) =>
  invoke<T>(`plugin:klimper-card|${command}`, args);

/** The build's card issuer (KLIMPER_CARD_ISSUER, docs/card.md). Read it with `useCardIssuer`. */
export interface CardIssuer {
  /** `gnosis-pay` */
  id: string;
  /** The name the screens show, e.g. `Gnosis Pay`. */
  name: string;
  /** The card scheme, e.g. `Visa`. */
  scheme: string;
  /** The chain the card account is on, e.g. `Gnosis Chain`. */
  network: string;
  /** What a top-up sends to the card account, e.g. `EURe`. */
  topUpAsset: string;
  environment: 'sandbox' | 'production' | 'custom';
  capabilities: {
    /** Number, expiry and CVV on the device. */
    cardDetails: boolean;
    freeze: boolean;
  };
}

export interface CardStatus {
  /** This build was made with a card issuer and its settings (docs/card.md). */
  configured: boolean;
  /** Null when not configured. */
  issuer: CardIssuer | null;
  /** This wallet can have a card (it has recovery words). */
  available: boolean;
  unavailable: 'noWallet' | 'noRecoveryWords' | null;
  /** The card address (EIP-55): the owner of the account at the card issuer. */
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

/** What a failed command rejects with. `code` is the issuer's short error code. */
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

/** The one thing to do next in the issuer's onboarding. */
export type Step =
  | { step: 'register' }
  | { step: 'acceptTerms'; terms: Term[] }
  | { step: 'waitingKyc' }
  /** `url` is an https link to the issuer's identity provider, or null when there is none. */
  | { step: 'completeKyc'; url: string | null; resubmission: boolean }
  | { step: 'kycReview' }
  | { step: 'sourceOfFunds'; questions: Question[] }
  | { step: 'createAccount' }
  | { step: 'waitingAccount' }
  | { step: 'verifyPhone' }
  | { step: 'completed' }
  | { step: 'rejected' }
  | { step: 'unknown'; status: string };

/** Steps where the issuer is working and the screen polls. */
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
  /** The on-chain address that holds the card's money, for top-ups (Gnosis Pay: the Safe). */
  accountAddress: string | null;
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

/** The card's details, for the screen only: never stored, never logged. */
export interface CardDetails {
  number: string;
  cvv: string;
  /** MM/YY */
  expires: string;
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
  details: (cardId: string) => call<CardDetails>('card_details', { cardId }),
};

export const CARD_ROUTE = '/card';
