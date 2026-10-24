# The Klimper card (Gnosis Pay)

A Visa card that pays from the user's own money. The card is issued through
[Gnosis Pay](https://docs.gnosispay.com). Its balance sits in a Safe account on Gnosis
Chain, and only the wallet's recovery words control that Safe. Klimper is software only.
Identity checks, card issuing and e-money stay with Gnosis Pay and its licensed partners,
and the card contract is between the user and them.

Status: MVP behind a build flag, off by default. It is tested against a fake Gnosis Pay.
Using it against Gnosis Pay's sandbox needs a registration, and production needs a
partnership (see "What a build needs").

## What it does

| Step         | Where                                                      | Notes                                                                                                   |
| ------------ | ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Card address | `tauri-plugin-klimper-card/src/evm.rs`                     | An Ethereum key from the wallet's own 24 words on `m/44'/60'/0'/0/0`; derived when needed, never stored |
| Sign in      | `src/siwe.rs`, `src/gnosis_pay.rs`                         | Gnosis Pay's SIWE message, checked before it is signed; session in memory                               |
| Onboarding   | `src/onboarding.rs`, `src/klimper/card/CardOnboarding.tsx` | e-mail code, terms, identity check (Sumsub, in the browser), source of funds, Safe, phone, virtual card |
| Account      | `src/account.rs`, `src/klimper/card/CardAccount.tsx`       | balance, the card, payments; freeze and unfreeze                                                        |
| Wallet total | `src/klimper/WalletTotal.tsx`, `card/useCardBalance.ts`    | on top of Assets: wallet and card together, then each on its own line, top-ups on the way, "Top up"     |

The card number, expiry and CVV are shown on the device (see "Card details and Apple
Pay"). Not in the MVP: in-app 3-D Secure, withdrawals and top-ups from Chia, and keeping
the sign-in across restarts.

**The wallet's total** (TASK-158). The total on top of Assets adds what the card can spend
to the wallet's value. The card's euros count at the current rate, taken from the same
price source as XCH (nodexch's XCH in USD over XCH in EUR). Beneath the total, one line
shows the wallet and one the card, in the card's own currency, with top-ups still on the
way. "Top up" opens the card screen, where the card account's address is. With no euro
rate, the total stays the wallet's. The total never signs in by itself: signed out, the
card line only links to the card screen. It reads the card every 30 s while Assets is
open, and when Klimper comes back to the front.

## Privacy and security rules

- **No Klimper server.** Every call goes from the device to Gnosis Pay with the user's own
  session. Gnosis Pay's webhooks need a partner backend, so Klimper polls while the screen
  is open instead.
- **Tokens stay in Rust and in memory.** The webview never sees a token, a key or the
  recovery words. Nothing about the card is written to disk. A restart means signing in
  again, which is one signature.
- **Klimper signs only a sign-in.** Gnosis Pay builds the SIWE message on its server.
  Klimper signs it only when it names this build's domain and URI, the card address, Gnosis
  Chain, a plain nonce and an issue time within five minutes, with no resources and no
  unknown lines. Every signature is checked to recover to the card address before it
  leaves `evm.rs`.
- **Identity data stays between the user and Gnosis Pay.** Klimper never calls
  `GET /user`, which returns name, date of birth and address. E-mail, phone and answers
  are passed through and not kept. The identity check opens in the system browser, and
  only from an https link on `sumsub.com`. A rejection passes on no reason.
- **Payments are reduced in Rust** to merchant name, amount, currency, state and time.
  Location, coordinates, MCC and scheme details never reach the webview. Privacy mode hides
  the amounts.
- **No logs with personal data.** Error bodies are cut down to the HTTP status and Gnosis
  Pay's short error code. Addresses, amounts, e-mail, phone and tokens are not logged.
- **Chia and the card are not linked by Klimper.** The card key is secp256k1 on a separate
  path. Klimper never sends a Chia address to Gnosis Pay.
- **The owner is for good.** Gnosis Pay makes the sign-in address the Safe's only owner,
  and it cannot be changed. Losing the recovery words means losing the card account, as for
  the wallet. An address can belong to one Gnosis Pay user across all partners.
- **The Safe's Delay module is never touched.** Gnosis Pay blocks the card on any use of
  it.

## Card details and Apple Pay

The number, expiry and CVV come through Gnosis Pay's direct PCI route. No backend is
needed:

1. `GET /pci/cards/{id}/challenge?action=view-details` returns EIP-712 typed data.
   `eip712::check_card_challenge` accepts only a Gnosis Pay `Message` with an
   authorization text and a nonce, on this build's chain, with no `verifyingContract` and
   no `salt`. Nothing of that shape can move money. Only then does the card key sign it,
   after the biometric prompt.
2. Klimper makes a fresh AES-256 key and wraps it for `GET /pci/public-key`
   (RSA-OAEP-SHA256).
3. `GET /pci/cards/{id}/details` with `x-eip712-signature` and `x-eip712-nonce` returns
   the number and CVV in AES-GCM under that key. They are decrypted in Rust and shown once.
   They are hidden after 60 s, by hand, or when Klimper goes to the background.

**Apple Pay today** is the route every Visa card has. The user opens the Wallet app, taps
+, chooses Debit or Credit Card, and enters the details shown in Klimper. The card issuer
then verifies the card. Google Wallet works the same way on Android. Copying the number
clears the clipboard after a minute.

**One-tap "Add to Apple Wallet"** (in-app provisioning with
`PKAddPaymentPassViewController`) needs, all from outside Klimper:

- The `com.apple.developer.payment-pass-provisioning` entitlement. Apple grants it to the
  card issuer's app, or to an app the issuer sponsors, after an application. It works
  only in TestFlight and App Store builds.
- An issuer API that takes Apple's certificates, nonce and nonce signature and returns
  `encryptedPassData`, `activationData` and `ephemeralPublicKey`. Gnosis Pay documents no
  such API (2026-10).
- Apple's "Add to Apple Wallet" button guidelines, and App Review of the flow.

Until then Klimper does not show Apple's badge, which is reserved for that button. The
question is with Gnosis Pay (TASK-147). The Google Pay equivalent (Push Provisioning API)
has the same shape.

## What a build needs

The card settings are public, not secrets: the app name and the domain appear in every
sign-in message. They are still not in this repository. A Gnosis Pay registration belongs
to one partner, and forks need their own.

| Variable (Rust build environment) | Meaning                                                                     |
| --------------------------------- | --------------------------------------------------------------------------- |
| `KLIMPER_CARD_APP_NAME`           | the app name Gnosis Pay registered                                          |
| `KLIMPER_CARD_DOMAIN`             | the SIWE domain Gnosis Pay whitelisted for it                               |
| `KLIMPER_CARD_URI`                | the SIWE URI; `https://<domain>` if unset                                   |
| `KLIMPER_CARD_API`                | the user API; Gnosis Pay's sandbox if unset. Release builds take https only |
| `KLIMPER_CARD_CHAIN_ID`           | the chain in the sign-in message; 100 (Gnosis Chain) if unset               |

| Variable (webview, `.env`) | Meaning                                                                                    |
| -------------------------- | ------------------------------------------------------------------------------------------ |
| `VITE_ENABLE_CARD=true`    | the `/card` route and the navigation entry. Off: no route, no entry, no call to the plugin |

A build with the flag on but without the Rust settings shows "No card in this build".

Access, as Gnosis Pay documents it (2026-10):

- **Sandbox:** by invitation, with the domain whitelisted. No agreement is needed.
- **Production:** only with a partnership agreement.
- **Card numbers through the secure elements:** a partner backend holding an mTLS
  certificate.

## Running it locally

Tests, with the fake Gnosis Pay in-process:

```sh
cargo test -p tauri-plugin-klimper-card
pnpm vitest run src/klimper/card
```

The screens in a browser, with a mocked plugin (no Rust, no network):

```sh
npx vite --config vite.preview.config.ts
# open http://localhost:5199/preview-card.html?state=signin
# states: signin register terms kyc review sof account phone order active frozen
#         nobuild nowords rejected; add &dark=1 for the dark theme
```

The desktop app against the fake server:

```sh
cargo run -p tauri-plugin-klimper-card --features fake --example fake-gnosis-pay
# in another shell (debug builds accept http):
KLIMPER_CARD_API=http://127.0.0.1:8787/user-api KLIMPER_CARD_APP_NAME=klimper-dev \
  KLIMPER_CARD_DOMAIN=localhost VITE_ENABLE_CARD=true pnpm tauri dev
```

The fake's codes are always `123456`. To move it along:

- `curl -X POST http://127.0.0.1:8787/fake/kyc-approved`: the identity check passes.
- `curl -X POST http://127.0.0.1:8787/fake/onboarded`: skips straight to an active card
  with money.
- `curl -X POST http://127.0.0.1:8787/fake/pay -H 'content-type: application/json' -d
'{"merchant":"Bakery","amount":"4.20"}'`: makes a card payment.

## Open questions for Gnosis Pay

These are written down where the code guesses. Each guess is marked in the code.

- The exact fields of the server-built SIWE message (the chain id, an expiry).
- What `GET /user/onboarding` answers before registration. Klimper reads a 404 or
  `USER_NOT_FOUND` as "register".
- Which body `POST /user/account` takes on Gnosis Chain. Klimper sends `{}`.
- The statement's amount fields. Klimper tries `billingAmount`, `amount` and
  `transactionAmount`.
- Whether a wallet may use the direct `/pci` endpoints to show the card number, the
  AES-GCM layout (tag appended assumed), and the status of a bad challenge (403 assumed:
  a 401 reads as an expired session).
- In-app provisioning to Apple Pay and Google Pay: whether Gnosis Pay or its issuer
  (Monavate) offers the provisioning API and sponsors the Apple entitlement.
- The withdrawal path (`/user/withdrawal` or `/user/withdrawals`), and the currency of the
  account limit.
