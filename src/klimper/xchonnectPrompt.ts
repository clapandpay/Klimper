// SPDX-License-Identifier: AGPL-3.0-or-later
// The trust boundary of the approval screen (clapandpay TASK-069, TASK-080).
//
// `Approval.prompt` is `serde_json::to_value` of the `Prompt` that xchonnect's wallet-kit
// built from running every spend locally. It is the only evidence about a request that
// exists: the dApp's own description of what it is asking for is not evidence and is not
// rendered anywhere.
//
// Rather than trust that by convention, this module is the only thing that reads the
// prompt, and it reads it **by explicit key and validated type**. A key that is not in
// `docs/xchonnect-screens.md`'s rendering table is dropped here and cannot reach a
// component, so a prompt that grew a `label`, a `description` or a second `amount` — by a
// wallet-kit change, by a crafted request, or by a bug — renders nothing new.
//
// Every string the view receives is either validated hex or the one piece of free text
// that *is* the subject of the request: the `signMessage` body, which is sanitised here
// and shown only inside its own quoted block.

import BigNumber from 'bignumber.js';

/** XCH is quoted in mojos: 10^12 per XCH. */
export const XCH_PRECISION = 12;
/** CATs carry three decimals by convention. */
export const CAT_PRECISION = 3;

/** How much of a signMessage body is shown before it is truncated. */
const MESSAGE_LIMIT = 2000;

export interface AssetView {
  /** `XCH`, or `CAT <first 8 hex>…`. Never a name the dApp supplied. */
  readonly label: string;
  readonly isXch: boolean;
  /** Full TAIL hash, for a CAT. */
  readonly assetId: string | null;
  readonly precision: number;
  /** `received − sent`: what happens for certain. Negative is a loss. */
  readonly net: BigNumber;
  /**
   * Arrives only from spends the user neither owns nor signs. Kept apart from `net` on
   * purpose and never added to it: it is income only if somebody else acts.
   */
  readonly conditionalReceived: BigNumber;
  /** Gross figures, shown only inside the disclosure — alone they mislead. */
  readonly sent: BigNumber;
  readonly received: BigNumber;
}

export interface SignatureView {
  readonly publicKey: string;
  /** `AGG_SIG_UNSAFE`: valid in any spend, not just this one. */
  readonly isUnsafe: boolean;
  readonly opcode: number | null;
}

export interface TimeLockView {
  readonly notBeforeSeconds: number | null;
  readonly notBeforeHeight: number | null;
  readonly expiresSeconds: number | null;
  readonly expiresHeight: number | null;
  readonly hasRelative: boolean;
}

export interface BoundPaymentView {
  readonly label: string;
  readonly precision: number;
  readonly amount: BigNumber;
}

export interface BindingView {
  readonly allBound: boolean;
  readonly payments: readonly BoundPaymentView[];
}

/** One coin the user's signature creates: where the money goes. */
export interface OutputView {
  readonly label: string;
  readonly isXch: boolean;
  readonly precision: number;
  readonly amount: BigNumber;
  /** The recipient's p2 puzzle hash (what an address encodes), or null when unknown. */
  readonly recipient: string | null;
  /** To one of the user's own addresses: change, or a send to oneself. */
  readonly toUser: boolean;
}

/** One claim of the website's declared intent, verified by the wallet. */
export type VerifiedFactView =
  | {
      readonly type: 'recipient';
      readonly label: string;
      readonly precision: number;
      readonly amount: BigNumber;
      readonly recipient: string;
    }
  | { readonly type: 'fee'; readonly amount: BigNumber }
  | {
      readonly type: 'net';
      readonly label: string;
      readonly precision: number;
      readonly amount: BigNumber;
    };

/** The website's declared intent, as the wallet verified it. */
export interface IntentView {
  /** The website's label (`[a-z0-9._-]` only), shown as described by the website. */
  readonly kind: string | null;
  readonly verified: readonly VerifiedFactView[];
  /** The website declared every payment to others; no undeclared payment exists. */
  readonly recipientsComplete: boolean;
}

