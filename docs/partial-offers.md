# Partial offers (TASK-164)

Partial offers (CHIP-0052, Final) sell one asset for another at a fixed price, and takers fill
them in parts. The text is bech32m with the prefix `partial` (`partial1…`), compressed like an
offer. Splash relays them since 0.3.0 (2026-10-23). nodexch decodes and lists them, and after each
fill it rebuilds the next offer with what is left (nodexch TASK-231). Upstream Sage does not
handle them (no branch or PR), nor does chia-wallet-sdk (no partial driver); Klimper does, behind
`VITE_ENABLE_PARTIAL_OFFERS`, in three parts made to be replaced by upstream's (below). The
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

A plain library with no Sage types (Apache-2.0, like chia-wallet-sdk), shaped so it can be
proposed to the SDK as it is. Ported from the reference (MIT, `NOTICE.md`), with nodexch's
stricter decoding (`indexer/src/offers/partial.rs`). It enables chia-wallet-sdk's `action-layer`
feature for the CAT makers.

- `PartialOffer`: `from_text` / `to_text`, `from_spend_bundle` / `to_spend_bundle`,
  `accept_offer` (a fill from the taker's ordinary offer), `child` (the next offer after a
  fill), `follow` (what a spend of the coin did: a fill with the rest, or a claw back),
  `claw_back`.
- `PartialOfferInfo`: the terms and the puzzle hashes. `from_hint` refuses a hint whose
  conditions are anything but an expiry and a fee (the reference hashes those two alone, so
  anything else would point at a coin that is not there).
- `make_partial_offer(one_sided_offer, terms, agg_sig)`: turns the wallet's signed one-sided
  offer (the offered asset plus one mojo of XCH, for nothing) into the first partial coin and a
  security coin, as the reference CLI does, with one difference that matters: the security
  coin's spend asserts each settlement coin's announcement of the payments into the partial
  coin. The reference spends it to a bare remark, which lets anyone who reads a first partial
  offer pay the maker's settlement coins to themselves before the first fill (found in review,
  2026-10-26; worth reporting to the reference and to whoever ports it).
- `encode_partial_offer` / `decode_partial_offer` / `is_partial_offer`.

