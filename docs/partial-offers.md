# Partial offers (TASK-164)

Partial offers (CHIP-0052, Final) sell one asset for another at a fixed price, and takers fill
them in parts. The text is bech32m with the prefix `partial` (`partial1…`), compressed like an
offer. Splash relays them since 0.3.0 (2026-10-23). nodexch decodes and lists them, and after each
fill it rebuilds the next offer with what is left (nodexch TASK-231). Klimper does not handle them
yet. Neither does upstream Sage (no branch or PR), nor chia-wallet-sdk (no partial driver). The
reference implementation is [Yakuhito/partial](https://github.com/Yakuhito/partial).

## How a partial offer works

- The decoded spend bundle holds the maker's spends (only before the first fill: they create the
  first partial coin, memo `(maker_ph, requested_asset_info)`). It also holds exactly one
  placeholder spend whose coin has the puzzle hash `0101…01`.
- The placeholder's puzzle is the hint: lineage proof, offered and requested asset (asset id;
  hidden puzzle hash for a revocable CAT), price `(initial offered . initial requested)`, maker
  puzzle hash, quoted conditions (`ASSERT_BEFORE_SECONDS_ABSOLUTE` = expiry, `RESERVE_FEE` = fee per
  fill), and min fill − 1. Its parent and amount are the partial coin's.
- The partial coin is XCH, a CAT or a revocable CAT. Inside the asset wrapper sits P2_1_OF_N over
  `[partial puzzle, maker]`, so the maker can cancel at any time by spending the latest coin.
- A fill pays `x ≥ min` of the requested asset to the maker and receives
  `x * initial_offered / initial_requested`. A child coin with the same puzzle hash keeps the rest.
- A taker can fill with a normal offer.

## Layout: three replaceable parts

Sage, or chia-wallet-sdk, will likely ship partial offers itself. So Klimper's version lives in
files of its own, in three parts with one seam each, and Sage's crates (`sage`, `sage-wallet`,
`sage-api`, `sage-database`) get no edit at all. When upstream ships its own, each part is
deleted or swapped on its own, and the merge has nothing to conflict with.

| Part                   | Where                                       | Depends on                                     | Replaced by                               | The seam                     |
| ---------------------- | ------------------------------------------- | ---------------------------------------------- | ----------------------------------------- | ---------------------------- |
| 1. Driver              | `crates/chia-partial-offer` (new)           | chia-wallet-sdk only                           | a partial driver in chia-wallet-sdk       | its public functions (below) |
| 2. Wallet and commands | `tauri-plugin-klimper-partial-offers` (new) | part 1, `sage`, `sage-wallet`, `sage-database` | Sage's own endpoints                      | the plugin's commands        |
| 3. UI                  | `src/klimper/partial/` (new)                | the plugin's commands, through `api.ts` only   | Sage's pages, or kept with a new `api.ts` | `src/klimper/partial/api.ts` |

Xchonnect (Klimper-only for good) uses part 1 directly.

### 1. Driver: `crates/chia-partial-offer`

A plain library with no Sage types, shaped so it can be proposed to chia-wallet-sdk as
`driver/offers/partial.rs` as it is. Port from the reference (Yakuhito/partial):

- `PartialOffer`, `PartialOfferInfo`: `from_spend_bundle`, `to_spend_bundle`, `quote` and
  `reverse_quote`, `accept_offer`, `child`, `claw_back`; the puzzle and its hash. nodexch's
  `indexer/src/offers/partial.rs` derives the hash from the `chia-sdk-types` action-layer args;
  Klimper's chia-wallet-sdk 0.36.0 lacks `action-layer`, so enable that feature for this crate
  only, or curry the CAT makers by hand.