export interface SpendPromptView {
  readonly kind: 'spend';
  /** Klimper signs **and sends** this (`submit_coin_spends`); otherwise it only signs. */
  readonly broadcasts: boolean;
  /** Klimper built the spend itself from the request (`chia_send`). */
  readonly walletBuilt: boolean;
  /** The wallet database and the independent simulation were compared and agree. */
  readonly crossChecked: boolean;
  /** The website's declared intent, every claim verified; `null` when none was given. */
  readonly intent: IntentView | null;
  /** The network's own name for its coin: `XCH`, or `TXCH` on testnet. */
  readonly xchLabel: string;
  readonly assets: readonly AssetView[];
  /** Every coin the user's own spends create, in order. Empty from an older wallet-kit. */
  readonly outputs: readonly OutputView[];
  /** Fee implied by the request. `null` means outputs exceed inputs (a partial spend). */
  readonly impliedFee: BigNumber | null;
  readonly reserveFee: BigNumber;
  /** Puzzle hashes the wallet does not recognise. Shown in full, and they gate a second confirmation. */
  readonly unknownPuzzles: readonly string[];
  readonly timeLocks: TimeLockView;
  readonly signatures: readonly SignatureView[];
  readonly foreignSignatures: number;
  readonly hasUnsafe: boolean;
  /** `partialSign`: one side of a multi-party spend. */
  readonly partial: boolean;
  readonly binding: BindingView | null;
}

export interface MessagePromptView {
  readonly kind: 'message';
  /** Printable text, control and direction characters neutralised. `null` when it is not text. */
  readonly messageText: string | null;
  readonly messageHex: string;
  readonly truncated: boolean;
  /** Characters in the original, so a truncated body still states its real size. */
  readonly length: number;
  readonly publicKey: string;
}

/** A prompt shape this build does not know. The screen refuses rather than guesses. */
export interface UnreadablePromptView {
  readonly kind: 'unreadable';
}

export type PromptView =
  | SpendPromptView
  | MessagePromptView
  | UnreadablePromptView;

// ---------------------------------------------------------------------------------------
// Readers. Each one answers "is this the type the rendering table says it is?" and
// returns a safe default otherwise. Nothing here throws: the prompt is parsed JSON from
// another process, and a surprise in it must degrade the screen, not break it.
// ---------------------------------------------------------------------------------------

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const arrayOf = (value: unknown): unknown[] =>
  Array.isArray(value) ? value : [];

const boolOf = (value: unknown): boolean => value === true;

/** An integer that fits a JS number, or null. */
function intOf(value: unknown): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  return Math.trunc(value);
}

/**
 * An amount in the smallest unit.
 *
 * Accepts a JSON number (what the plugin sends today) and a decimal string, so the
 * plugin can move to strings — which it should, because an amount above 2^53 mojos
 * (about 9007 XCH) has already lost precision by the time `JSON.parse` has run — without
 * the screens changing.
 */
function amountOf(value: unknown): BigNumber {
  // Absent is zero (an optional field the simulation left out).
  if (value === undefined || value === null) return BigNumber(0);
  // A number is exact only up to 2^53: past that, or with a fraction, the figure on screen
  // would differ from the one signed.
  if (typeof value === 'number' && Number.isSafeInteger(value)) {
    return BigNumber(value);
  }
  if (typeof value === 'string' && /^-?\d+$/.test(value)) {
    return BigNumber(value);
  }
  // Anything else cannot be shown truthfully: the whole prompt becomes unreadable.
  throw new UnreadableAmount();
}

/** A present amount that is not an exact integer; `readPrompt` turns it into `unreadable`. */
/**
 * How a CAT is named wherever Klimper cannot vouch for a name: by both ends of its asset id,
 * since a CAT whose first 32 bits match a real one is cheap to make.
 */
export function catLabel(assetId: string): string {
  return `CAT ${assetId.slice(0, 8)}…${assetId.slice(-6)}`;
}

/** Something in the prompt cannot be shown exactly (an amount, an asset): the whole prompt is unreadable. */
class UnreadableAmount extends Error {}

