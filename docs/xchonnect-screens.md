# Xchonnect screens

The design pass for the Xchonnect screens (clapandpay **TASK-069**), and the contract the
screens that implement it keep (TASK-072, TASK-080, TASK-083, TASK-084, TASK-085).

Pairing is where phishing happens and the approval screen is where money leaves. These
screens are the security surface of the feature, so this document is prescriptive about
what is shown, in what words, and what the user is **not** offered.

Normative behaviour is xchonnect's `docs/spec/xchonnect-spec.md`; what a wallet owes the
user is its `docs/wallet-integration.md` (§1, §5, §6 and the MUST checklist). The typed
command surface is `src/klimper/xchonnect/xchonnect.ts`.

## The four rules nothing may drop

1. **Both domain forms when they differ, and every warning.** `domainUnicode` is what the
   user reads; `domainAscii` is what the computer resolved. When `domainDiffers`, both are
   on screen, and every entry of `warnings` has its own line with its own plain-language
   sentence.
2. **No continue-anyway on a failure.** A failed origin verification, a used pairing code
   or an expired one ends the flow. The only controls are "Start over" and "Cancel". There
   is no control that proceeds, not behind a confirmation and not behind a details
   disclosure.
3. **Only the wallet's own simulation is rendered.** `prompt.summary` and `prompt.plan`
   come from local CLVM execution. A dApp's claims about amounts, recipients or purpose
   are not evidence and are not rendered — the one exception is the message of a
   `signMessage` request, which _is_ the object being signed and is shown inside a labelled
   quoted block, never as an amount or a recipient.
4. **One decision per request**. Approving answers the whole request:
   every signature it needs is made after one unlock, and a request Klimper broadcasts
   (`submit_coin_spends`) is sent by Klimper — the button says "Approve and send".

## Design language

Klimper is deliberately cleaner than Sage: Sage's `AdvancedTransactionSummary` answers
"what does this transaction contain", and this screen answers "what happens to my money".
The second question has a short answer, and the long one goes behind a disclosure.

| Role                 | Token                                  | Used for                                                               |
| -------------------- | -------------------------------------- | ---------------------------------------------------------------------- |
| Verdict line         | `text-2xl font-semibold`               | the one number the decision turns on: the net effect per asset         |
| Loss                 | `text-destructive`                     | a negative net, a fee                                                  |
| Gain                 | `text-foreground`                      | a positive net — never `text-green`, so "safe" is never a colour claim |
| Soft                 | `text-muted-foreground`                | anything conditional, informational or claimed by the dApp             |
| Danger panel         | `border-destructive` + `AlertTriangle` | `AGG_SIG_UNSAFE`, an unknown contract, a homograph warning             |
| Verified-domain slab | `text-xl font-semibold break-all`      | the domain, always the largest text on a pairing screen                |
| SAS                  | `text-5xl font-mono tracking-[0.3em]`  | the six digits                                                         |
| Claimed text         | `text-muted-foreground` + "it claims"  | `dappName`, the `signMessage` body                                     |

**Short on screen, the why behind a dot.** Every screen keeps one short line per fact. The
sentence that explains it (why a domain counts as verified, what "simulated" means, what
happens when the countdown ends) sits behind an info dot (`Hint`, `src/klimper/Hint.tsx`), which
opens on a tap as well as on hover, since a phone has no hover. What is never moved behind a
dot: a danger panel and its sentence, a homograph warning, the dead-end sentence, "only the
domain is verified" next to "Domain verified", "if they are different, somebody is in the
middle" under the six digits, what Klimper checked about a trade, "these are the only payments
to anyone else", and the "only if the counterparty completes" line. Those stay on screen.

Both themes come from the existing theme tokens rather than literal colours, so light and
dark follow the app (`theme-o-rama`); `text-destructive` and `border-destructive` are the
only semantic colour used, and every danger panel also carries an icon and a word, so the
warning survives a colour-blind reader and a monochrome screenshot.

TASK-004's tokens do not exist yet (that task is open), and TASK-062's Klimper theme is in
progress. These screens therefore use Sage's existing token names, which both of those
tasks redefine rather than rename; a theme change moves these screens with it. No literal
hex value appears in any Xchonnect screen.

## Screens and states

### 1. Connected dApps — `/xchonnect` (TASK-084)

| State                 | What is shown                                                                                                                                         |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| empty                 | "No dApps are connected." plus what a connection is and the one button that starts one                                                                |
| a session             | verified domain (both forms when they differ), claimed name as claimed, when it was connected, key epoch, listening or not, methods, limits, switches |
| `needsRepair`         | the card is marked unusable, with "Connect again" — no action that would use it                                                                       |
| `revoked`             | the website withdrew its origin key: "Klimper ended this connection", why, and only **Remove**                                                        |
| `rotating`            | the keys read "being renewed" and **Renew keys** is disabled until the website has answered                                                           |
| `warnings`            | a danger line per warning, on the card                                                                                                                |
| dangerous switches on | a warning line naming which one                                                                                                                       |
| no wallet open        | the list is empty and says the wallet has to be unlocked first                                                                                        |

