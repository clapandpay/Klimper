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
/** An option singleton is counted whole: one mojo is one option (TASK-140). */
export const OPTION_PRECISION = 0;

/** How much of a signMessage body is shown before it is truncated. */
const MESSAGE_LIMIT = 2000;

export interface AssetView {
  /** `XCH`, `CAT <first 8 hex>…`, or `Option <first 8 hex>…`. Never a name the dApp supplied. */
  readonly label: string;
  readonly isXch: boolean;
  /** An option contract (TASK-140), identified by its launcher id in `assetId`. */
  readonly isOption: boolean;
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
  readonly isOption: boolean;
  readonly precision: number;
  readonly amount: BigNumber;
  /** The recipient's p2 puzzle hash (what an address encodes), or null when unknown. */
  readonly recipient: string | null;
  /** To one of the user's own addresses: change, or a send to oneself. */
  readonly toUser: boolean;
  /**
   * The output belongs to an option the request mints (TASK-140): the collateral it locks or
   * its launcher. Never a payment to anyone; `recipient` is null then.
   */
  readonly option: OptionOutputView | null;
  /** To the offer settlement puzzle: whoever takes the offer collects it. */
  readonly toSettlement: boolean;
  /**
   * Created by a spend Klimper signs but cannot read (an unknown contract around the user's
   * key: an NFT, a vault). Never counted as received or shown as change (spec 11.1 item 3).
   */
  readonly unaccounted: boolean;
}

export interface OptionOutputView {
  readonly launcherId: string;
  readonly role: 'collateral' | 'launcher';
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
  /**
   * `kind` is one of wallet-kit's verified kinds (`option.mint`, `loan.request`, …) and the
   * simulation shows it holds: shown as Klimper's finding, not the website's words.
   */
  readonly kindVerified: boolean;
  readonly verified: readonly VerifiedFactView[];
  /** The website declared every payment to others; no undeclared payment exists. */
  readonly recipientsComplete: boolean;
}

/** One asset of a trade and how much of it, in the smallest unit. */
export interface TradeLegView {
  /** `XCH`, or `CAT <first 8 hex>…<last 6>`: never a name from outside. */
  readonly label: string;
  readonly isXch: boolean;
  readonly isOption: boolean;
  readonly assetId: string | null;
  readonly precision: number;
  readonly amount: BigNumber;
}

/** An asset named in a price. */
export interface PricedAssetView {
  readonly label: string;
  readonly isXch: boolean;
  readonly isOption: boolean;
  readonly assetId: string | null;
}

/** The effective price of a one-for-one trade, as the wallet computed it, both ways round. */
export interface PriceView {
  readonly base: PricedAssetView;
  readonly quote: PricedAssetView;
  /** How much `quote` one whole `base` costs: a plain decimal. */
  readonly quotePerBase: string;
  readonly basePerQuote: string;
}

/**
 * The trade overview (TASK-139): Klimper's own key in the prompt, computed by the wallet from
 * the same simulation (`trade.rs`). What is received includes what the user's spends are
 * bound to receive from the other side, which is why it exists only for a bound or plain spend.
 */
export interface TradeView {
  /** For a take: how many offers are taken together, all or nothing. */
  readonly offers: number | null;
  readonly pays: readonly TradeLegView[];
  readonly receives: readonly TradeLegView[];
  /** The fee the user's own spends pay. */
  readonly fee: BigNumber;
  readonly price: PriceView | null;
}

/** The terms of an option, proven by the request (wallet-kit, spec 11.1.1). */
export interface OptionTermsView {
  /** Receives the strike, may take the collateral back from the expiry on (p2 puzzle hash). */
  readonly creator: string;
  /** Unix seconds: exercise strictly before, clawback from then on. */
  readonly expiresAt: number;
  readonly collateral: TradeLegView;
  readonly strike: TradeLegView;
}

