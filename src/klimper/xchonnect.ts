// SPDX-License-Identifier: AGPL-3.0-or-later
// Klimper ↔ Xchonnect (tauri-plugin-klimper-xchonnect): the whole command and event
// surface the webview has.
//
// Everything that decides whether a request is safe happens in Rust, in xchonnect's
// wallet-kit. The webview's job is to render what it is told and to send back exactly one
// answer per approval. In particular:
//
//   * `Approval.prompt` is what the wallet simulated locally, never what the dApp claimed.
//     Render `summary` and `plan`; there is nothing else to trust.
//   * `conditionalReceived` is not received. It only arrives if the counterparty completes.
//   * Show `domainUnicode` and, when `domainDiffers`, `dapp` (the ASCII form) next to it,
//     plus every entry of `warnings`.
//   * A session with `needsRepair` or without `sasConfirmed` cannot be used at all.
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

const call = <T>(command: string, args?: Record<string, unknown>) =>
  invoke<T>(`plugin:klimper-xchonnect|${command}`, args);

export interface XchonnectStatus {
  /** Developer-mode pairing codes are accepted. Debug builds only. */
  developerModeAllowed: boolean;
  fingerprint: number | null;
  /** CHIP-0002 chainId of the open wallet. */
  chainId: string | null;
  sessionCount: number;
  /** OS keyring service the sessions live under, shown in settings. */
  keyringService: string;
}

/** Whether wake-ups can reach this phone (clapandpay TASK-087). */
export interface PushStatus {
  /** False in a build without a push gateway: nothing could be woken. */
  enabled: boolean;
  /** What the phone's notification settings said when last asked; null before the first answer. */
  permission: PushPermission | null;
}

export type PushPermission = 'granted' | 'denied' | 'undetermined';

export interface PairingPreview {
  pairingId: string;
  /** The domain as the user reads it. */
  domainUnicode: string;
  /** The ASCII (A-label) form. */
  domainAscii: string;
  /** True when the two differ: show both. */
  domainDiffers: boolean;
  /** Homograph warnings, e.g. "NonAscii", "MixedScripts". Show all of them. */
  warnings: string[];
  /** Name the origin document claims. Untrusted text. */
  dappName: string;
  relay: string;
  expiresAt: number;
  developerMode: boolean;
  /** The default grant: the CHIP-0002 required methods. */
  grantedMethods: string[];
  /** The first exposed key (the wallet's first receive address). */
  grantedKey: string | null;
  /** How many receive keys will be exposed; the dApp can see their balances on chain. */
  grantedKeyCount: number;
  chainId: string;
}

export interface PairingSas {
  pairingId: string;
  /** Six digits. Show them large. */
  sas: string;
  /** The dApp answered: the user may now confirm or reject. */
  dappConfirmed: boolean;
  /** Unix seconds after which the attempt is abandoned. */
  deadline: number;
}

export interface AssetLimit {
  perRequest?: string | number | null;
  perDay?: string | number | null;
}

export interface SessionSummary {
  sessionId: string;
  /** Verified domain, ASCII form. */
  dapp: string;
  domainUnicode: string;
  domainDiffers: boolean;
  warnings: string[];
  dappName: string;
  chainId: string;
  createdAt: number;
  epoch: number;
  sasConfirmed: boolean;
  /** Stored state was lost or rolled back: pair again. */
  needsRepair: boolean;
  /** The website withdrew the key this connection was made with; Klimper ended it. */
  revoked: boolean;
  /** New keys were offered and are not in use yet. */
  rotating: boolean;
  developerMode: boolean;
  methods: string[];
  exposedKeys: string[];
  limits: Record<string, AssetLimit>;
  allowAggSigUnsafe: boolean;
  allowUnknownContracts: boolean;
  /** It sees every receive address, not only its own one. */
  allReceiveKeys: boolean;
  /** The message loop is running. */
  connected: boolean;
}

export interface DappPolicy {
  limits?: Record<string, AssetLimit>;
  allowAggSigUnsafe?: boolean;
  allowUnknownContracts?: boolean;
  methods?: string[];
  allReceiveKeys?: boolean;
}

/** What wallet-kit computed, as JSON. See xchonnect docs/wallet-integration.md §5. */
export type Prompt = Record<string, unknown>;