Actions per session: **Permissions** (screen 5), **Renew keys** (spec 9.2.1, only while
the connection is listening) and **End connection**. Ending is immediate and needs no
confirmation dialog — it is the safe direction.

### 2. Connect a dApp — `/xchonnect/connect` (TASK-072, TASK-085)

| Stage        | What is shown                                                                                                                                                                                                                                                                                                                                   |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| entry        | a viewfinder; **Scan with the camera** (on desktop the webview's camera, read live with `jsQR` in `QrCamera.tsx` and released as soon as it reads; on mobile the native scanner), **Paste the code**, **Choose a picture**, or type it. The field accepts `xchonnect:v1?…` and `/pair#…`. No camera: a sentence, and the other three still work |
| checking     | one line, no spinner theatre: "Checking the website…"                                                                                                                                                                                                                                                                                           |
| verified     | the domain slab, warnings, what the dApp may ask for, the single address shared, the network, the relay, and "nothing is signed automatically"                                                                                                                                                                                                  |
| failed       | the reason, "there is no way to continue past this", and only Start over / Cancel                                                                                                                                                                                                                                                               |
| SAS          | six digits, large; "the website shows the same six digits"; both answers only once the dApp has answered                                                                                                                                                                                                                                        |
| waiting      | "Waiting for the website…" with the deadline                                                                                                                                                                                                                                                                                                    |
| used/expired | the same dead end as `failed`, with the copy below                                                                                                                                                                                                                                                                                              |

A link that arrives from the OS is handed over in memory, never through the app URL: the
app uses a hash router, so a fragment in the URL would both break routing and write the
pairing secret into the history stack.

Every Xchonnect screen renders inside the app's `Layout`, so the sidebar and the wallet stay one
tap away during and after pairing. A step line (Scan · Check · Compare) sits above the flow and is
text, never a control; it is not shown on the dead end. Matching digits open `/xchonnect` on the
new connection: a banner names the domain, its card is outlined in teal, and **Back to my
wallet** returns to the wallet. Shared pieces (site coin, step line, digit tiles, facts, the
listening pill) are `XchonnectUi.tsx` and `xchonnect.css`, theme tokens only.

### 3. Approval — the overlay, and `/xchonnect/request` (TASK-080)

One view of a waiting request, `XchonnectApprovalDetail`, shown in two places:

- **The overlay** (`XchonnectApprovalHost`, mounted once next to the app in `App.tsx`)
  opens it over whatever screen is open the moment the `approval` event arrives, oldest
  first, and on the desktop brings the window to the front. It also resumes every stored
  session's message loop when the app starts and comes back to the front, so requests are
  fetched without the user opening an Xchonnect screen first. **Later** hides it without
  answering: a sun pill in the corner counts down and reopens it, and a new request opens it
  again. It never shows on the requests page.
- **The requests page** (`/xchonnect/request`, `/xchonnect/request/:approvalId`) lists
  everything waiting — domain, what kind of request, the countdown, no amount — and shows the
  one picked in the same view: side by side on a wide window, list then view on a phone.
  Any request can be picked, but **Approve exists only in the full view**, so nothing is
  approved without everything below on screen. Opened by "See all requests", by a `/req`
  link, and rebuilt from `pendingApprovals()` after a reload.

```
┌──────────────────────────────────────────┐
│ (p) pengui.xyz                     Later │  verified domain only
│     [Sign a spend] ⏱ 4:31  +1 waiting ⓘ │  chip names the method on hover; red in the last minute
├──────────────────────────────────────────┤
│ [Simulated by Klimper ⓘ] [Checked twice ⓘ]│  badges, the sentence behind each dot
│ X You pay            − 1.5 XCH           │  the verdict line, per asset
│                  ≈ $18.20 · then 3.2 XCH │  dollars (XCH only) and the balance afterwards
│ C You receive   + 250 CAT 1a2b3c4d…      │  a CAT by its id, always
│   your wallet: USDC.b                    │  the wallet's ticker beside it, never in its place
│   Only if the counterparty completes     │  never added to a total
│ ➤ 1.5 XCH → xch1qz4…k9f2                 │  where it goes
│ Network fee   0.0000005 XCH   <$0.01     │
├──────────────────────────────────────────┤
│ ⚠ Unknown contract                       │  danger panel, puzzle hash in full
│ ⚠ Unsafe signature                       │  danger panel, red, the loudest thing present
│ ▸ Details                                │  signature counts and the parsed view
├──────────────────────────────────────────┤
│ ( ⟫  Slide to approve               )    │  glass track, sun knob
│ [ ✕ Reject ]              [ Withdraw ]   │
└──────────────────────────────────────────┘
```

**A trade** (TASK-139): any spend that both pays and receives (an offer take, several taken
together, a swap a website built) opens with the trade card, computed by the wallet and sent as
the prompt's `trade` (`src/trade.rs`, read by `readPrompt`, drawn by `XchonnectTrade.tsx`):

```
┌──────────────────────────────────────────┐
│ (p) pengui.space                   Later │
│     [Take 3 offers together] ⏱ 4:30  ⓘ  │  the count of offers, never an amount
├──────────────────────────────────────────┤
│ [Simulated by Klimper ⓘ] [Built by Klimper ⓘ] [Checked twice ⓘ]
│ ┌ YOU PAY ─────────────────────────────┐ │
│ │ (X) −1.5 XCH               ≈ $18.60  │ │  display face, tabular, thousands set apart
│ │  ↓                                   │ │
│ │ YOU RECEIVE                          │ │
│ │ (S) +3 000 SBX                       │ │  the wallet's ticker and icon, no dollars …
│ │     Spacebucks  CAT 5b5b5b5b…5b5b5b  │ │  … with the asset id, always
│ │     Two tokens in your wallet use    │ │  only when another CAT's ticker reads
│ │     the ticker SBX. Check the id.    │ │  the same
│ │ ──────────────────────────────────── │ │
│ │ Price  1 SBX 5b5b5b5b = 0.0005 XCH [⇄] │  ticker and id; the flip turns it round
│ │ ≋ 3 separate offers taken together — all or nothing ⓘ
│ │ Your network fee   0.0001 XCH        │ │  the user's own; the whole trade's is below
│ └──────────────────────────────────────┘ │
│ One half of a trade …                    │  the binding panel, as before
│ ▸ Each asset, as simulated               │  the per-asset verdict lines, folded
│ Network fee of the whole trade …         │
├──────────────────────────────────────────┤
│ ( ⟫  Slide to approve and send      )    │
└──────────────────────────────────────────┘
```

- **What is paid and what is received** are the wallet's figures for the bundle it signs. The
  user's fee is taken out of the XCH paid and shown on its own line; it is the `RESERVE_FEE`
  of the user's spends only as far as the user's own XCH paid for it (at most the XCH that left
  the user's coins). A fee the user's spend declares but another party's coin funds (a
  settlement coin needs no signature) is not the user's and is never counted as XCH received
  (TASK-139 audit, H1). So when Sage pays the fee of a sale out of the XCH the offer pays the
  user, the card shows the XCH that arrives, already less the fee, and no fee line. Received counts what the
  user's own spends create and what the user's spends are bound to receive from the other side
  (`binding.bound_received`): a settlement payment the user's spends assert cannot be left out
  without the user's spends failing, so it is as certain as change. Anything conditional and
  not bound is not in the card; it keeps its own line in the per-asset verdict.
- **Many assets.** A side shows its first three assets and "and N more" (a side one over shows
  in full); the button expands and folds again. The binding panel folds a list of more than
  three payments under "Depends on N payments to you". The card never pushes the slider away:
  the body scrolls, the actions stay. Nothing is cut at 320px; a long amount wraps.
- **Names and icons** come from the wallet's own records (`getToken`, `getCats`), never from
  the website. A CAT's id (`CAT 1a2b3c4d…9f8e7d`) is under every CAT, ticker or not, and a CAT
  the wallet has never held says "new to your wallet". (Audit of TASK-139, M2 and L3.)
  - A ticker is used only when it is plain ASCII (`A–Z a–z 0–9 . $ _ -`, at most 12) after
    NFKC, and its skeleton (look-alike Cyrillic and Greek letters and digits mapped to the
    Latin letter they imitate, separators dropped, upper case; `ticker.ts`) holds neither
    `XCH` nor `CHIA`: "ХСН", "ΧCΗ", "ＸＣＨ", "XCH.", "X-CH", "wXCH", "CH1A" are refused.
    A refused ticker shows "CAT" as the unit and loses its icon too: a monogram of the id.
  - A CAT's **name** gets the same check (TASK-140, from the TASK-139 audit; `safeName`): a
    CAT named "Totally XCH", "Chia", "Chia Network" or "Сhia" (Cyrillic С) reads as the
    network's coin, so the card says "Unnamed token" in its place, its id beside it as always.
    The per-asset verdict shows the wallet's ticker only when it passes the ticker check.
  - When another CAT the wallet knows has a ticker with the same skeleton, the leg says "Two
    tokens in your wallet use the ticker SBX. Check the id."
  - A CAT's icon is loaded only for a CAT the wallet holds and shows (its token list loads
    that icon anyway) and only from `https:`; XCH is always the screen's own blue coin. So
    an approval never asks an icon host about a CAT the user has never had: that request
    would tell it the user's IP address and the trade before the user decided. Otherwise a
    monogram coin in one of six warm Klimper colours chosen by its id.