- `encode_partial_offer` / `decode_partial_offer` (prefix `partial`, compressed like an offer;
  the SDK's `compress.rs` knows only `offer`), and `is_partial_offer(text)`.
- `build_partial_offer(maker spends, terms)`: the one-sided offer that pays into the partial coin
  with the memo `(maker_ph, requested_asset_info)`, plus the hint spend. Refuse a non-standard
  maker puzzle, as the reference does.
- Tests against the reference vectors and nodexch's decoded offers.

Swap-out: when chia-wallet-sdk has a partial driver, delete the crate and change the imports in
parts 2 and Xchonnect; nothing else names it.

### 2. Wallet and commands: `tauri-plugin-klimper-partial-offers`

Built like `tauri-plugin-klimper-xchonnect`: it takes Sage's state (`Arc<Mutex<Sage>>`) and
uses Sage's wallet, coins and keychain through their public APIs. Modules:

- `make.rs`, `take.rs`, `cancel.rs`: Sage-shaped functions over `sage_wallet::Wallet`
  (`make_partial_offer`, `take_partial_offer(bundle, requested_amount, fee)`,
  `cancel_partial_offer`), so they can become `sage-wallet/src/wallet/offer/partial.rs` in a
  pull request to Sage without rewriting.
- `store.rs`: the partial offers the user made or took (kind, latest coin id, remaining, fills),
  in a table of its own, `klimper_partial_offers`, created with `CREATE TABLE IF NOT EXISTS` in
  the wallet's database. **Not** a numbered file in `migrations/`: Sage numbers its own, and a
  `0006` of ours would collide with upstream's next one and break every wallet database on the
  merge.
- `sync.rs`: follow the chain of partial coins for the offers in the store. A child with the
  same puzzle hash is a fill with a rest, a settlement child is the last fill, a claw back is a
  cancel; the expiry also counts. `ClawbackV2` (`sage-wallet/src/child_kind.rs:16`, also
  P2_1_OF_N, also hinted) is the model. Runs on the plugin's own timer, not in Sage's queues.
- `commands.rs`: `view_partial_offer`, `take_partial_offer`, `make_partial_offer`,
  `cancel_partial_offer`, `get_partial_offers`. Records (`PartialOfferSummary`: initial and
  remaining amounts, price, min fill, required fee, expiry, maker, coin id, revocable, status
  active / partly filled / filled / cancelled / expired, fills) are the plugin's own types,
  named after nodexch's `partial` fields so the UI reads both the same way.
- Permissions in the plugin's own `permissions/`; bindings generated for the plugin, not into
  `src/bindings.ts`.

Shared files touched: one `.plugin(...)` line in `src-tauri/src/lib.rs`, the workspace member in
`Cargo.toml`; plus `klimper-partial-offers:default` in `src-tauri/capabilities/klimper.json`,
which is ours. Sage's `view_offer`, `import_offer` and `get_offers` stay as they are: a
`partial1…` never reaches them, the UI routes it first.

Swap-out: when Sage has endpoints, drop the plugin, port the store rows into Sage's offers
(a one-time import of the `klimper_partial_offers` table), and point `api.ts` at Sage's commands.

### Xchonnect: Klimper-only (`tauri-plugin-klimper-xchonnect`)

All in a new `execute/partial_offer.rs`, using part 1:

- `execute.rs:586` `decode_offer_bundle` takes only `offer1`. `check_offer` (:607) refuses a
  partial offer: wallet-kit's `simulate` marks the P2_1_OF_N coin `SpendKind::Unknown`, and the
  hint spend is not a placeholder by its parent. Recognise the partial coin in a pre-check.
- `chia_takePartialOffer {offer, requestedAmount, fee}`; add it to `WALLET_BUILT_METHODS` (:52)
  and raise `METHODS_REVISION` (:66). A `partial` flag on create (`execute/create_offer.rs`).
  No cancel method yet.
- The limits (`AssetLimit`, `DailySpend`) apply to the fill `x`, not the whole offer. The price
  shown on approval is `initial_requested / initial_offered`.

### In-app bridge (`crates/sage-apps`, `packages/sage-app-sdk`)

Not ours to add: there is no offer method in the bridge today (only derivations,
`sign_coin_spends`, `send_transaction`, `send_xch`, coins and balances). Pengui can build a fill
with those, as the reference CLI does. Offer methods for apps wait for Sage.

### 3. UI: `src/klimper/partial/`

- `api.ts`: the only file that names the plugin's commands (`view`, `take`, `make`, `cancel`,
  `list`). Every screen calls it. When Sage ships endpoints, this file changes and the screens
  do not.