/** One request waiting for the user: one decision signs (and, if Klimper broadcasts, sends) it. */
export interface Approval {
  id: string;
  sessionId: string;
  dapp: string;
  method: string;
  /** The Xchonnect request it answers. */
  requestId: string;
  expiresAt: number;
  kind: 'request';
  prompt: Prompt;
}

export const xchonnect = {
  status: () => call<XchonnectStatus>('status'),

  /**
   * Verify a pairing URI: parse it, fetch the origin document fresh over HTTPS and check
   * the origin signature. Rejects on any failure — there is no way to continue anyway.
   * Nothing is created on the relay yet.
   */
  inspectPairingUri: (uri: string) =>
    call<PairingPreview>('inspect_pairing_uri', { uri }),

  /** The user agreed to the verified domain: reply and start waiting for the dApp. */
  approvePairing: (pairingId: string) =>
    call<PairingSas>('approve_pairing', { pairingId }),

  cancelPairing: (pairingId: string) =>
    call<null>('cancel_pairing', { pairingId }),

  /**
   * The user compared the six digits. `false` ends the session and deletes everything.
   * `allReceiveKeys`: the website sees every receive address, not only its own one.
   */
  confirmSas: (pairingId: string, matches: boolean, allReceiveKeys = false) =>
    call<SessionSummary | null>('confirm_sas', {
      pairingId,
      matches,
      allReceiveKeys,
    }),

  /** Connected dApps. Also resumes the message loop of every stored session. */
  listSessions: () => call<SessionSummary[]>('list_sessions'),

  endSession: (sessionId: string) => call<null>('end_session', { sessionId }),

  /** Offer the website new connection keys. Rejects when the connection is not live. */
  rotateSessionKeys: (sessionId: string) =>
    call<null>('rotate_session_keys', { sessionId }),

  setDappPolicy: (sessionId: string, policy: DappPolicy) =>
    call<SessionSummary>('set_dapp_policy', { sessionId, policy }),

  pendingApprovals: () => call<Approval[]>('pending_approvals'),

  /** Answer one approval. No answer within five minutes is a rejection. */
  resolveApproval: (approvalId: string, approved: boolean) =>
    call<null>('resolve_approval', { approvalId, approved }),
  /** Withdraw a request: the website is told it was cancelled (4102), not rejected. */
  cancelApproval: (approvalId: string) =>
    call<null>('cancel_approval', { approvalId }),
  pushStatus: () => call<PushStatus>('push_status'),
};

export const XCHONNECT_EVENTS = {
  /** `{ pairingId, state: 'awaitingSas' | 'failed', sas?, dappConfirmed?, error? }` */
  pairing: 'klimper-xchonnect://pairing',
  /** `{}` — the list of connected dApps changed. */
  sessions: 'klimper-xchonnect://sessions',
  /** `{ sessionId, state: 'ended' | 'rotating' | 'rotated' | 'revoked' | 'needsRepair', epoch?, dapp? }` */
  session: 'klimper-xchonnect://session',
  /** `{ sessionId, dapp, method }` — a request arrived. */
  request: 'klimper-xchonnect://request',
  /** An `Approval`: something is waiting for the user. */
  approval: 'klimper-xchonnect://approval',
  /** `{ id, approved }` — answered, withdrawn or timed out. */
  approvalResolved: 'klimper-xchonnect://approval-resolved',
  /** `{ dapp, method, transactionId, status }` — Klimper broadcast a transaction. */
  sent: 'klimper-xchonnect://sent',
  /** `{ state: PushPermission }` — the user answered the notification prompt. */
  push: 'klimper-xchonnect://push',
} as const;

export interface SentEvent {
  dapp: string;
  method: string;
  transactionId: string;
  status: string | null;
}

export interface PairingEvent {
  pairingId: string;
  state: 'awaitingSas' | 'failed';
  sas?: string | null;
  dappConfirmed?: boolean;
  error?: string;
}

export interface SessionEvent {
  sessionId: string;
  state: 'ended' | 'rotating' | 'rotated' | 'revoked' | 'needsRepair';
  epoch?: number;
  dapp?: string;
}

export const onXchonnectEvent = <T>(
  event: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> =>
  listen<T>(event, (message) => handler(message.payload));