/** What a request does with one option (TASK-140), every fact computed by the wallet. */
export type OptionEffectView =
  | {
      readonly action: 'mint';
      readonly launcherId: string;
      readonly owner: string;
      readonly ownerIsUser: boolean;
      readonly creatorIsUser: boolean;
      readonly fundedByUser: boolean;
      /** False: the new option is only conditional on spends the user does not control. */
      readonly guaranteed: boolean;
      readonly terms: OptionTermsView;
    }
  | {
      readonly action: 'transfer';
      readonly launcherId: string;
      readonly fromUser: boolean;
      readonly to: string;
      readonly toUser: boolean;
      /** The new owner is the offer settlement puzzle: the option is offered (TASK-142). */
      readonly toSettlement: boolean;
    }
  | {
      readonly action: 'exercise';
      readonly launcherId: string;
      readonly byUser: boolean;
      readonly creatorIsUser: boolean;
      readonly terms: OptionTermsView;
    }
  | {
      readonly action: 'clawback';
      /** Null when the request does not prove which option it was. */
      readonly launcherId: string | null;
      readonly creator: string;
      readonly creatorIsUser: boolean;
      readonly collateral: TradeLegView;
      readonly unlocksAt: number;
    };

/**
 * A loan (TASK-140): Klimper's own key, from the request it built itself. A borrower signs a
 * loan request (an offer) the website hands to the lender; a lender takes one.
 */
export interface LoanView {
  readonly role: 'borrower' | 'lender';
  /** What the borrower receives from the lender now. */
  readonly principal: TradeLegView;
  /** The lender, for a borrower; the borrower, for a lender (p2 puzzle hash). */
  readonly counterparty: string;
  /** What the borrower's request gives whoever takes it, to bind the lender's payment. */
  readonly tie: TradeLegView | null;
  /** The option the loan is made of: its mint, from `summary.options`. */
  readonly mint: Extract<OptionEffectView, { action: 'mint' }>;
}

/** An option an offer gives, with its terms as the wallet holds them (TASK-142). */
export interface OfferedOptionView {
  readonly launcherId: string;
  readonly terms: OptionTermsView;
}

/**
 * An offer the wallet makes (`chia_createOffer`, TASK-142): Klimper's own key, from the
 * request it built itself. What it gives (options with their terms, XCH and CATs) and what
 * whoever takes it pays the user.
 */
export interface OfferView {
  readonly gives: readonly TradeLegView[];
  readonly options: readonly OfferedOptionView[];
  readonly asks: readonly TradeLegView[];
  /** Unix seconds from which nobody can take it (the earliest expiry of its options). */
  readonly expiresAt: number | null;
}

