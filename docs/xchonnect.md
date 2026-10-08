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

## One decision per request; Klimper broadcasts

Decided 2026-10-05 (xchonnect-wiki `04-wallet-side-execution.md`): spec 11.1 item 4 is one
authentication per signing **request**, covering all of its signatures. The earlier
desktop reading — one confirmation per signature — asked 1 + 5 times for a five-coin send
and is gone.

1. **One approval** of the whole request, on the overlay or the requests page.
2. **One unlock**: `KlimperSigner::sign_all` opens the keychain once, derives every key the
   approved plan needs, signs and verifies everything, and drops the key material. With a
   passkey enrolled, `keychain.password()` fails with `Locked` until the passkey has opened
   it, and nothing is signed. A fresh Touch ID prompt per request on macOS is a follow-up:
   it needs Objective-C calls the workspace's `unsafe_code = "forbid"` rules out here.
3. **Klimper broadcasts** (`execute.rs`): `chia_send` is built by Sage from the wallet's
   own coins, `xchonnect_submitCoinSpends` carries a bundle the website built; both are
   signed once and sent through Klimper's own peers (`send_transaction_immediately`). The
   website receives `{ transactionId, status }`.
4. **Checked twice**: before the prompt, Sage's summary from the wallet database and
   wallet-kit's simulation must agree on the user's coins spent, the payments to others and
   the fee, or the request is refused (`crosscheck_mismatch`). A website's declared intent
   is verified claim by claim against the simulation; `chia_send` carries one Klimper
   derives from the request itself.

Key material lives only inside one `sign_all` call. The
keychain password is a `Zeroizing<Vec<u8>>`. `chia_bls::SecretKey` has neither `Drop` nor
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
