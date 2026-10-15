// SPDX-License-Identifier: AGPL-3.0-or-later
// The Tauri backend for the approvals preview: sample requests and their answers.
import { emit } from '@tauri-apps/api/event';
import { mockIPC } from '@tauri-apps/api/mocks';
import * as opt from '@/klimper/xchonnect/optionFixtures';

const now = () => Math.floor(Date.now() / 1000);
const TAIL = '1a2b3c4d'.repeat(8);
function spend(
  id: string,
  dapp: string,
  assets: unknown[],
  extra: Record<string, unknown> = {},
) {
  return {
    id,
    sessionId: 's1',
    dapp,
    method: 'signCoinSpends',
    expiresAt: now() + 271,
    kind: 'request',
    prompt: {
      type: 'sign_coin_spends',
      dapp,
      summary: {
        assets,
        implied_fee: 500_000,
        reserve_fee: 0,
        time_locks: {},
        unknown_puzzles: [],
        cost: 1,
        spends: [],
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
      partial: false,
      binding: null,
      ...extra,
    },
  };
}
let pending: unknown[] = [
  spend('a1', 'pengui.space', [
    {
      asset: { type: 'xch' },
      sent: 1_500_000_000_000,
      received: 0,
      conditional_received: 0,
      net: -1_500_000_000_000,
    },
  ]),
  spend(
    'a2',
    'dexie.space',
    [
      {
        asset: { type: 'xch' },
        sent: 250_000_000_000,
        received: 0,
        conditional_received: 0,
        net: -250_000_000_000,
      },
      {
        asset: { type: 'cat', asset_id: TAIL },
        sent: 0,
        received: 0,
        conditional_received: 1_000_000,
        net: 0,
      },
    ],
    {
      partial: true,
      binding: {
        all_bound: true,
        unbound_user_spends: [],
        bound_payments: [
          {
            user_spend: 0,
            settlement_spend: 1,
            asset: { type: 'cat', asset_id: TAIL },
            amount: 1_000_000,
          },
        ],
        bound_received: [],
      },
    },
  ),
  {
    id: 'm1',
    sessionId: 's2',
    dapp: 'mintgarden.io',
    method: 'signMessage',
    expiresAt: now() + 48,
    kind: 'request',
    prompt: {
      type: 'sign_message',
      dapp: 'mintgarden.io',
      message_hex: '00',
      message_text: 'Sign in to mintgarden.io\nNonce: 8f2c41',
      public_key: 'b'.repeat(96),
    },
  },
];
// ---------------------------------------------------------------------------------------
// Trades (TASK-139): `?trade=2|5|10` shows one take with that many assets in all, the way
// the plugin sends it: wallet-kit's prompt plus Klimper's `trade` and `cross_checked`.
// ---------------------------------------------------------------------------------------

type Asset = { type: 'xch' } | { type: 'cat'; asset_id: string };
const XCH: Asset = { type: 'xch' };
const cat = (seed: string): Asset => ({
  type: 'cat',
  asset_id: seed.repeat(64 / seed.length),
});
/** The wallet's own records for some of the CATs: ticker, name, icon, balance. */
const TOKENS: Record<
  string,
  { ticker: string; name: string; icon?: string; balance: number }
> = {
  ['5b'.repeat(32)]: {
    ticker: 'SBX',
    name: 'Spacebucks',
    icon: 'https://icons.preview.invalid/sbx.svg',
    balance: 4_000,
  },
  ['d1'.repeat(32)]: {
    ticker: 'DBX',
    name: 'dexie bucks',
    icon: 'https://icons.preview.invalid/dbx.svg',
    balance: 12_500,
  },
  ['7e'.repeat(32)]: { ticker: 'HOA', name: 'HOA Coin', balance: 2_000_000 },
  ['a9'.repeat(32)]: { ticker: 'MZ', name: 'Monkeyzoo Token', balance: 0 },
  ['3c'.repeat(32)]: {
    ticker: 'wUSDC.b',
    name: 'Base warp.green USDC',
    balance: 75_000,
  },
  ['c4'.repeat(32)]: { ticker: 'XCH', name: 'Totally XCH', balance: 0 },
  // A look-alike in Cyrillic, held, with an icon of its own: neither is used.
  ['e2'.repeat(32)]: {
    ticker: 'ХСН',
    name: 'Chia Network',
    icon: 'https://icons.preview.invalid/xch.svg',
    balance: 1_000,
  },
  // A second token calling itself SBX: both get the twin warning.
  ['0f'.repeat(32)]: { ticker: 'SBX', name: 'Spacebucks', balance: 0 },
};

type Leg = [Asset, number];
const asJson = ([asset, amount]: Leg) => ({ asset, amount: String(amount) });

function take(
  id: string,
  pays: Leg[],
  receives: Leg[],
  offers: number,
  fee: number,
  price: Record<string, unknown> | null,
) {
  const assets = [
    ...pays.map(([asset, amount]) => ({
      asset,
      sent: amount + (asset.type === 'xch' ? fee : 0),
      received: 0,
      conditional_received: 0,
      net: -(amount + (asset.type === 'xch' ? fee : 0)),
    })),
    ...receives.map(([asset, amount]) => ({
      asset,
      sent: 0,
      received: 0,
      conditional_received: amount,
      net: 0,
    })),
  ];
  return {
    id,
    sessionId: 's1',
    dapp: 'pengui.space',
    method: 'chia_takeOffer',
    expiresAt: now() + 271,
    kind: 'request',
    prompt: {
      type: 'submit_coin_spends',
      dapp: 'pengui.space',
      method: 'chia_takeOffer',
      wallet_built: true,
      summary: {
        assets,
        implied_fee: fee + 250_000_000,
        reserve_fee: fee + 250_000_000,
        time_locks: {},
        unknown_puzzles: [],
        cost: 1,
        spends: [],
        outputs: [],
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
        foreign: [{}],
        has_unsafe: false,
      },
      binding: {
        all_bound: true,
        unbound_user_spends: [],
        bound_payments: receives.map(([asset, amount], index) => ({
          user_spend: 0,
          settlement_spend: index + 1,
          asset,
          amount,
        })),
        bound_received: receives.map(([asset, amount]) => ({ asset, amount })),
      },
      intent: { kind: 'offer.take', verified: [], recipients_complete: false },
      cross_checked: true,
      trade: {
        offers,
        pays: pays.map(asJson),
        receives: receives.map(asJson),
        fee: String(fee),
        price,
      },
    },
  };
}

const TRADES: Record<string, unknown> = {
  2: take(
    't2',
    [[XCH, 1_500_000_000_000]],
    [[cat('5b'), 3_000_000]],
    3,
    100_000_000,
    {
      base: cat('5b'),
      quote: XCH,
      quote_per_base: '0.0005',
      base_per_quote: '2000',
    },
  ),
  5: take(
    't5',
    [
      [XCH, 2_250_000_000_000],
      [cat('7e'), 1_250_000],
    ],
    [
      [cat('5b'), 3_000_000],
      [cat('d1'), 420_000],
      [cat('a9'), 99_999_999],
    ],
    4,
    50_000_000,
    null,
  ),
  10: take(
    't10',
    [
      [XCH, 12_345_678_901_234],
      [cat('3c'), 250_000_000],
    ],
    [
      [cat('5b'), 3_000_000],
      [cat('d1'), 420_000],
      [cat('a9'), 99_999_999],
      [cat('7e'), 18_000],
      [cat('c4'), 5_000],
      [cat('e2'), 1_000_000_000],
      [cat('0f'), 7_000],
      [cat('91'), 123_456_789],
    ],
    9,
    100_000_000,
    null,
  ),
};

// ---------------------------------------------------------------------------------------
// Option contracts (TASK-140): `?option=mint|exercise|claim|send|loan|lend` shows one
// option approval as the plugin sends it (`optionFixtures.ts`), on testnet's coin.
// ---------------------------------------------------------------------------------------

function optionApproval(method: string, prompt: Record<string, unknown>) {
  return {
    id: `o-${method}`,
    sessionId: 's1',
    dapp: 'options.example',
    method,
    expiresAt: now() + 271,
    kind: 'request',
    prompt,
  };
}

const soon = now() + 30 * 24 * 3600;
const OPTIONS: Record<string, () => unknown> = {
  mint: () =>
    optionApproval(
      'chia_mintOption',
      opt.optionPrompt({
        options: [opt.mint({ terms: opt.terms({ expiresAt: soon }) })],
      }),
    ),
  exercise: () =>
    optionApproval(
      'chia_exerciseOption',
      opt.optionPrompt({
        method: 'chia_exerciseOption',
        options: [opt.exercise()],
        assets: [
          [opt.USDC, 500 * opt.ONE_CAT, 0],
          [opt.XCH, 1_000_000, 1, 100 * opt.ONE_XCH],
          [opt.optionAsset(opt.LAUNCHER), 1, 0],
        ],
        outputs: [],
        kind: 'option.exercise',
        extra: {
          binding: {
            all_bound: true,
            bound_payments: [{ asset: opt.XCH, amount: 100 * opt.ONE_XCH }],
            bound_received: [{ asset: opt.XCH, amount: 100 * opt.ONE_XCH }],
          },
        },
      }),
    ),
  claim: () =>
    optionApproval(
      'chia_claimOption',
      opt.optionPrompt({
        method: 'chia_claimOption',
        options: [opt.clawback({ unlocks_at_seconds: now() - 86_400 })],
        assets: [[opt.XCH, 100 * opt.ONE_XCH, 100 * opt.ONE_XCH - 1_000_000]],
        outputs: [
          {
            asset: opt.XCH,
            amount: 100 * opt.ONE_XCH - 1_000_000,
            puzzle_hash: opt.ME,
            recipient: opt.ME,
            to_user: true,
            option: null,
          },
        ],
        kind: 'option.clawback',
      }),
    ),
  send: () =>
    optionApproval(
      'chia_transferOption',
      opt.optionPrompt({
        method: 'chia_transferOption',
        options: [opt.transfer()],
        assets: [
          [opt.optionAsset(opt.LAUNCHER), 1, 0],
          [opt.XCH, 1_000_000, 0],
        ],
        outputs: [],
        kind: 'option.transfer',
      }),
    ),
  loan: () => optionApproval('chia_mintOption', opt.loanRequest()),
  lend: () => optionApproval('chia_takeOffer', opt.loanTake()),
};

const params = new URLSearchParams(location.search);
if (params.get('n')) pending = pending.slice(0, Number(params.get('n')));
const optionCase = params.get('option');
if (optionCase !== null && OPTIONS[optionCase]) {
  pending = [OPTIONS[optionCase]()];
}
const tradeCase = params.get('trade');
if (tradeCase === '1') pending = pending.slice(0, 1);
else if (tradeCase !== null && TRADES[tradeCase]) pending = [TRADES[tradeCase]];

mockIPC(
  (cmd, args) => {
    if (cmd.endsWith('|pending_approvals')) return pending;
    if (cmd.endsWith('|list_sessions')) {
      return [
        {
          sessionId: 's1',
          dapp: 'pengui.space',
          dappName: 'Pengui',
          chainId: 'testnet11',
          createdAt: now() - 3600,
          epoch: 0,
          sasConfirmed: true,
          needsRepair: false,
          revoked: false,
          rotating: false,
          developerMode: false,
          methods: [
            'chainId',
            'connect',
            'getPublicKeys',
            'signCoinSpends',
            'signMessage',
          ],
          exposedKeys: [`0x${'b2c4'.repeat(24)}`],
          limits: {},
          allowAggSigUnsafe: false,
          allowUnknownContracts: false,
          allReceiveKeys: false,
          connected: true,
        },
      ];
    }
    if (cmd === 'get_user_themes') return { themes: [] };
    if (cmd === 'get_cats') {
      return {
        cats: Object.entries(TOKENS).map(([id, known]) => ({
          asset_id: id,
          name: known.name,
          ticker: known.ticker,
          precision: 3,
          description: null,
          icon_url: known.icon ?? null,
          visible: true,
          balance: String(known.balance),
          selectable_balance: String(known.balance),
          revocation_address: null,
        })),
      };
    }
    if (cmd === 'get_token') {
      const id = (args as { req: { asset_id: string | null } }).req.asset_id;
      if (id === null) {
        return {
          token: {
            asset_id: null,
            name: 'Chia',
            ticker: 'XCH',
            precision: 12,
            description: null,
            icon_url: null,
            visible: true,
            balance: '15250000000000',
            selectable_balance: '15250000000000',
            revocation_address: null,
          },
        };
      }
      const known = TOKENS[id];
      return {
        token: known
          ? {
              asset_id: id,
              name: known.name,
              ticker: known.ticker,
              precision: 3,
              description: null,
              icon_url: known.icon ?? null,
              visible: true,
              balance: String(known.balance),
              selectable_balance: String(known.balance),
              revocation_address: null,
            }
          : null,
      };
    }
    if (cmd.endsWith('|resolve_approval')) {
      const id = (args as { approvalId: string }).approvalId;
      pending = pending.filter((p) => (p as { id: string }).id !== id);
      void emit('klimper-xchonnect://approval-resolved', { id });
      return null;
    }
    return null;
  },
  { shouldMockEvents: true },
);
