// SPDX-License-Identifier: AGPL-3.0-or-later
// Option prompts as the plugin sends them (clapandpay TASK-140), for the tests and the
// preview: wallet-kit's `summary.options`, `outputs[].option`, the option asset, and Klimper's
// own `loan` and `makes_offer` keys. Wallet-kit writes every amount and Unix time as a decimal
// string (Xchonnect 0.1.0); Klimper's own `offer` writes its times as numbers.

export type Asset =
  | { type: 'xch' }
  | { type: 'cat'; asset_id: string }
  | { type: 'option'; asset_id: string };

export const XCH: Asset = { type: 'xch' };
export const cat = (byte: string): Asset => ({
  type: 'cat',
  asset_id: byte.repeat(64 / byte.length),
});
export const optionAsset = (launcherId: string): Asset => ({
  type: 'option',
  asset_id: launcherId,
});

/** One XCH in mojos. */
export const ONE_XCH = 1_000_000_000_000;
/** A CAT with three decimals: 1 whole token. */
export const ONE_CAT = 1_000;

export const LAUNCHER = '9c1f'.repeat(16);
export const LAUNCHER_2 = '77a0'.repeat(16);
export const ME = '4f45'.repeat(16);
export const LENDER = 'ab12'.repeat(16);
export const BORROWER = 'cd34'.repeat(16);
export const STRANGER = 'e5e5'.repeat(16);
export const COLLATERAL_COIN = 'e2d0'.repeat(16);
export const SETTLEMENT =
  'cfbfdeed5c4ca2de3d0bf520b9cb4bb7743a359bd2e6a188d19ce7dffc21d3e7';
export const USDC = cat('3c');
/** 13 Nov 2026 14:00 UTC. */
export const EXPIRY = 1_794_578_400;

export const leg = (asset: Asset, amount: number) => ({
  asset,
  amount: String(amount),
});

export function terms({
  creator = ME,
  collateral = leg(XCH, 100 * ONE_XCH),
  strike = leg(USDC, 500 * ONE_CAT),
  expiresAt = EXPIRY,
}: {
  creator?: string;
  collateral?: ReturnType<typeof leg>;
  strike?: ReturnType<typeof leg>;
  expiresAt?: number;
} = {}) {
  return {
    creator_puzzle_hash: creator,
    expiration_seconds: String(expiresAt),
    collateral,
    strike,
  };
}

export function mint(overrides: Record<string, unknown> = {}) {
  return {
    action: 'mint',
    launcher_id: LAUNCHER,
    owner: ME,
    owner_is_user: true,
    creator_is_user: true,
    funded_by_user: true,
    guaranteed: true,
    collateral_coin_id: COLLATERAL_COIN,
    terms: terms(),
    ...overrides,
  };
}

export function exercise(overrides: Record<string, unknown> = {}) {
  return {
    action: 'exercise',
    launcher_id: LAUNCHER,
    by_user: true,
    creator_is_user: false,
    terms: terms({ creator: STRANGER }),
    ...overrides,
  };
}

export function clawback(overrides: Record<string, unknown> = {}) {
  return {
    action: 'clawback',
    launcher_id: LAUNCHER,
    creator: ME,
    creator_is_user: true,
    collateral: leg(XCH, 100 * ONE_XCH),
    unlocks_at_seconds: String(EXPIRY),
    ...overrides,
  };
}

export function transfer(overrides: Record<string, unknown> = {}) {
  return {
    action: 'transfer',
    launcher_id: LAUNCHER,
    from_user: true,
    to: STRANGER,
    to_user: false,
    ...overrides,
  };
}

type Delta = [Asset, number, number, number?];

/** A wallet-built request with option effects, as `UiApprover` hands it to the screen. */
export function optionPrompt({
  method = 'chia_mintOption',
  options = [mint()],
  assets = [
    [XCH, 100 * ONE_XCH + 1_000_000 + 2, 0],
    [optionAsset(LAUNCHER), 0, 1],
  ] as Delta[],
  outputs = [
    {
      asset: XCH,
      amount: String(100 * ONE_XCH),
      puzzle_hash: 'f1'.repeat(32),
      recipient: null,
      to_user: false,
      option: { launcher_id: LAUNCHER, role: 'collateral' },
    },
    {
      asset: XCH,
      amount: '1',
      puzzle_hash: 'eff0'.repeat(16),
      recipient: null,
      to_user: false,
      option: { launcher_id: LAUNCHER, role: 'launcher' },
    },
    {
      asset: XCH,
      amount: String(4 * ONE_XCH),
      puzzle_hash: ME,
      recipient: ME,
      to_user: true,
      option: null,
    },
  ] as unknown[],
  fee = 1_000_000,
  kind = 'option.mint',
  extra = {},
}: {
  method?: string;
  options?: unknown[];
  assets?: Delta[];
  outputs?: unknown[];
  fee?: number;
  kind?: string | null;
  extra?: Record<string, unknown>;
} = {}) {
  return {
    type: 'submit_coin_spends',
    dapp: 'options.example',
    method,
    wallet_built: true,
    summary: {
      assets: assets.map(([asset, sent, received, conditional = 0]) => ({
        asset,
        sent: String(sent),
        received: String(received),
        conditional_received: String(conditional),
        net: String(received - sent),
      })),
      implied_fee: String(fee),
      reserve_fee: String(fee),
      time_locks: {},
      unknown_puzzles: [],
      cost: 1,
      spends: [],
      outputs,
      options,
    },
    plan: {
      ours: [
        {
          spend: 0,
          opcode: 50,
          public_key: 'a'.repeat(96),
          message: 'ff',
          is_unsafe: false,
        },
      ],
      foreign: [],
      has_unsafe: false,
    },
    binding: null,
    intent:
      kind === null
        ? null
        : {
            kind,
            kind_verified: true,
            verified: [{ type: 'fee', amount: String(fee) }],
            recipients_complete: true,
          },
    cross_checked: true,
    ...extra,
  };
}

