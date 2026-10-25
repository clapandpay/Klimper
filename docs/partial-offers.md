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

## Work list

### Engine (Rust): upstream-shaped, propose to Sage

1. `crates/sage-wallet/src/wallet/offer/partial.rs` (new): port `PartialOffer` and
   `PartialOfferInfo` from the reference (`from_spend_bundle`, `to_spend_bundle`,
   `quote`/`reverse_quote`, `accept_offer`, `child`, `claw_back`) and add the puzzle. Klimper uses
   chia-wallet-sdk 0.36.0 without `action-layer`, so either enable that feature (`CatMaker`) or
   curry the CAT makers by hand. nodexch's `indexer/src/offers/partial.rs` derives the puzzle hash
   from the `chia-sdk-types` action-layer args alone, and tests it against the curried puzzle.
2. Encode and decode with the `partial` prefix. The SDK only knows `offer`
   (`driver/offers/compress.rs`).
3. `offer/make_offer.rs:42` → `make_partial_offer`: a one-sided offer of the offered asset that
   pays into the partial coin with the memo, plus the hint spend. The reference refuses a
   non-standard maker puzzle.
4. `offer/take_offer.rs:21` → `take_partial_offer(bundle, requested_amount, fee)`: the taker's
   side for `x` + the required fee, then `accept_offer`.
5. `offer/cancel_offer.rs:6`: claw back through the maker leaf.
6. `crates/sage/src/endpoints/offers.rs`: decode by prefix (the decode calls are at :202, :270,
   :286, :715, :738); extend `view_offer` and `import_offer`; add `view_partial_offer`,
   `take_partial_offer` (with an amount), `make_partial_offer` and `cancel_partial_offer`.
   Register each in `crates/sage-api/src/requests/offers.rs`, `crates/sage-api/endpoints.json`,
   `src-tauri/src/lib.rs` (commands), `src-tauri/permissions/main-host.toml`, then regenerate
   `src/bindings.ts`.
7. Records: `OfferSummary.partial` (initial and remaining amounts, price, min fill, required fee,
   expiry, maker, coin id, revocable) in `crates/sage-api/src/records/offer_summary.rs`, filled by
   `crates/sage/src/utils/offer_summary.rs` `summarize_offer`. Add a status `PartiallyFilled`, or
   a fills count, in `records/offer.rs`.
8. A migration `migrations/0006_partial_offers.sql` (kind, latest coin id, remaining, fills),
   with methods in `crates/sage-database/src/tables/offers.rs`.
9. Status sync, `crates/sage-wallet/src/queues/offer_queue.rs:64`: follow the chain of partial
   coins. A child with the same puzzle hash is a fill with a rest, a settlement child is the last
   fill, a claw back is a cancel; the expiry also counts. `ClawbackV2` (`child_kind.rs:16`, also
   P2_1_OF_N, also hinted) is the closest model.

### In-app bridge: upstream only (`crates/sage-apps`, `packages/sage-app-sdk`)

There is no offer method today: an app only has derivations, `sign_coin_spends`,
`send_transaction`, `send_xch`, coins and balances. Pengui could build a fill itself with those, as
the reference CLI does. View, take and make methods for offers would have to come from Sage.

### Xchonnect: Klimper-only (`tauri-plugin-klimper-xchonnect`)

- `execute.rs:586` `decode_offer_bundle` takes only `offer1`. `check_offer` (:607) refuses a
  partial offer: wallet-kit's `simulate` marks the P2_1_OF_N coin `SpendKind::Unknown`, and the hint
  spend is not a placeholder by its parent. Teach the check to recognise the partial coin, in
  xchonnect-wallet-kit or in a pre-check here.
- Add `chia_takePartialOffer {offer, requestedAmount, fee}` (or an amount on `chia_takeOffer`,
  :829). Add it to `WALLET_BUILT_METHODS` (:52) and raise `METHODS_REVISION` (:66).
- Add a `partial` flag on create (`execute/create_offer.rs`).
- There is no cancel method yet.
- The limits (`AssetLimit`, `DailySpend`) apply to the fill `x`, not the whole offer. The price
  shown on approval is `initial_requested / initial_offered`.

### UI

- Flag: `partialOffersEnabled = VITE_ENABLE_PARTIAL_OFFERS === 'true'` in `src/lib/features.ts`
  (like `gnosisPayEnabled`), declared in `src/vite-env.d.ts`, off in release builds.
- Accept `partial1` in `src/klimper/scanTarget.ts:47` (Klimper's), `src/klimper/Scanner.tsx:90`,
  `src/pages/Offers.tsx:73-170` (paste, NFC, QR), `src/pages/QrScanner.tsx:35` and
  `src/lib/offerData.ts:29` (links).
- Shared pages go through a new `PartialOffer` component that `Offer.tsx` (:55 view, :109 take),
  `MakeOffer.tsx`, `OfferRowCard.tsx`, `OfferSummaryCard.tsx` and `SavedOffer.tsx` only route to.
  Upstream that hiding point.
- Strings in `src/locales/*/messages.po` (en-US, de-DE, es-MX, zh-CN).

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
