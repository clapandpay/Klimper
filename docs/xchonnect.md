# Xchonnect dApp connections

Klimper answers CHIP-0002 signing requests from outside websites over
[Xchonnect](https://github.com/maximedogawa/xchonnect). Recorded as clapandpay
**decision-008**; the wiki page is `architecture/xchonnect.md`. Normative behaviour is
xchonnect's `docs/spec/xchonnect-spec.md`; the wallet side is `docs/wallet-integration.md`.

Everything lives in `tauri-plugin-klimper-xchonnect/` and `src/klimper/`, behind
`VITE_ENABLE_XCHONNECT` (off). `docs/UPSTREAM.md` lists every shared file that was touched.

The screens are `docs/xchonnect-screens.md` (what is shown, and the rendering table for
every prompt field) and the entry points are `docs/xchonnect-links.md` (QR codes,
universal links, the same-device round trip).

## Where the thinking happens

Two Apache-2.0 crates, consumed by path for now:

| Crate                  | Does                                                                                                                                                                    |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `xchonnect-core`       | URIs, origin verification, pairing, sessions, envelopes, the six-digit SAS                                                                                              |
| `xchonnect-wallet-kit` | runs every requested spend locally, derives the effect from the conditions, signature policy, permissions and limits, the multi-party binding check, CHIP-0002 handlers |

This plugin supplies the four things a host has to and nothing else: HTTP, storage, the
approval UI, and signing.

## Modules

| Module         | Task          | Does                                                                                                                                 |
| -------------- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `store`        | TASK-073      | session state in the OS keyring under `com.clapandpay.klimper.xchonnect`; per-wallet; rollback detection; the persistent limit store |
| `relay`        | TASK-074      | mailbox creation (ticket or proof-of-work off the UI thread), long polling, ack, drain, delete                                       |
| `origin`       | TASK-075      | the origin document: HTTPS, no redirects, no cookies, 16 KiB, 10 s, every pairing                                                    |
| `pairing`      | TASK-075, 076 | verified domain in both forms, the default grant, the SAS, persist before posting `session.ready`                                    |
| `session_loop` | TASK-077      | requests, rotation, ping, `session.end`; seal → persist → post → acknowledge                                                         |
| `wallet`       | TASK-078      | ownership and the signable key set from the Sage database; signing through `sage-keychain`                                           |
| `requests`     | TASK-079      | the request context, the approver bridge, the limit store, the response                                                              |
| `approval`     | TASK-078, 079 | wallet-kit's synchronous hooks parked on a channel until the webview answers                                                         |

## Session storage

Two keyring entries per session, so the message loop and the limit store never overwrite
each other:

| Entry                        | Holds                                        |
| ---------------------------- | -------------------------------------------- |
| `session:<fingerprint>:<id>` | the session record and `Session::to_bytes()` |
| `limits:<fingerprint>:<id>`  | the daily spend totals                       |

A plaintext index, `xchonnect-sessions.json` in the Sage app data directory, lists the
session ids per fingerprint with the last `state_version` written. The keyring entry is
written **before** the index, so a crash leaves the keyring ahead of the index and never
behind it. An entry whose `state_version` is below the index is restored state: reusing a
`seq` would let a replay through, so the session is marked as needing re-pairing instead
(spec 5.3).

`crates/sage-keychain` is untouched. Nothing Xchonnect ever reaches `keys.bin`.

## Connecting and disconnecting

- **One loop per session, ever.** `live` holds each loop's handles; an entry counts only
  while its loop still holds the `wake` receiver. Entries are never pruned by the open
  wallet's list, which used to orphan another wallet's loops and let a second loop start on
  the same session (two writers of one `seq`).
- **Disconnect is immediate.** `end_session` rejects whatever the session is waiting on,
  cuts the loop's long poll short (`end_signal`), and drops the session from
  `list_sessions` while the loop posts `session.end` (retried, not on a `404`) and deletes
  the mailboxes. A loop that already returned is not flagged and forgotten: the session is
  ended through `end_without_loop`. `tests/live_relay.rs`
  `a_disconnect_does_not_wait_for_the_long_poll` checks it against the reference relay.
- **The dApp ending it** (`session.end`, or a `404` from the relay) runs the same clean-up,
  including our own mailbox and any open prompt.
- **Pairing attempts are locked one by one.** The `session.confirm` poll holds only its own
  attempt, so cancelling, confirming or scanning another code never waits for it. If
  `session.ready` cannot be posted, the stored session is undone and the attempt discarded,
  rather than listing a connection the website never learned about.
- **Renewing keys** (spec 9.2.1). `rotate_session_keys` asks the session's loop to offer
  new keys: a fresh mailbox, then `session.rotate` under the current epoch. The loop also
  offers on its own once `Session::needs_rotation` says so (30 days or 10 000 messages).
  On the dApp's accept it switches, pings the dApp's new mailbox so the dApp may retire
  its old one, then drains and deletes its own old mailbox. A dApp that never answers
  leaves the offer pending; the session keeps working on the old keys, and a dApp offer
  of its own still wins.
- **Origin-key revocation** (spec 6.1). The record keeps the `kid` the pairing code was
  signed with, and the loop re-fetches the origin document when it starts and every 24
  hours. Only a document that was fetched, parsed and no longer lists the `kid` revokes:
  the loop posts `session.end`, deletes the mailboxes, erases the session bytes and keeps
  the record with `revoked: true`, which the screen shows until the user removes it. A
  site that is down never revokes anything. Sessions paired before the `kid` was kept
  cannot be checked.
- **Still open:** an end the app quits in the middle of is not resumed at the next start
  (the session comes back and can be ended again).

## On phones (TASK-125)

What keeps a website's messages flowing when the browser and Klimper take turns in front:

- **What a website sees first.** `getPublicKeys` and `session.permissions` list the wallet's
  receive addresses first and the website's own key last; websites read the first key as the
  wallet. Connections stored the other way round (2026-10-08 to TASK-125.01) are reordered
  when they load, and the website is told.
- **Reads never wait.** Keys, network, balances and coins are answered without a receipt,
  and at once even while a signing request waits for the user.
- **Nothing dropped.** A request is persisted when opened; its receipt is best-effort and
  its answer is posted again until the request expires. A loop reads first and does its
  housekeeping (relay limits, origin check, push registration) afterwards.
- **Back in front.** `RunEvent::Resumed` wakes every loop and drops a long poll that may sit
  on a socket the phone cut; the relay client has a 5 s connect timeout, a 10 s pool idle
  limit and keepalive. The requests queue and the connection list are read again.
- **Leaving Klimper.** iOS keeps the app running about 25 s after the user switches to the
  browser (a background task), so the website's next reads are answered.
- **Locked phone.** Session secrets are `AfterFirstUnlockThisDeviceOnly`; a push with
  `content-available` (remote-notification background mode) wakes the app, Rust reads every
  mailbox, and only then is iOS told the fetch finished. A request that needs the user waits
  in the queue; the Websites entry shows the count, and opening Klimper shows the pop-up. The
  gateway must send `content-available` for this (xchonnect `fix/mobile-reliability`).
- **Gone means gone three times.** A mailbox the relay no longer knows ends a connection only
  after three answers 5 s apart; a website ending it is shown as a notice.
- **Networks.** On a connection approved for the other network, `chainId` and `connect` are
  answered with the wallet's current network; everything else is refused with
  `{"reason":"wrong_network","walletChainId":…,"sessionChainId":…}`, and the websites screen
  marks the connection.
- **Not yet.** Android has no equivalent of the background drain when the process is gone;
  messages read while a prompt waits are held in memory and lost if the app is killed then.

## One decision per request; Klimper broadcasts

Decided 2026-10-05 (xchonnect-wiki `04-wallet-side-execution.md`): spec 11.1 item 4 is one
authentication per signing **request**, covering all of its signatures. The earlier
desktop reading — one confirmation per signature — asked 1 + 5 times for a five-coin send
and is gone.

1. **One approval** of the whole request, on the overlay or the requests page.
2. **One unlock**: `KlimperSigner::sign_all` opens the keychain once, derives every key the
   approved plan needs, signs and verifies everything, and drops the key material. The
   keychain is Sage's own, opened with Sage's empty password like every Sage endpoint. A
   fresh Touch ID prompt per request on macOS is a follow-up:
   it needs Objective-C calls the workspace's `unsafe_code = "forbid"` rules out here.
3. **Klimper broadcasts** (`execute.rs`): `chia_send` and `chia_takeOffer` are built by
   Sage from the wallet's own coins, `xchonnect_submitCoinSpends` carries a bundle the
   website built; each is signed once and sent through Klimper's own peers
   (`send_transaction_immediately`). The website receives `{ transactionId, status }`.
   `chia_takeOffer { offer, fee? }` (TASK-137) works as in `WalletConnect`: the website sends
   only the offer. Before anything is built, the offer is refused if it carries more than 64
   spends, asks for nothing, has a maker spend that is not a recognised puzzle (an NFT, a DID,
   an option: whatever the website's "unknown contracts" setting) or a maker signature that
   does not cover the maker's spends; each refusal counts like a failed build. The take is
   then built from the wallet's own coins with Sage's offer code and **no key** (the wallet's
   `take_offer`, unsigned), and each of the user's spends is tied to the maker's payment
   (spec 11.2): with a fee, Sage leaves one of the user's coins asserting nothing, and that
   coin then asserts a coin announcement of a bound user spend. The key is used once, by
   wallet-kit, after the approval. The intent is `offer.take` and claims **no fee**:
   wallet-kit checks a fee claim against the whole bundle's fee, which also holds the maker's
   own fee. What the user pays, their fee included, is the net change the approval shows; the
   whole trade's fee is shown as such. `src/execute/take_tests.rs` takes real offers between
   two Sage wallets on a simulated chain and checks that each settles.
   **Several offers at once** (TASK-139): an offer may be several makers' offers joined into
   one, their spends together and their signatures summed, as Sage's `aggregate_offers`
   makes it, by plain concatenation, or merged (one placeholder per requested asset carrying
   every maker's notarized payments, as chia's `Offer.aggregate` writes it and Pengui sends
   by default). It is checked, built and taken the same
   way, in one approval, all or nothing; the one extra check is that no maker coin is spent
   twice (one offer joined twice is refused before the user is asked,
   `offer_duplicate_coin`). The 64-spend bound holds for the aggregate as a whole.
   `src/execute/aggregate_tests.rs` takes two and three makers' offers (one CAT bought from
   all, one CAT sold to all, CATs for a CAT, with both fees, joined all three ways) and checks each
   settles; a forged or missing signature of one maker refuses the whole. The approval's
   **trade overview** is Klimper's own prompt key `trade`, next to `cross_checked`, computed
   by the wallet (`src/trade.rs`) from the simulation of the bundle it signs: what is paid and
   received (bound settlement payments counted as received, since the user's spends assert
   them), the user's own fee (`RESERVE_FEE` of the user's spends, only as far as the user's
   own XCH funds it: a fee another party's coin pays is neither the user's nor received), the
   price of a one-for-one trade both ways round, and for a take how many offers it holds: the
   smaller of the distinct nonces of its requested payments and the groups of maker coins not
   tied to each other (a maker picks nonces freely). `tests/fixtures/aggregate_offers.json`
   holds real offers, Sage's aggregate of them, the plain concatenation and the merged form,
   for a website to test its
   own aggregation against (`export_aggregate_fixtures`, ignored, writes it).
   A connection paired before `chia_takeOffer` existed is given it once, when Klimper next
   loads it, if it may use `chia_send` (`methods_revision`, `execute::upgrade_methods`), and
   the website is told by `session.permissions`; it does not have to pair again. A take the
   user turns off afterwards stays off.
   **Option contracts** (TASK-140, `src/execute/options.rs`), built by the wallet like a send
   and shown from wallet-kit's own option facts (xchonnect spec 11.1.1):

   | Method | Params | Intent kind (verified) |
   | --- | --- | --- |
   | `chia_mintOption` | `underlying: {assetId?, amount}`, `strike: {assetId?, amount}` (`assetId` absent or `"xch"`: XCH; else a CAT; not the same asset), `expiresAt` (unix seconds, 60 s to 10 years ahead), `creatorAddress?`, `loan?: {assetId?, amount}`, `fee?` | `option.mint`, or `loan.request` |
   | `chia_exerciseOption` | `launcherId`, `fee?` | `option.exercise` |
   | `chia_claimOption` | `launcherId`, `fee?` | `option.clawback` |
   | `chia_transferOption` | `launcherId`, `address`, `fee?` | `option.transfer` |

   - **Write** (`chia_mintOption`, no `creatorAddress` or one of the wallet's own): the wallet
     locks the collateral, is the creator and holds the option. Built from the wallet's coins
     with Sage's coin selection and spend code and the SDK's option launcher, key-free: the fee
     and the launcher's coin, then the collateral locked at the option's collateral puzzle (XCH
     set aside first at another of the wallet's addresses, so a one-coin wallet works), then
     the launch, whose eve option has a quoted inner puzzle: its outputs are fixed and
     wallet-kit shows the option as guaranteed. Sage's own `mint_option` spends the eve with
     the wallet's standard puzzle, which wallet-kit counts as conditional, so `option.mint`
     would not verify. The two coin sets assert each other's spend; the launcher's parent
     asserts the launcher's announcement. Intent: nothing paid to anyone (collateral and
     launcher are option outputs), this fee. Answer `{ transactionId, status, launcherId }`.
   - **Loan request** (`creatorAddress` someone else's, and `loan`, the principal): the
     creator is the lender, the option (the right to repay) stays with the borrower. Nothing is
     sent: the same build plus an assertion that the principal is paid to the wallet and a
     1-mojo payment offered to whoever takes it, signed after one approval and returned as an
     offer, `{ offer, launcherId }`, valid until `expiresAt`. The mojo is what lets wallet-kit
     bind the lender's take (spec 11.2 binds a taker only to a settlement payment it
     receives). A lender's address without `loan`, or `loan` without a lender, is refused.
   - **Lend**: the lender takes the request with `chia_takeOffer`. When the bundle mints an
     option whose creator is the wallet and whose collateral and option are someone else's,
     the intent is `loan.take` (verified: the creator is the wallet). A take that mints an
     option neither the wallet's to hold nor to claim is refused (`option_not_yours`).
   - **Use** (`chia_exerciseOption`): Sage's `exercise_options`, only before the expiry
     (`option_expired`), for an option the wallet holds (`unknown_option`). Every user spend,
     the melted option included, is tied to the settlement payment that releases the
     collateral to the wallet (wallet-kit refuses an unbound exercise, `unbound_exercise`).
   - **Claim back** (`chia_claimOption`): Sage has none, so it is built from the SDK's
     clawback spend, only from the expiry on (`option_not_expired`) and only by the creator
     (`not_creator`). The terms come from the wallet database, or for an option the wallet never
     held (a loan it lent) from the chain through a peer, as Sage reads an option it is offered.
     The fee comes out of XCH collateral (`fee` if larger), or from the wallet's XCH for a CAT.
   - **Send** (`chia_transferOption`): Sage's `transfer_options`; the intent claims exactly this
     option to exactly this address.
   - **Permissions**: the four methods are **not** in a new connection's default grant and no
     existing connection is upgraded to them (unlike `chia_takeOffer`): they lock funds until a
     date. The pairing screen and the website's settings have an "Option contracts" switch
     (`confirm_sas`'s `optionContracts`, `set_dapp_policy`'s `methods`). A refusal names
     `not_granted`. A paused website, the wrong network and the build-failure counter apply as
     for a send.
   - **Limits**: wallet-kit counts the collateral a mint locks and the strike an exercise pays
     in their own assets against the per-request and per-day limits; an option itself has none.
   - **Cross-check**: options are left out of the comparison with Sage's summary (Sage does not
     see an eve the request mints as the wallet's coin); where an option goes is wallet-kit's
     own option facts and the verified intent.
   - **Approval**: Klimper's prompt keys `loan` (`role`, `principal`, `counterparty`, `tie`) and
     `makes_offer` (a loan request signs, does not send), next to `trade`; the option card is in
     `docs/xchonnect-screens.md`.
   - **Tests**: `src/execute/option_tests.rs` writes XCH- and CAT-backed options with XCH and CAT
     strikes, sends one, uses it before the expiry and not after, claims back after the expiry
     (and not before, and not someone else's), buys an offered option with `chia_takeOffer`,
     and requests, lends, repays and defaults on a loan, two Sage wallets on a simulated chain,
     every bundle accepted by the chain; refusals without the permission, over the limit, on
     the wrong network, and a website's mint with a fake collateral (an unknown contract).
   - **Dependency**: wallet-kit's option support is in xchonnect 0.1.0-rc.5, not yet on
     crates.io; `../Cargo.toml` patches both crates to the git revision until it is published.
4. **Checked twice**: before the prompt, Sage's summary from the wallet database and
   wallet-kit's simulation must agree on the user's coins spent, the payments to others and
   the fee, or the request is refused (`crosscheck_mismatch`). A website's declared intent
   is verified claim by claim against the simulation; `chia_send` and `chia_takeOffer` carry
   one Klimper derives from the request itself.

Key material lives only inside one `sign_all` call. `chia_bls::SecretKey` has neither `Drop` nor
`Zeroize` in chia-bls 0.36, so its bytes are not scrubbed; the workspace denies
`unsafe_code`, so that cannot be fixed from here. Worth proposing to chia-bls.

## Running it against a local relay

xchonnect's `scripts/dev.sh` starts an in-memory relay and its example dApp:

```sh
cd ../../xchonnect/xchonnect && ./scripts/dev.sh      # relay on :8787, dApp on :5173
```

Then, in `klimper/`:

```sh
echo 'VITE_ENABLE_XCHONNECT=true' >> .env.local       # .env.local is git-ignored
pnpm tauri dev
```

Open the dApp and press "Connect wallet". In Klimper, Settings → Connected websites →
Connect a website, then either paste the `xchonnect:v1?…` code from the clipboard or
choose a screenshot of the QR; compare the six digits, and approve the request.

Developer-mode codes — a plain-HTTP loopback relay and a `localhost` origin document —
are accepted **only** in a debug build (`cfg!(debug_assertions)`). There is no setting and
no environment variable that turns that on in a release build.

## Not done here

- **Push wake-ups**: clapandpay TASK-086 and TASK-087. The message loop waits on a channel
  rather than on a sleep, so a push becomes another sender.
- **OHTTP**: TASK-089. Every relay call goes through one `DirectTransport`, which is the
  piece an encapsulating client replaces.
- **OS link delivery**: `tauri-plugin-deep-link` is not installed, so nothing hands the
  app a `/pair` or `/req` URL yet; `docs/xchonnect-links.md` has the four lines and the
  hosting that is left.
- **Same-device requests**: the pairing reply does not announce the wallet's link base
  (`WalletMeta.link`), and the verified `return_url` is not persisted with the session or
  surfaced on `SessionSummary`. Both are small plugin changes; the UI guard is written and
  tested (`src/klimper/xchonnect/xchonnectReturn.ts`).
- **Coin and balance reads, `sendTransaction`**: TASK-082. Only the methods wallet-kit
  handles are answered; anything else returns the CHIP-0002 method-not-found code.