export interface SpendPromptView {
  readonly kind: 'spend';
  /** Klimper signs **and sends** this (`submit_coin_spends`); otherwise it only signs. */
  readonly broadcasts: boolean;
  /** Klimper built the spend itself from the request (`chia_send`). */
  readonly walletBuilt: boolean;
  /** The wallet database and the independent simulation were compared and agree. */
  readonly crossChecked: boolean;
  /**
   * wallet-kit read the request as sealed (`linking: "sealed"`, xchonnect spec 11.2):
   * Klimper built it, sends it itself and keeps every signature, so the user's spends reach
   * the chain together or not at all. Only for a request Klimper builds and sends.
   */
  readonly sealed: boolean;
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
  /**
   * Coin ids of the spends Klimper signs without recognising the coin as the user's
   * (`summary.unaccounted_spends`): what they move is unknown. Shown as a danger.
   */
  readonly unaccountedSpends: readonly string[];
  readonly timeLocks: TimeLockView;
  readonly signatures: readonly SignatureView[];
  readonly foreignSignatures: number;
  readonly hasUnsafe: boolean;
  /** `partialSign`: one side of a multi-party spend. */
  readonly partial: boolean;
  readonly binding: BindingView | null;
  /** What is paid for what, when the spend is a trade; `null` otherwise. */
  readonly trade: TradeView | null;
  /** Option contracts the request writes, uses, sends or claims back (TASK-140). */
  readonly options: readonly OptionEffectView[];
  /** A loan request or a loan, when the request is one; `null` otherwise. */
  readonly loan: LoanView | null;
  /** Klimper signs an offer (a loan request) and sends nothing to the network itself. */
  readonly makesOffer: boolean;
  /** An offer the wallet makes, when the request is one; `null` otherwise. */
  readonly offer: OfferView | null;
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
 * A decimal string, which is what wallet-kit writes since Xchonnect 0.1.0 (an amount above
 * 2^53 mojos, about 9007 XCH, has already lost precision as a JSON number by the time
 * `JSON.parse` has run), or a JSON number, as Klimper's own keys and older builds write it.
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

/** How an option contract is named: by both ends of its launcher id (TASK-140). */
export function optionLabel(launcherId: string): string {
  return `Option ${launcherId.slice(0, 8)}…${launcherId.slice(-6)}`;
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

/** Hex of exactly `bytes` bytes: an id, a puzzle hash, a key. A shorter one is not that. */
function exactHex(value: unknown, bytes: number): string | null {
  const hex = hexOf(value, bytes);
  return hex !== null && hex.length === bytes * 2 ? hex : null;
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
  isOption: boolean;
  assetId: string | null;
} {
  // wallet-kit always names the asset (`{ "type": … }`): one missing is not XCH by default.
  if (!isRecord(value)) throw new UnreadableAmount();
  if (value.type === 'cat' || value.type === 'option') {
    // A token whose id is not a full 32 bytes cannot be named: nobody can consent to it.
    const assetId = exactHex(value.asset_id, 32);
    if (assetId === null) throw new UnreadableAmount();
    return value.type === 'cat'
      ? { label: catLabel(assetId), isXch: false, isOption: false, assetId }
      : { label: optionLabel(assetId), isXch: false, isOption: true, assetId };
  }
  // A kind of asset this build does not know is not shown as XCH: nobody can consent to it.
  if (value.type !== 'xch') throw new UnreadableAmount();
  return { label: xchLabel, isXch: true, isOption: false, assetId: null };
}

/** Decimals of an asset: XCH in mojos, a CAT in thousandths, an option counted whole. */
function precisionOf(asset: { isXch: boolean; isOption: boolean }): number {
  if (asset.isXch) return XCH_PRECISION;
  return asset.isOption ? OPTION_PRECISION : CAT_PRECISION;
}

function assetViewOf(entry: unknown, xchLabel: string): AssetView | null {
  if (!isRecord(entry)) return null;
  const asset = assetOf(entry.asset, xchLabel);
  return {
    label: asset.label,
    isXch: asset.isXch,
    isOption: asset.isOption,
    assetId: asset.assetId,
    precision: precisionOf(asset),
    net: amountOf(entry.net),
    conditionalReceived: amountOf(entry.conditional_received),
    sent: amountOf(entry.sent),
    received: amountOf(entry.received),
  };
}

function outputViewOf(entry: unknown, xchLabel: string): OutputView | null {
  if (!isRecord(entry)) return null;
  const asset = assetOf(entry.asset, xchLabel);
  const option = optionOutputOf(entry.option);
  const recipient = exactHex(entry.recipient, 32);
  return {
    label: asset.label,
    isXch: asset.isXch,
    isOption: asset.isOption,
    precision: precisionOf(asset),
    amount: amountOf(entry.amount),
    // An option's collateral or launcher has no recipient: it is nobody's payment.
    recipient: option === null ? recipient : null,
    toUser: boolOf(entry.to_user),
    option,
    toSettlement:
      option === null &&
      exactHex(entry.puzzle_hash, 32) === SETTLEMENT_PAYMENT_HASH,
    unaccounted: optionalStrictBool(entry.unaccounted),
  };
}

/**
 * A flag a newer wallet-kit writes: absent (an older one) is false; present, it must be a
 * boolean, or the prompt is unreadable.
 */
function optionalStrictBool(value: unknown): boolean {
  if (value === undefined) return false;
  if (typeof value !== 'boolean') throw new UnreadableAmount();
  return value;
}

/**
 * `summary.unaccounted_spends`: absent (an older wallet-kit) is none; present, every entry
 * must be a coin id, or the prompt is unreadable. Never shortened.
 */
function unaccountedSpendsOf(value: unknown): string[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new UnreadableAmount();
  return value.map((entry) => strictHex32(entry));
}

/** The offer settlement puzzle's hash (chia-puzzles `SETTLEMENT_PAYMENT_HASH`). */
export const SETTLEMENT_PAYMENT_HASH =
  'cfbfdeed5c4ca2de3d0bf520b9cb4bb7743a359bd2e6a188d19ce7dffc21d3e7';

/** `outputs[].option`: absent or null is none; anything else must be exact. */
function optionOutputOf(value: unknown): OptionOutputView | null {
  if (value === undefined || value === null) return null;
  if (!isRecord(value)) throw new UnreadableAmount();
  const launcherId = exactHex(value.launcher_id, 32);
  if (launcherId === null) throw new UnreadableAmount();
  if (value.role !== 'collateral' && value.role !== 'launcher') {
    throw new UnreadableAmount();
  }
  return { launcherId, role: value.role };
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
    const precision = precisionOf(asset);
    if (entry.type === 'recipient') {
      const recipient = exactHex(entry.puzzle_hash, 32);
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
  const kind = kindOf(value.kind);
  // Absent is an older wallet-kit: not verified. Present, it must be a boolean, and a kind
  // the wallet verifies; a claim of verification on anything else is not one to show.
  const flag = value.kind_verified;
  if (flag !== undefined && typeof flag !== 'boolean') {
    throw new UnreadableAmount();
  }
  const kindVerified = flag === true;
  if (kindVerified && (kind === null || !VERIFIED_KINDS.includes(kind))) {
    throw new UnreadableAmount();
  }
  return {
    kind,
    kindVerified,
    verified,
    recipientsComplete: boolOf(value.recipients_complete),
  };
}

/** The kinds wallet-kit verifies against the simulation (spec 9.1). */
export const VERIFIED_KINDS: readonly string[] = [
  'option.mint',
  'option.transfer',
  'option.exercise',
  'option.clawback',
  'loan.request',
  'loan.take',
];

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
    notBeforeSeconds: optionalSeconds(value.not_before_seconds),
    notBeforeHeight: intOf(value.not_before_height),
    expiresSeconds: optionalSeconds(value.expires_seconds),
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
      precision: precisionOf(asset),
      amount: amountOf(entry.amount),
    });
  }
  return { allBound: boolOf(value.all_bound), payments };
}