- `isPartialOffer.ts`: the one `partial1` test, used by every entry point below.
- Screens (below) as Klimper routes appended in `src/klimper/routes.tsx`
  (`/offers/partial/view/:offer`, `/offers/partial/mine/:id`), so Sage's `Offer.tsx`,
  `SavedOffer.tsx`, `OfferRowCard.tsx` and `OfferSummaryCard.tsx` stay untouched.
- Flag: `partialOffersEnabled = VITE_ENABLE_PARTIAL_OFFERS === 'true'` in `src/lib/features.ts`
  (like `gnosisPayEnabled`), declared in `src/vite-env.d.ts`, off in release builds. Off, the
  routes redirect through `GATED` and the entry points below do nothing.
- Entry points, one line each in shared files: accept `partial1` in `src/klimper/scanTarget.ts:47`
  (ours), `src/klimper/Scanner.tsx:90` (ours), `src/pages/Offers.tsx` paste/NFC/QR (route to the
  partial view), `src/pages/QrScanner.tsx:35`, `src/lib/offerData.ts:29` (links).
- Make: one element, `<PartialFillsOption />`, in `MakeOffer.tsx`; it shows only for exactly one
  asset for one asset and, when on, sends the make through `api.ts`.
- My offers: one element, `<MyPartialOffers />`, in `Offers.tsx`.
- Strings in `src/locales/*/messages.po` (en-US, de-DE, es-MX, zh-CN).
- Each shared file touched gets its row in `docs/UPSTREAM.md` and the `Modified by Klimper`
  line.

### Order

1. Driver, with tests (no app change; can go to chia-wallet-sdk early).
2. View: plugin `view_partial_offer`, the view screen, the entry points. Reaches AC #1.
3. Take with an amount (AC #2), then Xchonnect's take.
4. Make with partial fills (AC #3), then store and sync, my offers, cancel (AC #4).

## Screens

- **View:**
  - A "Partial offer" badge.
  - Remaining of initial for both assets, with a filled bar.
  - The fixed price both ways.
  - The min fill in both assets; the max is what is left.
  - The fee per fill, apart from the network fee.
  - The expiry, as a date and a countdown.
  - The maker address and coin id.
  - A revocable-CAT warning, and the status (active, partly filled, filled, cancelled, expired).
- **Take:**
  - A slider and an amount field from the min to what is left, with Min, 25 %, 50 % and Max.
  - A live quote: you pay `x` + fee per fill + network fee, you receive `y` (rounded down).
  - Check the coin again right before signing. If someone filled it first, say "offer changed"
    and quote again against the newest coin.
- **Make:** an "Allow partial fills" toggle on Make offer, only for exactly one asset for one
  asset (XCH or CAT). Fields: min fill, fee per fill (optional), expiry. Output: `partial1…` with
  copy, QR and post to nodexch.
- **My offers:**
  - "Filled 3× · 42 %" and what is left.
  - The fills from the coin chain.
  - "Cancel (claw back)": works at any time, costs a network fee, and returns the rest.

## nodexch fields

nodexch's offer answers, book entries and `offer` event carry `kind: "partial"` and `partial`
{`initial_offered`, `initial_requested`, `remaining_offered`, `remaining_requested`, `filled`,
`price`, `min_fill_requested`, `min_fill_offered`, `required_fee_xch`, `expires_at_ms`,
`maker_puzzle_hash`, `coin_id`, `first`, `revocable`}, in units. A wallet that reads offers from
nodexch can show the view screen from these alone.