/** The borrower's loan request: 300 wUSDC from the lender against 100 XCH, repay 315. */
export function loanRequest(extra: Record<string, unknown> = {}) {
  return optionPrompt({
    options: [
      mint({
        terms: terms({ creator: LENDER, strike: leg(USDC, 315 * ONE_CAT) }),
        creator_is_user: false,
      }),
    ],
    outputs: [
      {
        asset: XCH,
        amount: String(100 * ONE_XCH),
        puzzle_hash: 'f1'.repeat(32),
        recipient: null,
        to_user: false,
        option: { launcher_id: LAUNCHER, role: 'collateral' },
      },
      {
        asset: XCH,
        amount: '1',
        puzzle_hash: SETTLEMENT,
        recipient: SETTLEMENT,
        to_user: false,
        option: null,
      },
    ],
    kind: 'loan.request',
    extra: {
      makes_offer: true,
      loan: {
        role: 'borrower',
        principal: leg(USDC, 300 * ONE_CAT),
        counterparty: LENDER,
        tie: leg(XCH, 1),
      },
      ...extra,
    },
  });
}

/** The lender taking it: pays 300 wUSDC, receives the 1-mojo tie. */
export function loanTake(extra: Record<string, unknown> = {}) {
  return optionPrompt({
    method: 'chia_takeOffer',
    options: [
      mint({
        owner: BORROWER,
        owner_is_user: false,
        creator_is_user: true,
        funded_by_user: false,
        guaranteed: false,
        terms: terms({ creator: ME, strike: leg(USDC, 315 * ONE_CAT) }),
      }),
    ],
    assets: [
      [USDC, 300 * ONE_CAT, 0],
      [XCH, 1_000_000, 0, 1],
    ],
    outputs: [],
    kind: 'loan.take',
    extra: {
      binding: {
        all_bound: true,
        bound_payments: [{ asset: XCH, amount: '1' }],
        bound_received: [{ asset: XCH, amount: '1' }],
      },
      trade: {
        offers: 1,
        pays: [{ asset: USDC, amount: String(300 * ONE_CAT) }],
        receives: [{ asset: XCH, amount: '1' }],
        fee: '1000000',
        price: null,
      },
      loan: {
        role: 'lender',
        principal: leg(USDC, 300 * ONE_CAT),
        counterparty: BORROWER,
        tie: leg(XCH, 1),
      },
      ...extra,
    },
  });
}

/**
 * The lender's open loan offer (TASK-142): the option (100 XCH locked, strike 315 wUSDC) and
 * 300 wUSDC given, 100 XCH asked; signed, not sent.
 */
export function openLoanOffer(extra: Record<string, unknown> = {}) {
  return optionPrompt({
    method: 'chia_createOffer',
    options: [transfer({ to: SETTLEMENT, to_settlement: true })],
    assets: [
      [USDC, 300 * ONE_CAT, 0],
      [optionAsset(LAUNCHER), 1, 0],
    ],
    outputs: [
      {
        asset: USDC,
        amount: String(300 * ONE_CAT),
        puzzle_hash: SETTLEMENT,
        recipient: SETTLEMENT,
        to_user: false,
        option: null,
      },
    ],
    fee: 0,
    kind: 'option.transfer',
    extra: {
      makes_offer: true,
      linking: 'direct',
      offer: {
        gives: [leg(USDC, 300 * ONE_CAT)],
        options: [
          {
            launcher_id: LAUNCHER,
            terms: {
              ...terms({ strike: leg(USDC, 315 * ONE_CAT) }),
              expiration_seconds: EXPIRY,
            },
          },
        ],
        asks: [leg(XCH, 100 * ONE_XCH)],
        expires_at: EXPIRY,
      },
      ...extra,
    },
  });
}

/**
 * A partial offer the wallet makes (`chia_createPartialOffer`, TASK-165): 300 wUSDC (and the
 * mojo of the security coin) for 100 XCH, any part from 10 XCH up, 5 mojos per fill, until
 * the expiry; signed, not sent.
 */
export function partialOffer(extra: Record<string, unknown> = {}) {
  return optionPrompt({
    method: 'chia_createPartialOffer',
    options: [],
    assets: [
      [USDC, 300 * ONE_CAT, 0],
      [XCH, 1, 0],
    ],
    outputs: [
      {
        asset: USDC,
        amount: String(300 * ONE_CAT),
        puzzle_hash: SETTLEMENT,
        recipient: SETTLEMENT,
        to_user: false,
        option: null,
      },
      {
        asset: XCH,
        amount: '1',
        puzzle_hash: SETTLEMENT,
        recipient: SETTLEMENT,
        to_user: false,
        option: null,
      },
    ],
    fee: 0,
    // `offer.create` is a label wallet-kit does not verify.
    kind: null,
    extra: {
      makes_offer: true,
      linking: 'direct',
      intent: {
        kind: 'offer.create',
        kind_verified: false,
        verified: [{ type: 'fee', amount: '0' }],
        recipients_complete: false,
      },
      offer: {
        gives: [leg(USDC, 300 * ONE_CAT), leg(XCH, 1)],
        options: [],
        asks: [leg(XCH, 100 * ONE_XCH)],
        expires_at: EXPIRY,
        partial: {
          min_fill: leg(XCH, 10 * ONE_XCH),
          min_fill_offered: leg(USDC, 30 * ONE_CAT),
          fee_per_fill: leg(XCH, 5),
        },
      },
      ...extra,
    },
  });
}