/** A price as the wallet writes it: digits, at most one point. Anything else is dropped. */
function decimalOf(value: unknown): string | null {
  return typeof value === 'string' && /^\d{1,40}(\.\d{1,40})?$/.test(value)
    ? value
    : null;
}

function legOf(entry: unknown, xchLabel: string): TradeLegView {
  if (!isRecord(entry)) throw new UnreadableAmount();
  const asset = assetOf(entry.asset, xchLabel);
  const amount = amountOf(entry.amount);
  // A leg is a positive amount; anything else is a trade nobody can read.
  if (!amount.isGreaterThan(0)) throw new UnreadableAmount();
  return {
    ...asset,
    precision: precisionOf(asset),
    amount,
  };
}

/** A strict boolean: absent or anything else makes the prompt unreadable. */
function strictBool(value: unknown): boolean {
  if (typeof value !== 'boolean') throw new UnreadableAmount();
  return value;
}

function strictHex32(value: unknown): string {
  const hex = exactHex(value, 32);
  if (hex === null) throw new UnreadableAmount();
  return hex;
}

/**
 * Unix seconds: a decimal string (wallet-kit since Xchonnect 0.1.0) or a whole,
 * non-negative number, either below 2^53. A time that cannot be shown exactly is unreadable.
 */
function strictSeconds(value: unknown): number {
  // A digit string past 2^53 rounds to at least 2^53, which is not a safe integer.
  const seconds =
    typeof value === 'string' && /^\d+$/.test(value) ? Number(value) : value;
  if (
    typeof seconds !== 'number' ||
    !Number.isSafeInteger(seconds) ||
    seconds < 0
  ) {
    throw new UnreadableAmount();
  }
  return seconds;
}