/** Lower-case hex of any length, with or without `0x`. Anything else is dropped. */
function hexOf(value: unknown, maxBytes: number): string | null {
  if (typeof value !== 'string') return null;
  const body =
    value.startsWith('0x') || value.startsWith('0X') ? value.slice(2) : value;
  if (body.length === 0 || body.length > maxBytes * 2) return null;
  if (body.length % 2 !== 0) return null;
  return /^[0-9a-fA-F]+$/.test(body) ? body.toLowerCase() : null;
}

/**
 * The asset, from wallet-kit's `{ "type": "xch" }` / `{ "type": "cat", "asset_id": … }`.
 *
 * The label is derived here, never read: a CAT's human name would be a string from
 * outside, and this screen shows no such string.
 */
function assetOf(
  value: unknown,
  xchLabel: string,
): {
  label: string;
  isXch: boolean;
  assetId: string | null;
} {
  if (!isRecord(value)) return { label: xchLabel, isXch: true, assetId: null };
  if (value.type === 'cat') {
    const assetId = hexOf(value.asset_id, 32);
    return assetId === null
      ? { label: 'Unknown CAT', isXch: false, assetId: null }
      : { label: catLabel(assetId), isXch: false, assetId };
  }
  // A kind of asset this build does not know is not shown as XCH: nobody can consent to it.
  if (value.type !== 'xch') throw new UnreadableAmount();
  return { label: xchLabel, isXch: true, assetId: null };
}

function assetViewOf(entry: unknown, xchLabel: string): AssetView | null {
  if (!isRecord(entry)) return null;
  const asset = assetOf(entry.asset, xchLabel);
  return {
    label: asset.label,
    isXch: asset.isXch,
    assetId: asset.assetId,
    precision: asset.isXch ? XCH_PRECISION : CAT_PRECISION,
    net: amountOf(entry.net),
    conditionalReceived: amountOf(entry.conditional_received),
    sent: amountOf(entry.sent),
    received: amountOf(entry.received),
  };
}

function outputViewOf(entry: unknown, xchLabel: string): OutputView | null {
  if (!isRecord(entry)) return null;
  const asset = assetOf(entry.asset, xchLabel);
  return {
    label: asset.label,
    isXch: asset.isXch,
    precision: asset.isXch ? XCH_PRECISION : CAT_PRECISION,
    amount: amountOf(entry.amount),
    recipient: hexOf(entry.recipient, 32),
    toUser: boolOf(entry.to_user),
  };
}

/** Only the characters the wallet itself accepts in a `kind`. Anything else is dropped. */
function kindOf(value: unknown): string | null {
  return typeof value === 'string' && /^[a-z0-9._-]{1,48}$/.test(value)
    ? value
    : null;
}

function intentOf(value: unknown, xchLabel: string): IntentView | null {
  if (!isRecord(value)) return null;
  const verified: VerifiedFactView[] = [];
  for (const entry of arrayOf(value.verified)) {
    if (!isRecord(entry)) continue;
    if (entry.type === 'fee') {
      verified.push({ type: 'fee', amount: amountOf(entry.amount) });
      continue;
    }
    const asset = assetOf(entry.asset, xchLabel);
    const precision = asset.isXch ? XCH_PRECISION : CAT_PRECISION;
    if (entry.type === 'recipient') {
      const recipient = hexOf(entry.puzzle_hash, 32);
      if (recipient === null) continue;
      verified.push({
        type: 'recipient',
        label: asset.label,
        precision,
        amount: amountOf(entry.amount),
        recipient,
      });
    } else if (entry.type === 'net') {
      verified.push({
        type: 'net',
        label: asset.label,
        precision,
        amount: amountOf(entry.amount),
      });
    }
  }
  return {
    kind: kindOf(value.kind),
    verified,
    recipientsComplete: boolOf(value.recipients_complete),
  };
}

