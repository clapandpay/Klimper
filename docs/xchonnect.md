# Xchonnect dApp connections

Klimper answers CHIP-0002 signing requests from outside websites over
[Xchonnect](https://github.com/maximedogawa/xchonnect). Recorded as clapandpay
**decision-008**. Normative behaviour is
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

| Module         | Task          | Does                                                                                                                                                                    |
| -------------- | ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `store`        | TASK-073      | session state in the OS keyring under `com.clapandpay.klimper.xchonnect`; per-wallet; rollback detection; the persistent limit store                                    |
| `relay`        | TASK-074      | mailbox creation (ticket or proof-of-work off the UI thread), long polling, ack, drain, delete                                                                          |
| `origin`       | TASK-075      | the origin document: HTTPS, no redirects, no cookies, 16 KiB, 10 s, every pairing                                                                                       |
| `pairing`      | TASK-075, 076 | verified domain in both forms, the default grant, the SAS, persist before posting `session.ready`                                                                       |
| `session_loop` | TASK-077      | requests, rotation, ping, `session.end`; seal → persist → post → acknowledge                                                                                            |
| `wallet`       | TASK-078      | ownership and the signable key set from the Sage database; signing through `sage-keychain`                                                                              |
| `requests`     | TASK-079      | the request context, the approver bridge, the limit store, the response                                                                                                 |
| `sign_guard`   | TASK-141      | refuses spends whose conditions the signature does not fix, before wallet-kit or the user sees them                                                                     |
| `signed_coins` | TASK-140      | the record of signatures outside the wallet (released coins, pruned per coin from the chain; sealed signatures per bundle, never pruned); sealed bundles kept to resend |
| `approval`     | TASK-078, 079 | wallet-kit's synchronous hooks parked on a channel until the webview answers                                                                                            |

## Session storage

Two keyring entries per session, so the message loop and the limit store never overwrite
each other:

| Entry                        | Holds                                        |
| ---------------------------- | -------------------------------------------- |
| `session:<fingerprint>:<id>` | the session record and `Session::to_bytes()` |
| `limits:<fingerprint>:<id>`  | the daily spend totals                       |

One more entry per wallet and network, `signed:<fingerprint>:<chain id>`, holds the record of
signed coins (below, "Sealed and released").

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

Decided 2026-10-05: spec 11.1 item 4 is one
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
   `take_offer`, unsigned) and handed to wallet-kit exactly as Sage built it: the take is a
   sealed request (below, "Sealed and released"), so the links Sage makes between the
   user's spends count. The key is used once, by
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

   | Method                | Params                                                                                                                                                                                                                                        | Intent kind (verified)           |
   | --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------- |
   | `chia_mintOption`     | `underlying: {assetId?, amount}`, `strike: {assetId?, amount}` (`assetId` absent or `"xch"`: XCH; else a CAT; not the same asset), `expiresAt` (unix seconds, 60 s to 10 years ahead), `creatorAddress?`, `loan?: {assetId?, amount}`, `fee?` | `option.mint`, or `loan.request` |
   | `chia_exerciseOption` | `launcherId`, `fee?`                                                                                                                                                                                                                          | `option.exercise`                |
   | `chia_claimOption`    | `launcherId`, `fee?`                                                                                                                                                                                                                          | `option.clawback`                |
   | `chia_transferOption` | `launcherId`, `address`, `fee?`                                                                                                                                                                                                               | `option.transfer`                |
   - **Write** (`chia_mintOption`, no `creatorAddress` or one of the wallet's own): the wallet
     locks the collateral, is the creator and holds the option. Sage's own `mint_option`, key-free,
     unchanged; sealed, so Sage's `ASSERT_CONCURRENT_SPEND` between the collateral and the launch
     makes the launch guaranteed (XCH or CAT collateral). Intent: nothing paid to anyone
     (collateral and launcher are option outputs), this fee. Answer
     `{ transactionId, status, launcherId }`.
   - **Loan request** (`creatorAddress` someone else's, and `loan`, the principal): the
     creator is the lender, the option (the right to repay) stays with the borrower. Sage has no
     builder for an option written for someone else, so Klimper builds it from the wallet's
     coins with Sage's coin selection and spend code and the SDK's option launcher (XCH
     collateral set aside first at another of the wallet's addresses, so a one-coin wallet
     works; the eve's inner puzzle is quoted, so its outputs are fixed). Nothing is sent: the
     signed spends go back as an offer, `{ offer, launcherId }`, valid until `expiresAt`, so
     its signatures are **released** and only direct assertions bind it (spec 11.2): every
     spend of the borrower asserts the settlement announcement of the principal paid to the
     wallet, and every spend that locks the collateral asserts the launcher's announcement.
     A 1-mojo payment is offered to whoever takes it, so wallet-kit can bind the lender's take
     (spec 11.2 binds a taker only to a settlement payment it receives). A lender's address
     without `loan`, or `loan` without a lender, is refused.
   - **Lend**: the lender takes the request with `chia_takeOffer`. When the bundle mints an
     option whose creator is the wallet and whose collateral and option are someone else's,
     the intent is `loan.take` (verified: the creator is the wallet). A take that mints an
     option neither the wallet's to hold nor to claim is refused (`option_not_yours`).
     wallet-kit does not verify `loan.take` yet (xchonnect TASK-78), so a lender's take is
     refused before the lender is asked (`intent_mismatch`).
   - **Use** (`chia_exerciseOption`): Sage's `exercise_options`, only before the expiry
     (`option_expired`), for an option the wallet holds (`unknown_option`), as Sage builds it:
     sealed, so Sage's links tie every user spend, the melted option included, to the spend
     that receives the collateral (wallet-kit refuses an unbound exercise, `unbound_exercise`).
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
     in their own assets against the per-request and per-day limits. Options leaving the user
     (sent to anyone else, or offered; not used by the user) have a limit of their own, a
     count: one per request and five per UTC day by default (`DappPermissions::options`,
     changeable in the website's settings next to the other limits, stored in the session
     record as `permissions.options` only when it differs from the default; the day's count
     is stored with the daily totals as `options`). Refusals are `4029`
     `options_per_request` / `options_per_day`, explained to the website.
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
     **Making offers** (TASK-142, `src/execute/create_offer.rs`): `chia_createOffer` as Sage's
     `WalletConnect` has it, so a website (Pengui's open loan offers: a lender writes an option,
     then offers it with the principal for the collateral) needs no Klimper-only path.

   | Method             | Params                                                                                                                    | Answer                                                                                                         | Intent kind                                                               |
   | ------------------ | ------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
   | `chia_createOffer` | `offerAssets: [{assetId?, amount}]`, `requestAssets: [{assetId?, amount}]`, `fee?` (other keys, e.g. `walletId`, ignored) | `{ offer: "offer1…", id }` (`id`: Sage's offer id, plain hex of the sorted offer's name); nothing is broadcast | `option.transfer` (verified) when it gives an option, else `offer.create` |
   - **Assets**: `assetId` absent, `null`, `""` or `"xch"` (any case) is XCH; 32 bytes of hex,
     with or without `0x`, a CAT; an option is named as Sage names it, by its `option1…` id
     (bech32m of the launcher id, prefix `option`) with `amount` 1. Amounts as Sage's
     `safeAmount` (a non-negative integer or a decimal string), positive; one asset named twice
     is added up, an option named twice is refused. At most 16 entries a side, at least one;
     `requestAssets` only XCH and CATs; giving and asking for the same asset is refused.
     NFTs (`nft1…`) and asking for an option are refused (`4101` `unsupported_asset`). `fee` is read and
     ignored, as for every request Klimper builds (Klimper's own fee).
   - **Build**: from the wallet's coins with Sage's offer code (`prepare_spends_for_selection`,
     `select_spends`, the settlement payments, the requested payments to the change address
     under the nonce of the coins spent, `complete_spends`), key-free, as Sage's `make_offer`
     builds them. One difference: the offer's signatures go to the website, so they are
     **released** and only direct assertions bind (spec 11.2). Sage gives the assertion of the
     requested payments to one spend and ties the others by `ASSERT_CONCURRENT_SPEND`, which
     does not bind a released offer; here every spend of the user's (XCH, CAT and option)
     asserts every requested payment, and, when the offer gives options, `ASSERT_BEFORE_SECONDS_ABSOLUTE`
     of the earliest option expiry, so nobody can take the offer once an option it sells can no
     longer be used. Then the sign guard, Sage's summary against the simulation, wallet-kit's
     submit (released, `linking: "direct"`), one approval; the signed spends and the
     placeholders of what is asked become the offer. The coins of the offer go into the record
     of signed coins as released before the answer leaves. The offer is not imported into
     Sage's offer list (Sage's `WalletConnect` does not either), so Sage may spend its coins
     later, which cancels it.
   - **Refusals**: an option the wallet does not hold (`4000` `unknown_option`), an expired one
     (`4001` `option_expired`), an option whose strike or collateral the card cannot show (a
     revocable CAT, an NFT; `4101` `unsupported_asset`), more than the wallet holds or an asset it does
     not have (`4003` `build_failed`, Sage's words never sent; these count towards the
     failed-build limit, and so do the refusals above), malformed params (`4000` with the field
     as reason: `offerAssets`, `requestAssets`, `fee`, `json`).
   - **Permissions**: `chia_createOffer` is granted like `chia_takeOffer`: in a new connection's
     default grant, and a connection paired before it existed is given it once if it may use
     `chia_send` (`METHODS_REVISION` 2; a method the user turned off stays off). Giving an
     **option** also needs the connection's option contracts (`chia_transferOption`), or it is
     refused before anything is built (`4001` `not_granted`); the option-count limit applies
     (an offered option leaves the user: `options_per_request` / `options_per_day`), and XCH and
     CATs given count against the per-request and per-day limits.
   - **Approval**: Klimper's prompt keys `offer` (`gives`, `options` with `launcher_id` and
     `terms` from the wallet's database in wallet-kit's `terms` shape, `asks`, `expires_at`) and
     `makes_offer`; the screen requires every option in `offer` to be a transfer from the user to
     the settlement puzzle in wallet-kit's own option facts (`docs/xchonnect-screens.md`).
   - **Tests**: `src/execute/create_offer_tests.rs`: an open loan offer (an option on a CAT and
     XCH for the CAT) and an option with a CAT for XCH, made through Klimper's path and taken by a
     second wallet with `chia_takeOffer` on the simulated chain (the taker holds the option and
     the principal, the maker the asset asked), a plain XCH-for-CAT offer; the refusals above.

4. **Sealed and released** (xchonnect spec 11.2, `src/signed_coins.rs`). Whether one of the
   user's spends could be taken out of a bundle and used without the others depends on who
   holds the user's signatures:
   - **Sealed**: Klimper built the request, sends it itself and answers the website only
     with the transaction id and status (`chia_send`, `chia_takeOffer`, writing, using,
     claiming and sending an option). Every signature exists only inside the one bundle
     Klimper sends, so the user's spends reach the network together or not at all, and the
     links Sage's builders make between them count. Sage's bundles are signed as Sage builds
     them; Klimper adds nothing. The approval shows **Sealed by Klimper** (prompt key
     `linking: "sealed"`).
   - **Released**: everything else — a `signCoinSpends` (the signature goes to the
     website), a website's own bundle (`xchonnect_submitCoinSpends`), a loan request or any
     offer the wallet makes (`chia_createOffer`). Each of the user's spends must then itself assert what the user is paid, or the
     request is refused.
     A request is sealed only if no coin of the user's it spends has a signature outside the
     wallet, and only while the wallet never makes one of a sealed bundle's signatures again
     (the bundle's aggregate is public once sent, and BLS signatures are deterministic: one
     more signature over the same message would let it be split). So Klimper keeps a
     **record of signed coins**, per wallet and network, in the OS keyring next to the
     sessions (`signed:<fingerprint>:<chain id>`), across restarts:
   - **released** coins: every coin whose signature left the wallet — answered to a
     `signCoinSpends`, in a loan request's offer, or in any bundle sent (also one the network
     refused). A request spending one of the user's coins in it is never sealed.
   - **sealed signatures**: for each sealed bundle Klimper sent (or tried to: a bundle the
     network refused counts), the signatures it made in it, as `sha256(public key ||
  message)`, each with the bundles it was made in, and the coins it signed for in it
     (stored as `sealed_digests` and `sealed_bundles`, record `format` 3; the per-coin
     record of the earlier builds is moved into one bundle holding every coin it named, and
     a record that does not read, or that a later version wrote, is refused, never read as
     empty). wallet-kit refuses to make one of them again in any
     request — `signCoinSpends`, `xchonnect_submitCoinSpends`, `signMessage`, or Klimper's
     own build of the same spends (`4001`, reason `sealed_signature`; the website is told
     "Klimper already signed this exact spend for a transaction it sent, and never signs it
     again. Wait for that transaction to confirm, or build a new one."). A different spend of
     the same coin signs a different message and is allowed, so a bundle that expired or was
     refused locks no coin. Sage's own coin selection leaves out the coins of a transaction
     it holds as pending; nothing else is excluded.
     wallet-kit hands over what to record before every broadcast (`Broadcaster::record`, a
     `BroadcastRecord`); Klimper's broadcaster (`SageBroadcaster`) writes it to the keyring
     before it returns, and when that fails nothing is sent (`4000`, reason `record_failed`,
     "Klimper could not save its record of this transaction, so nothing was sent. Try
     again."). wallet-kit reports a `signCoinSpends` answer the same way, after signing and
     before the signature is returned (every coin of the request goes into released; if the
     write fails the website gets `record_failed`, no signature, and nothing counts against
     its limit). A loan request's offer is recorded as released by Klimper before the answer
     leaves; if that fails, it does not leave.
     **Signatures Sage makes outside Xchonnect** go into released too, read from Sage's own
     database without changing Sage (`signed_coins::signed_by_sage`): every coin a
     transaction in Sage's mempool table spends (Sage's send screen, its offer takes,
     anything Sage submitted) and every coin of an offer Sage made, whatever its status, as
     long as the coin is unspent. This runs before every signing request and every 30 seconds
     while the app runs. Not covered, since they leave nothing in Sage's database: a
     signature Sage's own `WalletConnect` hands to a website (`signCoinSpends`) and a signed
     transaction shown for copying instead of being sent.
     **Sealed signatures are never pruned** (wallet-kit's recommended default; it would allow
     dropping a bundle once every coin signed for in it is spent): a few dozen bytes per
     signature. A **released** coin leaves the record once it is spent on chain, as the
     wallet's own synced database says, at least 6 blocks below its synced peak
     (`signed_coins::CONFIRMATIONS`; a shallower spend could be undone by a reorg). Klimper
     checks this before every request that signs. No peer is asked: a coin Sage's database
     does not hold (another party's, or one created and spent inside a bundle) stays in the
     record for good, about 64 bytes each. wallet-kit's `host_checks` (`check_sealed_record`,
     `check_one_at_a_time`) run in Klimper's tests against this storage, `SageBroadcaster`'s
     record and the signing lock (`signed_coins.rs`).
     Requests that sign — `signCoinSpends`, `xchonnect_submitCoinSpends`, `signMessage`
     (with their `chip0002_` forms) and every request Klimper builds — run one at a time: the
     lock is taken before the record is read and held across wallet-kit's check, the approval
     prompt and the signature until what was signed is recorded, so a request that arrives
     while another's prompt is open is checked against that request's record.
     **Sending a sealed bundle again.** wallet-kit never signs its messages twice, not even
     for the identical bundle, so Klimper keeps each sealed bundle's bytes (`bundles:<fingerprint>:<chain
id>` in the keyring, at most 16, each dropped once a coin it spends is spent on chain)
     and `execute::resend_sealed` broadcasts them again unchanged (a bundle no peer took, or
     one that fell out of the mempool). There is no screen for it yet.
     For the user this means: a website can never take apart a transaction Klimper sends; a
     coin the user signed for a website stays under the strict rule until it is spent; and the
     exact same transaction is never signed twice. Signatures Sage makes outside Xchonnect
     are in the record as far as Sage's database shows them (above).
     **The fee of a sealed request is Klimper's** (spec 11.1.1, 11.2): wallet-kit credits a
     sealed bundle's fee as the wallet's own choice (in an exercise, the fee can come out of
     the released collateral), and reads a request whose user spends reserve any other fee
     than `SubmitRequest::wallet_fee` as released. So a website's `fee` (`chia_send`,
     `chia_takeOffer`, the option methods) is read (a malformed one is refused) and ignored:
     Klimper builds with the wallet's **default fee**, the one the user sets in the wallet's
     settings (the webview hands it to the plugin, `set_own_fee`; 0 until it has; at most
     **0.001 XCH**, `execute::MAX_FEE`), and passes that fee as `wallet_fee`. The approval
     shows the fee the bundle pays. A loan request uses it too, though its signatures are
     released anyway. Passing `signatures_released: true` for every request with a website's
     fee instead would make those requests strict and refuse Sage's own shapes that link the
     fee coin only by `ASSERT_CONCURRENT_SPEND` (a take of CATs for a CAT with a fee).
     **Spends Klimper signs but cannot read** (an unknown contract around the user's key: an
     NFT, a vault) count as the user's for binding and limits; what they create is never
     received (`outputs[].unaccounted`, `summary.unaccounted_spends`), and the approval shows
     them as a danger ("Coins Klimper can’t account for"). A multi-party request with one is
     refused (`4001` `unknown_partial`, explained to the website).
5. **Checked twice**: before the prompt, Sage's summary from the wallet database and
   wallet-kit's simulation must agree on the user's coins spent, the payments to others and
   the fee, or the request is refused (`crosscheck_mismatch`). A website's declared intent
   is verified claim by claim against the simulation; `chia_send` and `chia_takeOffer` carry
   one Klimper derives from the request itself.
6. **Only spends the signature fixes** (`sign_guard.rs`, TASK-141): a standard coin's key
   signs the spend's delegated puzzle, not the conditions it returns, so Klimper signs a
   standard spend — the coin's own puzzle, or the inner puzzle of a CAT, NFT, DID, option or
   other layer it can follow — only when its delegated puzzle is quoted conditions,
   `(q . conditions)`, and never by the hidden puzzle. Anything else could be completed with
   other conditions than the ones the approval showed, so it is refused before the user is
   asked (`4001`, reason `unsigned_conditions`), as is one of the wallet's standard puzzles
   anywhere in a spend in a shape Klimper cannot follow. Only the keys the request may sign
   with are looked at. It applies to `signCoinSpends` and `xchonnect_submitCoinSpends` (and
   their `chip0002_` aliases), and holds Sage's own builds for `chia_send` and
   `chia_takeOffer` to the same rule; for a take, only the spends Sage built from the user's
   coins, since the offer's own spends are the website's and a refusal of one would say
   something about the wallet's keys.
   The same guard limits `AGG_SIG_UNSAFE`, which a website can use only once the user allowed
   it: its signature carries no coin or network data, so a message is refused (`4001`, reason
   `unsafe_message`) when it has the shape of what another kind of signature covers — it ends
   with any of the network's `AGG_SIG` additional data (mainnet's and testnet11's are always
   included), it is 32 bytes long, the size of what `signMessage` signs, or it starts with
   `signMessage`'s framing. Other unsafe messages are asked about as before.

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