- **Dollars** for XCH only, from the last price fetch. A CAT shows none (M1): its last price
  on a market is whatever its maker last traded it at, with themselves if they like.
- **Price** only for one asset for one asset: a CAT priced in XCH, otherwise what is received
  priced in what is paid; six significant digits, cut, never rounded up. A CAT's unit in the
  price always carries the start of its id (`SBX 5b5b5b5b`, or `CAT 5b5b5b5b`).
- **Offers**: "N separate offers taken together — all or nothing" when a take holds more than
  one offer, counted by the wallet from the offer itself (L1): the smaller of the distinct
  nonces of its requested payments and the groups of maker coins not tied to each other by an
  announcement, a concurrent-spend assertion or a shared puzzle hash. A maker chooses nonces
  freely, so one offer notarized five times still counts as one. One transaction, so either
  every offer goes through or none does.
- Danger panels (unknown contract, unsafe signature) stay above the card; the binding panel,
  destinations, the whole trade's fee, the intent and time locks follow it unchanged.

- **Slide to approve.** Approving is a slide, not a tap (`SlideToApprove.tsx`): the knob has
  to travel nearly the whole track, a short or abandoned slide springs back, and no tap, long
  press or slow click on the knob approves. One finger at a time, and a slide counts only for
  the request it began on. The phone ticks at the half and at the point where letting go
  approves, and confirms with a success haptic (`feel` in the jingle plugin; the phone's
  haptics setting decides, not the Klimper sound switch). The knob answers pointers only.
  Keyboards and assistive technology get a separate button named "Approve" (or "Approve and
  send"), out of sight until it has focus, which approves at once since pressing it is
  deliberate. The drag never re-renders React: it sets one CSS custom property, the knob
  moves by transform and the trail by clip.
- **The deadline holds.** When the countdown reaches zero the slider turns off, and the
  approval bridge refuses an answer after the request's own deadline: a website that gave
  up on a request (and may have sent it again) never has the old one signed late.
- **What the wallet knows about each asset.** "then …" is the wallet's balance plus the net
  (`getToken`). XCH also shows its dollar value from the last price fetch (no request of its
  own; none on testnet). A CAT stays named `CAT <8 hex>…` in the verdict line: its ticker was
  registered by whoever made it, and a CAT can be called "XCH", so the wallet's ticker appears
  only beside it, small ("your wallet: USDC.b"), and a CAT gets no picture and no price here,
  only a lettered coin; one the wallet holds none of says "not in your wallet". The id shows
  both ends (`CAT 1a2b3c4d…9f8e7d`), since matching the first 32 bits is cheap. A
  CAT whose id did not parse gets none of this. None of it comes from the website. (This is
  the per-asset verdict; the trade card above shows the wallet's ticker and icon for a CAT
  when they pass its checks, with its id beside them, and no dollars, as described there.)
- **Approve** on a prompt that carries an unknown puzzle opens the second confirmation,
  which shows the puzzle hash and has its own Approve. Going back puts the slider back.
- **Doing nothing is a rejection.** The plugin treats five minutes of silence as one;
  "Later" and navigating away only postpone within that time, and the countdown says
  when it runs out.
- A `submit_coin_spends` prompt also says who built the transaction (Klimper from the
  wallet's own coins, or the website), that Klimper sends it, "checked twice" when the
  wallet database and the simulation agreed, and the website's declared intent as verified
  facts only: its `kind` label as "described by the website", each claim with a tick.

**An option contract** (TASK-140): when `summary.options` has an entry, the option card opens
the screen (`XchonnectOption.tsx`), in the trade card's panel, one card per option, every
figure from wallet-kit's own reading of the spends (spec 11.1.1). Amounts name a CAT by the
wallet's ticker with the start of its id (`500 wUSDC.b 3c3c3c3c`), or `CAT 3c3c3c3c`; a few
mojos read as "1 mojo". Dates are the phone's local time, `13 Nov 14:00`.

| Effect                               | Title        | Lines                                                                                                                                                                                                |
| ------------------------------------ | ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `mint`, the user writes and funds it | Write option | "Lock 100 XCH until 13 Nov 14:00" · "Whoever holds it can buy it for 500 wUSDC.b 3c3c3c3c, paid to you" · "After that you can claim it back" · "You hold the option" (or "The option goes to xch1…") |
| `mint`, someone else's               | New option   | "100 XCH locked until …" · "… paid to xch1…" · "After that xch1… can claim it back"                                                                                                                  |
| `mint`, `guaranteed: false`          | —            | a warning: "The new option is only conditional: spends you do not sign decide whether it is made."                                                                                                   |
| `exercise` by the user               | Use option   | "Pay 500 wUSDC.b 3c3c3c3c, receive 100 XCH" · "Paid to the option's writer, xch1…"; the binding panel says "Tied to what you receive"                                                                |
| `clawback` by the user               | Claim back   | "Claim back 100 XCH (option expired 13 Nov)"                                                                                                                                                         |
| `transfer` from the user             | Send option  | "Option 9c1f9c1f…1f9c1f to xch1…"; its terms are not part of a transfer and are not shown                                                                                                            |

Every card ends with the option's id (`Option 9c1f9c1f…1f9c1f`; a clawback that does not name
its option says "not named by this request"). Headlines: "Write an option" ("Write N
options"), "Use an option" ("Use N options"), "Claim back", "Send an option", "New option".

**Locked is not paid.** An output with `option.role == "collateral"` is listed apart, on a
saffron panel: "100 XCH locked under Option 9c1f…"; the launcher's mojo "becomes Option …".
Neither has an address and neither is counted as sending to others. With an option card on
top, the per-asset verdict folds under "Each asset, as simulated", as for a trade: its "You pay
100 XCH" would read the locked collateral as a payment.

**A loan** (Klimper's own `loan` key, with the one mint it is made of):

- Borrower (`chia_mintOption` with a lender): title "Request a loan"; "Borrow 300 wUSDC.b
  3c3c3c3c from xch1…ab"; "Lock 100 XCH until 13 Nov 14:00" · "Repay 315 wUSDC.b 3c3c3c3c to
  get it back" · "Otherwise the lender keeps it"; "1 mojo to the lender, to tie the loan
  together" (the mojo the request offers whoever takes it, so the lender's payment is bound to
  the borrower's lock); and, since Klimper signs an offer and sends nothing (`makes_offer`):
  "Klimper signs a loan request. Nothing moves until a lender takes it." The badge says
  "Signed by Klimper, not sent", the slider "Slide to approve", and the tie's output goes to
  "whoever takes the request" instead of the settlement puzzle's address.
- Lender (`chia_takeOffer` of a loan request): title "Lend"; "Lend 300 wUSDC.b 3c3c3c3c to
  xch1…cd"; "They lock 100 XCH until …" · "If they repay 315 wUSDC.b 3c3c3c3c by then, they get
  it back" · "Otherwise you can claim it". The loan card replaces the trade card.

**An offer the wallet makes** (`chia_createOffer`, TASK-142, Klimper's own `offer` key):
title "Make an offer"; "You give": per option "Option 9c1f…: whoever takes the offer may pay
315 wUSDC.b 3c3c3c3c to xch1…ab for 100 XCH until 13 Nov 14:00. After that, xch1…ab takes the
100 XCH back.", then each other asset; "You ask for": each asset; "Klimper signs an offer.
Nothing moves until someone takes it. Nobody can take it from 13 Nov 14:00, when the option
expires." The badge says "Signed by Klimper, not sent"; the offer card replaces the trade card.

A verified intent kind (`intent.kind_verified`) shows as "option.mint — verified by your
wallet" with a tick, instead of "The website says".

Preview: `?option=mint|exercise|claim|send|loan|lend` on the approvals preview.

### 4. Lock screen and notifications

No amount, no address, no asset, no domain in Unicode form. The whole copy is:

> **Klimper** — A connected website is asking you to sign something. Open Klimper to see
> what it is.

That is also what an encrypted notification preview may say (TASK-088); a preview that is
missing, malformed, unauthenticated or expired produces exactly this text, so the user
cannot tell the difference and no preview can ever drive a decision.

Checked 2026-10-08 (TASK-123): the gateway's alert is a pure function of its own
configuration (`XCHONNECT_GATEWAY_APNS_ALERT_*`, default "Signing request / Open your wallet
to review it."), never of the request, the mailbox or the device; nothing in our deployment
overrides it. An encrypted preview rides along as a custom key that only a Notification
Service Extension could open, and Klimper has none, so nothing a website sends reaches the
lock screen. Klimper keeps the preview key with the registration for a future extension.

### 5. Permissions and limits — `/xchonnect/dapp/:sessionId` (TASK-083)

Allowed methods (as checkboxes over the methods the session was granted), the exposed key,
a per-request and a per-day limit per asset (XCH and every token the wallet holds), and the
two dangerous switches (unsafe signatures, unreadable contracts). Defaults: the CHIP-0002
required methods, the receive addresses plus one key of the website's own, no
auto-approval, both dangerous switches off, no limits.

**What a website sees** (TASK-120). Pairing has one switch, **Can see my receive addresses**,
**on by default** (owner, 2026-10-08): a website such as Pengui then sees the user's receive
addresses with their coins and can show balances and build transfers and offers from them.
Every request still needs the user's approval. The same switch is in the website's settings,
without a confirmation dialog; changing it re-sends `session.permissions`.

Either way, when the six digits match the website is also given one of the wallet's
**hardened** keys as its own address: the first one no website has ever had (a per-wallet
list, `websites:<fingerprint>` in the keyring, outlives every connection, and the claim is
made under the store's write lock). Sage never hands out a hardened key as a receive or change
address, and coins paid to one do not move Sage's next receive address, so a website's key
never overlaps the user's addresses or another website's, and the receive addresses never
include another website's key. With the switch **off**, that own address is all the website
sees: it cannot link the user's addresses through Klimper, and a message it asks to have
signed is signed by a key no other website knows. Requests a website builds itself
(`signCoinSpends`, `submitCoinSpends`, `signMessage`) are signed with the keys it was given
only, so a guessed key of the wallet's answers like a stranger's (TASK-122); `chia_send` and
`chia_takeOffer`, which Klimper builds from its own coins, use the whole wallet. A payment the wallet makes on
chain still shows its input coins and change address to anyone who looks, and a page that
sees the addresses asks some chain service about them; Klimper cannot hide either.

**Kept per website** (TASK-121). Its own key, its limits and its daily totals are stored per
wallet and verified domain, not per connection: removing a connection and pairing again
brings the same address and limits back and does not reset the day's budget. A day is a UTC
day. After three sends it asked for could not be built within ten minutes, a website's
further sends are refused without building, so the refusals stop telling it the balance.

**Option contracts** (TASK-140). Pairing has a second switch, **Option contracts**, **off by
default**: "Let this website ask to write, use, claim back or send option contracts, and to
request or take loans. Each one still asks you first; they lock funds until a date." On, the
website is granted `chia_mintOption`, `chia_exerciseOption`, `chia_claimOption` and
`chia_transferOption` (`confirm_sas` `optionContracts`). The same switch is in the website's
settings: on adds the four methods, off removes them, every other method kept
(`set_dapp_policy` `methods`); it shows on only when all four are granted, so a connection
made before TASK-140 shows it off until the user turns it on. No dialog: every request still
asks.

Each dangerous switch opens its own confirmation dialog, which names the consequence in
one sentence and requires the explicit button; cancelling leaves the switch off. Turning
one **off** is immediate.

A request refused by a limit never reaches a prompt: wallet-kit refuses it and the dApp
gets the error. The screen says that in the limits section, so a user who wonders why
nothing appeared can find the reason.

## Rendering table

Every field the view may read, and nothing else. `src/klimper/xchonnect/xchonnectPrompt.ts` reads
exactly this list by explicit key and validated type; any other key in the prompt JSON —
including anything a dApp managed to influence — is dropped before the view sees it.

| Prompt field                                                                                  | Shown as                                                                                                                                                                                                                                                                                                                        | Copy                                                      |
| --------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------- |
| `summary.assets[].asset`                                                                      | asset name: `XCH`, or `CAT <first 8 hex of asset_id>…`; never a dApp-supplied name                                                                                                                                                                                                                                              | —                                                         |
| `summary.assets[].net` < 0                                                                    | the verdict line, `text-destructive`, absolute value                                                                                                                                                                                                                                                                            | "You pay"                                                 |
| `summary.assets[].net` > 0                                                                    | the verdict line                                                                                                                                                                                                                                                                                                                | "You receive"                                             |
| `summary.assets[].net` = 0                                                                    | one line, soft                                                                                                                                                                                                                                                                                                                  | "No change"                                               |
| `summary.assets[].conditional_received`                                                       | its own soft line under the asset, **never summed into `net`** and never called received                                                                                                                                                                                                                                        | "Only if the counterparty completes"                      |
| `summary.assets[].sent` / `received`                                                          | inside the disclosure only — the gross figures mislead on their own                                                                                                                                                                                                                                                             | —                                                         |
| `summary.outputs[]` (`amount`, `asset`, `recipient`, `to_user`)                               | "Sending": each output that is not `to_user`, amount in the display face → the address Klimper encodes itself from `recipient` (network prefix, shortened, whole address on hover); `to_user` outputs as one "Change back to you" line; when every output is `to_user`, the teal "Only to your own addresses" note listing them | "Sending", "Only to your own addresses"                   |
| `summary.implied_fee`                                                                         | a line under the assets                                                                                                                                                                                                                                                                                                         | "Network fee"                                             |
| `summary.reserve_fee`                                                                         | a line under the assets, when non-zero                                                                                                                                                                                                                                                                                          | "Fee reserved by the spend"                               |
| `summary.unknown_puzzles[]`                                                                   | danger panel, every puzzle hash in full and selectable; gates a second confirmation                                                                                                                                                                                                                                             | "Unknown contract"                                        |
| `summary.time_locks`                                                                          | one line per lock, with the absolute time                                                                                                                                                                                                                                                                                       | "Not spendable before" / "Expires"                        |
| `plan.ours[].is_unsafe` or `plan.has_unsafe`                                                  | the loudest danger panel on the screen, red, icon, never collapsed                                                                                                                                                                                                                                                              | "Unsafe signature"                                        |
| `plan.ours.length`                                                                            | a soft line inside "Details"                                                                                                                                                                                                                                                                                                    | "Signatures you would produce"                            |
| `plan.foreign`                                                                                | count only, inside the disclosure                                                                                                                                                                                                                                                                                               | —                                                         |
| `partial`                                                                                     | a panel explaining that this is one side of a multi-party spend                                                                                                                                                                                                                                                                 | "This is one half of a trade"                             |
| `binding.all_bound` = true                                                                    | inside that panel                                                                                                                                                                                                                                                                                                               | "Klimper checked the other side is bound"                 |
| `binding.all_bound` = false                                                                   | danger panel — this should have been refused upstream                                                                                                                                                                                                                                                                           | "Not bound — do not sign"                                 |
| `binding.bound_payments[]`                                                                    | what the signature depends on receiving, per payment                                                                                                                                                                                                                                                                            | "Your signature only pays out if…"                        |
| `trade.pays[]`, `trade.receives[]` (`asset`, `amount`)                                        | the trade card at the top: you pay, you receive, per asset; a malformed leg makes the prompt unreadable                                                                                                                                                                                                                         |
| `trade.fee`                                                                                   | "Your network fee" in the trade card: the part of the user's `RESERVE_FEE` the user's own XCH funds                                                                                                                                                                                                                             |
| `trade.price` (`base`, `quote`, `quote_per_base`, `base_per_quote`)                           | "Price: 1 base = x quote" with a flip; dropped unless both are plain decimals                                                                                                                                                                                                                                                   |
| `trade.offers`                                                                                | "N separate offers taken together — all or nothing" when above 1, and the headline "Take N offers together"; dropped unless a whole number 1–10000                                                                                                                                                                              |
| `summary.options[]` (`action` `mint` / `exercise` / `clawback` / `transfer`, by explicit key) | the option card (above); an unknown action, a missing flag, a malformed id, time or term makes the prompt unreadable; absent is none                                                                                                                                                                                            | "Write option", "Use option", "Claim back", "Send option" |
| `summary.outputs[].option` (`launcher_id`, `role` `collateral` / `launcher`)                  | listed as locked under the option, never as a payment; its `recipient` is dropped; malformed is unreadable                                                                                                                                                                                                                      | "locked under Option …"                                   |
| `asset.type == "option"` (`asset_id`)                                                         | `Option <8 hex>…<6 hex>`, counted whole (one mojo, one option); never XCH; an option as collateral or strike is unreadable                                                                                                                                                                                                      | —                                                         |
| `intent.kind_verified`                                                                        | "verified by your wallet" next to the kind; must be a boolean, and true only for one of the six verified kinds, else unreadable                                                                                                                                                                                                 | "verified by your wallet"                                 |
| `loan` (`role`, `principal`, `counterparty`, `tie`)                                           | the loan card; must match the one mint in `summary.options` (the borrower holds and funded it, the lender created it, the counterparty is the other side), else unreadable                                                                                                                                                      | "Request a loan", "Lend"                                  |
| `makes_offer`                                                                                 | the request is signed, not sent: badge, slider and note say so; must be a boolean                                                                                                                                                                                                                                               | "Signed by Klimper, not sent"                             |
| `offer` (`gives[]`, `options[]` (`launcher_id`, `terms`), `asks[]`, `expires_at`)             | the offer card (above), only with `makes_offer`; it must ask for something and give something, and every option it names must be a `transfer` in `summary.options` from the user to the settlement puzzle (`to_settlement`), once; else unreadable                                                                              | "Make an offer"                                           |
| `message_text` (signMessage)                                                                  | a labelled quoted block, monospace, control characters escaped, length shown                                                                                                                                                                                                                                                    | "The website asks you to sign this text"                  |
| `message_hex` (signMessage)                                                                   | the same block when there is no text form                                                                                                                                                                                                                                                                                       | "…this data"                                              |
| `public_key` (signMessage)                                                                    | monospace, full                                                                                                                                                                                                                                                                                                                 | "With this key"                                           |
| anything else                                                                                 | **dropped**                                                                                                                                                                                                                                                                                                                     | —                                                         |

The "Everything Klimper computed" disclosure prints the **parsed** view, not the prompt
JSON. Printing the JSON would put every key the prompt happens to carry on the screen,
which is the one thing this screen must not do — a field a dApp could influence would be
rendered after all, just inside a disclosure. (That is not hypothetical care: the test
that asserts no dApp string reaches the screen failed on exactly this.)

`conditional_received` is the field this table exists for. It is a number the user would
read as income; it is income only if somebody else acts. It is never added to a total, never
rendered next to `net` without its sentence, and never styled like a gain.

## Copy, English and German

The security-critical strings. German is the second launch language (`src/locales/de-DE`);
these are the translations to put in the catalogue, written for a reader who does not know
what a puzzle hash is.

| Key                    | English                                                                                                                  | German                                                                                                                                          |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| verified domain        | This domain proved it owns the key that signed the code. Only the domain is verified — the name below is what it claims. | Diese Domain hat bewiesen, dass ihr der Schlüssel gehört, der den Code signiert hat. Geprüft ist nur die Domain — der Name ist eine Behauptung. |
| non-ASCII warning      | This domain uses letters outside the English alphabet. Check it character by character.                                  | Diese Domain benutzt Buchstaben außerhalb des englischen Alphabets. Prüfen Sie sie Zeichen für Zeichen.                                         |
| mixed scripts          | This domain mixes alphabets, which is how look-alike domains are built. Check it carefully.                              | Diese Domain mischt Alphabete — so werden Domains gebaut, die echten zum Verwechseln ähnlich sehen. Prüfen Sie genau.                           |
| failure dead end       | There is no way to continue past this. Ask the website for a new code.                                                   | Hier geht es nicht weiter. Lassen Sie sich von der Website einen neuen Code geben.                                                              |
| code already used      | This code was already used, possibly on another device. Ask the website for a new one.                                   | Dieser Code wurde schon benutzt, möglicherweise auf einem anderen Gerät. Lassen Sie sich einen neuen geben.                                     |
| SAS                    | The website shows the same six digits. If they are different, somebody is in the middle.                                 | Die Website zeigt die gleichen sechs Ziffern. Wenn sie abweichen, sitzt jemand dazwischen.                                                      |
| simulated, not claimed | This is what Klimper worked out by running the spend itself, not what the website says it does.                          | Das hat Klimper selbst ausgerechnet, indem es die Ausgabe ausgeführt hat — nicht, was die Website behauptet.                                    |
| conditional            | Only if the counterparty completes                                                                                       | Nur wenn die Gegenseite ihren Teil erfüllt                                                                                                      |
| unsafe signature       | This signature would be valid in any spend, not just this one. Reject it unless you know exactly why it is needed.       | Diese Signatur wäre in jeder Ausgabe gültig, nicht nur in dieser. Lehnen Sie ab, außer Sie wissen genau, warum sie nötig ist.                   |
| unknown contract       | Klimper does not recognise this contract. Nobody can tell you what it does with your coins.                              | Klimper kennt diesen Vertrag nicht. Niemand kann Ihnen sagen, was er mit Ihren Coins macht.                                                     |
| unbound partial        | Not bound — do not sign. Your signature would not be tied to the payment you are expecting.                              | Nicht gebunden — nicht signieren. Ihre Signatur wäre nicht an die erwartete Zahlung gebunden.                                                   |
| dismiss is reject      | Leaving this screen rejects the request.                                                                                 | Wenn Sie diesen Bildschirm verlassen, wird die Anfrage abgelehnt.                                                                               |
| notification           | A connected website is asking you to sign something. Open Klimper to see what it is.                                     | Eine verbundene Website möchte, dass Sie etwas signieren. Öffnen Sie Klimper, um zu sehen, was.                                                 |
| limit refused          | A request over a limit is refused before it reaches you — you will not see a signing prompt for it.                      | Eine Anfrage über dem Limit wird abgelehnt, bevor sie bei Ihnen ankommt — dafür erscheint keine Signatur-Abfrage.                               |

Rules the copy follows: no word for a state the wallet cannot prove ("safe", "trusted",
"official"); the domain is never described as "verified" without the sentence that says
_only_ the domain is; the word "website" rather than "dApp" wherever a user reads it, and
`dApp` only in headings that name the feature.

## What the screens cannot do yet

Reported rather than faked. Each needs a plugin command that the transport does not expose:

- **Return to the dApp** (TASK-085 AC #1, #2). The verified `return_url` reaches the plugin
  as `VerifiedUri::return_url` and the core already refuses one that leaves the dApp's
  domain, but it is neither persisted with the session nor surfaced on `SessionSummary`.
  The guard is implemented and tested in `xchonnectLink.ts`; it has nothing to guard until
  the field exists.
- **Same-device requests at all** (TASK-085 AC #1). The pairing reply sets `WalletMeta`
  with a name only. Without `link: "https://clapandpay.com"` a dApp cannot build a `/req`
  link and fails with `no_wallet_link`.
- **OS link delivery** (TASK-072 AC #2). `tauri-plugin-deep-link` is not installed, so
  nothing hands the app a `/pair` or `/req` URL. `handleIncomingXchonnectLink()` is the
  single entry point that a deep-link listener calls; `docs/xchonnect-links.md` has the
  wiring and the hosting steps.