function timeLocksOf(value: unknown): TimeLockView {
  if (!isRecord(value)) {
    return {
      notBeforeSeconds: null,
      notBeforeHeight: null,
      expiresSeconds: null,
      expiresHeight: null,
      hasRelative: false,
    };
  }
  return {
    notBeforeSeconds: intOf(value.not_before_seconds),
    notBeforeHeight: intOf(value.not_before_height),
    expiresSeconds: intOf(value.expires_seconds),
    expiresHeight: intOf(value.expires_height),
    hasRelative: boolOf(value.has_relative),
  };
}

function bindingOf(value: unknown, xchLabel: string): BindingView | null {
  if (!isRecord(value)) return null;
  const payments: BoundPaymentView[] = [];
  for (const entry of arrayOf(value.bound_payments)) {
    if (!isRecord(entry)) continue;
    const asset = assetOf(entry.asset, xchLabel);
    payments.push({
      label: asset.label,
      precision: asset.isXch ? XCH_PRECISION : CAT_PRECISION,
      amount: amountOf(entry.amount),
    });
  }
  return { allBound: boolOf(value.all_bound), payments };
}

/**
 * Neutralise a message body before it is shown.
 *
 * It is the one piece of text on the screen that the requester wrote, so it gets the
 * treatment text from outside gets: C0/C1 controls, the bidirectional overrides that make
 * a string render in an order it is not written in, and the zero-width characters that
 * hide content, are all replaced with a visible replacement character. The result stays
 * inside its own quoted block, labelled as what the website asked for.
 */
export function sanitiseMessageText(text: string): {
  text: string;
  truncated: boolean;
  length: number;
} {
  const cleaned = Array.from(text)
    .map((character) => {
      const code = character.codePointAt(0) ?? 0;
      if (character === '\n' || character === '\t') return character;
      if (code < 0x20 || (code >= 0x7f && code <= 0x9f)) return '�';
      // Every format character (bidi controls, zero-width and word joiners, BOM, soft
      // hyphen, tag characters…), line and paragraph separators, and the Hangul fillers
      // that render as nothing.
      if (
        /[\p{Cf}\p{Zl}\p{Zp}]/u.test(character) ||
        code === 0x115f ||
        code === 0x1160 ||
        code === 0x3164 ||
        code === 0xffa0
      ) {
        return '�';
      }
      return character;
    })
    .join('');
  const characters = Array.from(cleaned);
  return {
    text: characters.slice(0, MESSAGE_LIMIT).join(''),
    truncated: characters.length > MESSAGE_LIMIT,
    length: Array.from(text).length,
  };
}

/** How much of a name the origin document claims is shown. */
const CLAIMED_TEXT_LIMIT = 64;

/**
 * A string the dApp wrote, prepared for the one place it is allowed to appear.
 *
 * The name in an origin document is a claim: only the domain was verified. It is shown
 * once, labelled as a claim, and never on the approval screen — but even there it must not
 * be able to impersonate the interface around it, so it is sanitised like any other text
 * from outside and cut to one line's worth of characters.
 */
export function claimedText(value: unknown): string {
  if (typeof value !== 'string') return '';
  const { text } = sanitiseMessageText(value.trim());
  const characters = Array.from(text);
  return characters.length > CLAIMED_TEXT_LIMIT
    ? `${characters.slice(0, CLAIMED_TEXT_LIMIT).join('')}…`
    : characters.join('');
}

/**
 * Read a wallet-kit prompt into exactly what the screen may show.
 *
 * An unknown `type`, or a `sign_message` without a readable message, is `unreadable`:
 * the screen then offers only Reject, because a request the wallet cannot describe is
 * not a request a user can consent to.
 */
export function readPrompt(
  prompt: unknown,
  options: { readonly xchLabel?: string } = {},
): PromptView {
  try {
    return readPromptStrict(prompt, options);
  } catch (problem) {
    // An amount that cannot be shown exactly: nothing a user could consent to.
    if (problem instanceof UnreadableAmount) return { kind: 'unreadable' };
    throw problem;
  }
}