/** `strictSeconds`, absent or null being none (a time lock the request does not set). */
function optionalSeconds(value: unknown): number | null {
  return value === undefined || value === null ? null : strictSeconds(value);
}

/** An option leg: XCH or a CAT (an option backed by an option is not one this build reads). */
function optionLegOf(entry: unknown, xchLabel: string): TradeLegView {
  const leg = legOf(entry, xchLabel);
  if (leg.isOption) throw new UnreadableAmount();
  return leg;
}

function termsOf(value: unknown, xchLabel: string): OptionTermsView {
  if (!isRecord(value)) throw new UnreadableAmount();
  return {
    creator: strictHex32(value.creator_puzzle_hash),
    expiresAt: strictSeconds(value.expiration_seconds),
    collateral: optionLegOf(value.collateral, xchLabel),
    strike: optionLegOf(value.strike, xchLabel),
  };
}

/**
 * `summary.options`, by explicit key. Absent is none (an older wallet-kit, or no option);
 * present, every entry must be one of the four actions exactly as wallet-kit writes it, or
 * the whole prompt is unreadable: these lines sit at the top of the screen.
 */
function optionsOf(value: unknown, xchLabel: string): OptionEffectView[] {
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value)) throw new UnreadableAmount();
  return value.map((entry): OptionEffectView => {
    if (!isRecord(entry)) throw new UnreadableAmount();
    switch (entry.action) {
      case 'mint':
        return {
          action: 'mint',
          launcherId: strictHex32(entry.launcher_id),
          owner: strictHex32(entry.owner),
          ownerIsUser: strictBool(entry.owner_is_user),
          creatorIsUser: strictBool(entry.creator_is_user),
          fundedByUser: strictBool(entry.funded_by_user),
          guaranteed: strictBool(entry.guaranteed),
          terms: termsOf(entry.terms, xchLabel),
        };
      case 'transfer':
        return {
          action: 'transfer',
          launcherId: strictHex32(entry.launcher_id),
          fromUser: strictBool(entry.from_user),
          to: strictHex32(entry.to),
          toUser: strictBool(entry.to_user),
          // Absent from an older wallet-kit: then not an offer.
          toSettlement:
            entry.to_settlement === undefined
              ? false
              : strictBool(entry.to_settlement),
        };
      case 'exercise':
        return {
          action: 'exercise',
          launcherId: strictHex32(entry.launcher_id),
          byUser: strictBool(entry.by_user),
          creatorIsUser: strictBool(entry.creator_is_user),
          terms: termsOf(entry.terms, xchLabel),
        };
      case 'clawback':
        return {
          action: 'clawback',
          launcherId:
            entry.launcher_id === null ? null : strictHex32(entry.launcher_id),
          creator: strictHex32(entry.creator),
          creatorIsUser: strictBool(entry.creator_is_user),
          collateral: optionLegOf(entry.collateral, xchLabel),
          unlocksAt: strictSeconds(entry.unlocks_at_seconds),
        };
      default:
        // An action this build does not know: nobody can consent to it.
        throw new UnreadableAmount();
    }
  });
}

/**
 * Klimper's `loan` key. Absent is `null`; present, it must be exact, and the option it is made
 * of must be in `summary.options` with the user on the side the role says: the borrower holds
 * the option and funded its collateral, the lender is its creator. Anything else is unreadable.
 */