Tests (`tests/simulator.rs`, chia-wallet-sdk's simulator): every pairing of XCH, a CAT and a
revocable CAT, with and without a required fee and an expiry, made from a one-sided offer, read
back from its text and filled in two parts, each fill followed from the chain; claw back of a
CAT and a revocable CAT; a claw back with another key fails; the maker's coins cannot be taken
before the first fill (and could with the reference's remark); a fill below the minimum or above
what is left is refused; an expired offer cannot be filled.

Swap-out: when chia-wallet-sdk has a partial driver, delete the crate and change the imports in
the plugin; nothing else names it.

### 2. Wallet and commands: `tauri-plugin-klimper-partial-offers`

Built like `tauri-plugin-klimper-xchonnect`: it takes Sage's state (`Arc<Mutex<Sage>>`) and
uses Sage's wallet, keychain, peer and sync through their public APIs. Sage's crates are not
changed.

- `wallet.rs`: `follow` (an offer along the chain to its newest coin, through every fill, to
  the last fill or the claw back; before the first fill, a spent input means cancelled; the
  expiry counts), `view`, `take` (checks the chain right before signing: an offer someone filled
  first is `offer_changed`, and nothing is signed), `terms` and `make` (the maker is the wallet's
  change puzzle, refused unless standard), `cancel` (before the first fill by spending one of the
  coins it is made from, after it by claw back, with a network fee). Written over
  `sage_wallet::Wallet` so they can become `sage-wallet/src/wallet/offer/partial.rs` in a pull
  request to Sage.
- `store.rs`, `mine.rs`: the wallet's own partial offers, in
  `wallets/<fingerprint>/<network>-klimper-partial-offers.json` next to Sage's wallet database.
  **Not** a table there: Sage's pool is private, and a numbered file in `migrations/` would
  collide with upstream's next migration and break every wallet database on the merge. Listing
  follows each live offer and keeps its newest text and fills.
- `lib.rs`: the commands `view_partial_offer`, `take_partial_offer`, `make_partial_offer`,
  `cancel_partial_offer`, `get_partial_offers`, `delete_partial_offer`. Errors come as
  `{code, message}`. Records are named after nodexch's `partial` fields; amounts are mojos as
  strings.
- Making does not import the one-sided offer into Sage: Sage would list it as an ordinary offer
  of the asset for nothing, and anyone given that text could take it. So its coins are not
  locked; if the wallet spends one before the first fill, the partial offer reads as
  cancelled and nothing is lost.

Tests (`src/tests.rs`): two Sage wallets on a simulated chain (`src/sage_sim.rs`, copied from
the Xchonnect plugin): CAT for XCH filled in two parts (the second with a network fee),
followed from the first text and listed; XCH for a CAT filled once and clawed back with a fee;
a cancel before any fill; a required fee on top; expiry; a stale text refused; only the maker
cancels.

Shared files touched: one `.plugin(...)` line in `src-tauri/src/lib.rs`, the dependency and the
workspace member; `klimper-partial-offers:default` in `src-tauri/capabilities/klimper.json`,
which is ours. Sage's `view_offer`, `import_offer` and `get_offers` stay as they are: a
`partial1…` never reaches them, the router sends it to the partial page first.

Swap-out: when Sage has endpoints, drop the plugin, import the JSON file's offers into Sage's
offers once, and point `api.ts` at Sage's commands.

### Xchonnect: Klimper-only (`tauri-plugin-klimper-xchonnect`), not done yet

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

- `api.ts`: the only file that names the plugin's commands. When Sage ships endpoints, this file
  changes and the screens do not.
- `isPartialOffer.ts`: the one `partial1` test. `math.ts`: the price as the puzzle computes it.
- `routes.tsx`, called from Klimper's router hook: `/offers/view/:offer` shows a `partial1…` on
  `PartialOfferPage` and anything else on Sage's page. Every way Sage opens an offer (paste, NFC,
  its QR scanner, dexie and offerco.de links) and the Scan tab end on that route, so Sage's
  `Offers.tsx` paste and scan code, `QrScanner.tsx` and `offerData.ts` need no change. It also
  adds `/offers/make-partial`. The pages load lazily.
- Flag: `partialOffersEnabled` (needs `offersEnabled` and `VITE_ENABLE_PARTIAL_OFFERS=true`),
  off in `.env.production` and `.env.desktop` (tested in `releaseFlags.test.ts`). Off, the routes
  are Sage's and the two elements render nothing.
- Make: `<PartialFillsOption />` in Sage's `MakeOffer.tsx` opens `MakePartialOffer`, which takes
  the assets, amounts and expiry from Sage's offer state (exactly one token for one token) and
  adds the least per fill and the fee per fill.
- My offers: Sage's `Offers.tsx` shows two groups, "Offers" and "Partial offers"
  (`<MyPartialOffers />`, with `useMyPartialOffers`), and Sage's status filter applies to both
  (`filter.ts`: active is open, partly filled, cancel pending or unknown; completed is filled).
- Cancel feedback: `cancel_partial_offer` notes the cancel in the store (transaction id and the
  partial coin it spends). Until the chain shows it, the list and the offer's page say
  `cancelling` ("Cancel pending", with the transaction id), and the list reloads on the
  wallet's coin events and every 10 s. Confirmed: "cancelled", with a toast. If a taker's
  fill confirms first (the offer has moved past that coin), the note is dropped and the offer
  is live again, with a toast. After 10 minutes still pending, "Cancel again" is offered.
- No QR code for a partial offer: its text is several kilobytes, more than a QR code holds.
  Copy it, or post it to nodexch or Splash.
- Strings extracted into `src/locales/*/messages.po` (English only so far, like the rest of
  Klimper's own strings).

### Status (2026-10-26)

Done: driver, plugin, screens, flag, simulator tests (driver and two Sage wallets), a code review
with its fixes (among them the security coin, above), and a run on testnet11
(`examples/testnet.rs`, two fresh Sage wallets): a test CAT issued; a partial offer of 1.000
of it for 0.0001 TXCH made; viewed by the taker; filled a quarter, the next offer as built
matching the coin on chain; the stale first text refused before signing; filled another
quarter; listed by the maker with two fills; the rest clawed back; balances taker +500,
maker −500 CAT mojos. AC #1–#5 are met.

Open: tapping through the screens in the running app; Xchonnect's take; posting a made offer
to nodexch from the make screen; nodexch's MCP `decode_offer` accepts only `offer1…`, so a
`partial1…` cannot be cross-checked there yet (its indexer reads them).

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
- **Make:** "Allow partial fills" on Make offer, only for exactly one asset for one asset (XCH
  or CAT), opens the partial make screen. Fields: min fill, fee per fill (optional); the expiry
  is Make offer's. Output: `partial1…` to copy.
- **My offers:**
  - A group of their own under the Offers list, under the same filter.
  - "Filled 3× · 42 %" and what is left.
  - The fills from the coin chain.
  - "Cancel (claw back)": works at any time, costs a network fee, and returns the rest.
  - After a cancel: "Cancel pending" until it is confirmed, then "Cancelled".

## nodexch fields

nodexch's offer answers, book entries and `offer` event carry `kind: "partial"` and `partial`
{`initial_offered`, `initial_requested`, `remaining_offered`, `remaining_requested`, `filled`,
`price`, `min_fill_requested`, `min_fill_offered`, `required_fee_xch`, `expires_at_ms`,
`maker_puzzle_hash`, `coin_id`, `first`, `revocable`}, in units. A wallet that reads offers from
nodexch can show the view screen from these alone.