function readPromptStrict(
  prompt: unknown,
  { xchLabel = 'XCH' }: { readonly xchLabel?: string },
): PromptView {
  if (!isRecord(prompt)) return { kind: 'unreadable' };

  if (prompt.type === 'sign_message') {
    const messageHex = hexOf(prompt.message_hex, 1024);
    if (messageHex === null) return { kind: 'unreadable' };
    const raw =
      typeof prompt.message_text === 'string' ? prompt.message_text : null;
    const cleaned = raw === null ? null : sanitiseMessageText(raw);
    return {
      kind: 'message',
      messageText: cleaned?.text ?? null,
      messageHex,
      truncated: cleaned?.truncated ?? false,
      length: cleaned?.length ?? messageHex.length / 2,
      publicKey: hexOf(prompt.public_key, 48) ?? '',
    };
  }

  // `submit_coin_spends`: the same simulation and plan, and Klimper also broadcasts.
  const broadcasts = prompt.type === 'submit_coin_spends';
  if (prompt.type !== 'sign_coin_spends' && !broadcasts) {
    return { kind: 'unreadable' };
  }

  const summary = isRecord(prompt.summary) ? prompt.summary : {};
  const plan = isRecord(prompt.plan) ? prompt.plan : {};

  const assets: AssetView[] = [];
  for (const entry of arrayOf(summary.assets)) {
    const view = assetViewOf(entry, xchLabel);
    if (view !== null) assets.push(view);
  }

  const outputs: OutputView[] = [];
  for (const entry of arrayOf(summary.outputs)) {
    const view = outputViewOf(entry, xchLabel);
    if (view !== null) outputs.push(view);
  }

  const signatures: SignatureView[] = [];
  for (const entry of arrayOf(plan.ours)) {
    if (!isRecord(entry)) continue;
    signatures.push({
      publicKey: hexOf(entry.public_key, 48) ?? '',
      isUnsafe: boolOf(entry.is_unsafe),
      opcode: intOf(entry.opcode),
    });
  }

  const unknownPuzzles: string[] = [];
  for (const entry of arrayOf(summary.unknown_puzzles)) {
    const hash = hexOf(entry, 32);
    if (hash !== null) unknownPuzzles.push(hash);
  }

  const impliedFee =
    summary.implied_fee === null || summary.implied_fee === undefined
      ? null
      : amountOf(summary.implied_fee);

  return {
    kind: 'spend',
    broadcasts,
    walletBuilt: broadcasts && boolOf(prompt.wallet_built),
    crossChecked: boolOf(prompt.cross_checked),
    intent: intentOf(prompt.intent, xchLabel),
    xchLabel,
    assets,
    outputs,
    impliedFee,
    reserveFee: amountOf(summary.reserve_fee),
    unknownPuzzles,
    timeLocks: timeLocksOf(summary.time_locks),
    signatures,
    foreignSignatures: arrayOf(plan.foreign).length,
    // `has_unsafe` is the summary flag; a single `is_unsafe` requirement is enough on its
    // own, so the warning does not depend on both being written.
    hasUnsafe: boolOf(plan.has_unsafe) || signatures.some((s) => s.isUnsafe),
    // A submitted bundle another party's signature completes is the binding case too.
    partial: boolOf(prompt.partial) || (broadcasts && isRecord(prompt.binding)),
    binding: bindingOf(prompt.binding, xchLabel),
  };
}

/** Format an amount in the smallest unit for display. Exact, never rounded up. */
export function formatAmount(amount: BigNumber, precision: number): string {
  return amount
    .dividedBy(BigNumber(10).pow(precision))
    .toFormat({ decimalSeparator: '.', groupSeparator: '' });
}

/** The absolute value, for a line whose sign is carried by the words "pay" or "receive". */
export function formatMagnitude(amount: BigNumber, precision: number): string {
  return formatAmount(amount.abs(), precision);
}

/**
 * Does this prompt need the second confirmation?
 *
 * An unrecognised puzzle is the case where nobody — not the wallet, not the user — can say
 * what the contract does with the coins, so approving it is a separate decision from
 * approving the request (spec 11.1 item 3).
 */
export function needsSecondConfirmation(view: PromptView): boolean {
  return view.kind === 'spend' && view.unknownPuzzles.length > 0;
}