function loanOf(
  value: unknown,
  options: readonly OptionEffectView[],
  xchLabel: string,
): LoanView | null {
  if (value === undefined || value === null) return null;
  if (!isRecord(value)) throw new UnreadableAmount();
  const role = value.role;
  if (role !== 'borrower' && role !== 'lender') throw new UnreadableAmount();
  const principal = optionLegOf(value.principal, xchLabel);
  const counterparty = strictHex32(value.counterparty);
  const tie =
    value.tie === undefined || value.tie === null
      ? null
      : optionLegOf(value.tie, xchLabel);
  const mints = options.filter(
    (effect): effect is Extract<OptionEffectView, { action: 'mint' }> =>
      effect.action === 'mint',
  );
  const mint = mints.length === 1 ? mints[0] : undefined;
  if (mint === undefined) throw new UnreadableAmount();
  const holds =
    role === 'borrower'
      ? mint.ownerIsUser && mint.fundedByUser && !mint.creatorIsUser
      : mint.creatorIsUser && !mint.fundedByUser && !mint.ownerIsUser;
  // The counterparty is the option's other party: the creator (lender) for a borrower, the
  // holder (borrower) for a lender.
  const other = role === 'borrower' ? mint.terms.creator : mint.owner;
  if (!holds || other !== counterparty) throw new UnreadableAmount();
  return { role, principal, counterparty, tie, mint };
}

/**
 * Klimper's `offer` key (TASK-142). Absent is `null`; present, it must be exact, it must ask
 * for something and give something, and every option it gives must be one the simulation sees
 * leaving the user for the settlement puzzle (`summary.options`), once. Anything else is
 * unreadable.
 */
function offerOf(
  value: unknown,
  options: readonly OptionEffectView[],
  xchLabel: string,
): OfferView | null {
  if (value === undefined || value === null) return null;
  if (!isRecord(value)) throw new UnreadableAmount();
  if (!Array.isArray(value.gives) || !Array.isArray(value.asks)) {
    throw new UnreadableAmount();
  }
  if (!Array.isArray(value.options)) throw new UnreadableAmount();
  const gives = value.gives.map((entry) => optionLegOf(entry, xchLabel));
  const asks = value.asks.map((entry) => optionLegOf(entry, xchLabel));
  const offered = value.options.map((entry): OfferedOptionView => {
    if (!isRecord(entry)) throw new UnreadableAmount();
    return {
      launcherId: strictHex32(entry.launcher_id),
      terms: termsOf(entry.terms, xchLabel),
    };
  });
  if (asks.length === 0 || gives.length + offered.length === 0) {
    throw new UnreadableAmount();
  }
  const transfers = options.filter(
    (effect): effect is Extract<OptionEffectView, { action: 'transfer' }> =>
      effect.action === 'transfer',
  );
  for (const option of offered) {
    const seen = transfers.filter((t) => t.launcherId === option.launcherId);
    if (
      seen.length !== 1 ||
      !seen[0].fromUser ||
      !seen[0].toSettlement ||
      offered.filter((o) => o.launcherId === option.launcherId).length !== 1
    ) {
      throw new UnreadableAmount();
    }
  }
  const expiresAt =
    value.expires_at === undefined || value.expires_at === null
      ? null
      : strictSeconds(value.expires_at);
  return { gives, options: offered, asks, expiresAt };
}

/**
 * The trade overview. Absent is `null` (not a trade, or an older plugin); present but
 * malformed makes the whole prompt unreadable, since its figures sit at the top of the screen.
 */
