# Bringing a Green Envelope wallet into Klimper

"Use my Clap and Pay money" on Klimper's start screen (`src/klimper/ClapAndPayConnect.tsx`)
brings the wallet from the Green Envelope page into Klimper. A passkey belongs to a website, so
the face or finger check happens on that page. This is handoff **version 2** (clapandpay
TASK-129); the protocol is `src/lib/clapandpayHandoff.ts`, a byte-for-byte copy of
`green-envelope/packages/handoff/src/receiver.ts`, and its background is the wiki's
`architecture/passkey-handoff.md`.

## How it goes

1. Klimper makes a one-time P-256 key pair (non-extractable, in memory) and a 16-byte nonce and
   opens `https://envelope.clapandpay.com/connect#v=2&k=<key>&n=<nonce>` in the browser.
2. The page says that Klimper will be able to spend everything, and asks for Face ID or the
   fingerprint on its button, never on load.
3. The page seals the wallet's **24 words** to Klimper's key (ECDH, HKDF bound to both keys and
   the nonce, AES-256-GCM) and offers one link: `https://clapandpay.com/klimper/return#cp2.…`.
   No copy button, no codes to compare.
4. The person taps it. `/klimper/return` is in the `applinks` of `clapandpay.com`
   (`green-envelope/apps/site/.well-known/apple-app-site-association`) and in Klimper's
   entitlements, so iOS gives the link to the signed Klimper app and to nothing else. Without
   Klimper the browser shows `apps/site/klimper-return.html`, which can read nothing.
5. The deep-link listener (`src/klimper/xchonnect/xchonnectDeepLink.ts`) routes the return path
   to `src/klimper/clapandpayReturn.ts`, where the waiting screen opens it: only with its own key
   and nonce, once, and within two minutes of the request. Then it is Sage's ordinary mnemonic
   import (`importKey` with the words) as an account named "Clap and Pay", and the offer to turn
   on Face ID (Sage's "Biometric Authentication").

A request made by anybody else (a phishing page, a chat bot) can still be approved by a person,
but the answer goes to Klimper, which never made that key and drops it. A wallet from before
recovery words has none to hand over; the page says so.

## The live page

Nothing to set up. Release builds hide the entry while `VITE_DISABLE_CLAPANDPAY_CONNECT=true` is
in `.env.production` (TASK-130 turns it on). The page's passkey belongs to `clapandpay.com`, on
every subdomain, so the same wallet comes back.

## Where it can be tried

- **On an iPhone**, with a Klimper build that has the `applinks:clapandpay.com` entitlement and
  the production `apple-app-site-association` (with `/klimper/return`) deployed. The return path
  is always on `clapandpay.com`, whatever page origin the build uses (`VITE_CLAPANDPAY_ORIGIN`).
- **Not on the Mac app**: a desktop build receives no universal links, so the answer never
  arrives. The whole round trip is covered on the Mac by `pnpm e2e:handoff` in `green-envelope/`
  instead: headless Chrome with a virtual passkey authenticator makes a wallet on a dev server,
  Klimper's own receiver makes the request, the page approves it, and the return link must open
  to the 24 words of the page's address, once, and for that request only.