function tradeOf(value: unknown, xchLabel: string): TradeView | null {
  if (value === undefined || value === null) return null;
  if (!isRecord(value)) throw new UnreadableAmount();
  const pays = arrayOf(value.pays).map((entry) => legOf(entry, xchLabel));
  const receives = arrayOf(value.receives).map((entry) =>
    legOf(entry, xchLabel),
  );
  if (pays.length === 0 || receives.length === 0) {
    throw new UnreadableAmount();
  }
  // A count: a whole number, from one to a bound no real aggregate comes near.
  const offers =
    typeof value.offers === 'number' &&
    Number.isSafeInteger(value.offers) &&
    value.offers >= 1 &&
    value.offers <= 10_000
      ? value.offers
      : null;
  let price: PriceView | null = null;
  if (isRecord(value.price)) {
    const quotePerBase = decimalOf(value.price.quote_per_base);
    const basePerQuote = decimalOf(value.price.base_per_quote);
    if (quotePerBase !== null && basePerQuote !== null) {
      price = {
        base: assetOf(value.price.base, xchLabel),
        quote: assetOf(value.price.quote, xchLabel),
        quotePerBase,
        basePerQuote,
      };
    }
  }
  return {
    offers,
    pays,
    receives,
    fee: amountOf(value.fee),
    price,
  };
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
      publicKey: exactHex(prompt.public_key, 48) ?? '',
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
      publicKey: exactHex(entry.public_key, 48) ?? '',
      isUnsafe: boolOf(entry.is_unsafe),
      opcode: intOf(entry.opcode),
    });
  }

  const unknownPuzzles: string[] = [];
  for (const entry of arrayOf(summary.unknown_puzzles)) {
    const hash = exactHex(entry, 32);
    if (hash !== null) unknownPuzzles.push(hash);
  }

  const unaccountedSpends = unaccountedSpendsOf(summary.unaccounted_spends);

  const impliedFee =
    summary.implied_fee === null || summary.implied_fee === undefined
      ? null
      : amountOf(summary.implied_fee);

  const options = optionsOf(summary.options, xchLabel);
  // Klimper signs an offer and sends nothing: only for the wallet's own build.
  if (
    prompt.makes_offer !== undefined &&
    typeof prompt.makes_offer !== 'boolean'
  ) {
    return { kind: 'unreadable' };
  }
  const makesOffer = broadcasts && prompt.makes_offer === true;
  // Which links between the user's spends counted: absent (an older wallet-kit, or a
  // signature request), "direct" or "sealed". Anything else is a prompt this build does not
  // know.
  if (
    prompt.linking !== undefined &&
    prompt.linking !== 'direct' &&
    prompt.linking !== 'sealed'
  ) {
    return { kind: 'unreadable' };
  }
  const walletBuilt = broadcasts && boolOf(prompt.wallet_built);

  return {
    kind: 'spend',
    broadcasts,
    walletBuilt,
    crossChecked: boolOf(prompt.cross_checked),
    sealed: walletBuilt && prompt.linking === 'sealed',
    intent: intentOf(prompt.intent, xchLabel),
    xchLabel,
    assets,
    outputs,
    impliedFee,
    reserveFee: amountOf(summary.reserve_fee),
    unknownPuzzles,
    unaccountedSpends,
    timeLocks: timeLocksOf(summary.time_locks),
    signatures,
    foreignSignatures: arrayOf(plan.foreign).length,
    // `has_unsafe` is the summary flag; a single `is_unsafe` requirement is enough on its
    // own, so the warning does not depend on both being written.
    hasUnsafe: boolOf(plan.has_unsafe) || signatures.some((s) => s.isUnsafe),
    // A submitted bundle another party's signature completes is the binding case too.
    partial: boolOf(prompt.partial) || (broadcasts && isRecord(prompt.binding)),
    binding: bindingOf(prompt.binding, xchLabel),
    trade: tradeOf(prompt.trade, xchLabel),
    options,
    loan: loanOf(prompt.loan, options, xchLabel),
    makesOffer,
    offer: makesOffer ? offerOf(prompt.offer, options, xchLabel) : null,
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
  return (
    view.kind === 'spend' &&
    (view.unknownPuzzles.length > 0 || hasUnaccounted(view))
  );
}

/** The request spends coins Klimper signs but cannot account for (spec 11.1 item 3). */
export function hasUnaccounted(view: SpendPromptView): boolean {
  return (
    view.unaccountedSpends.length > 0 ||
    view.outputs.some((output) => output.unaccounted)
  );
}
