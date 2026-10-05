# Xchonnect links: QR codes, universal links, same-device

How a pairing code or a request hint reaches Klimper (clapandpay **TASK-072**, **TASK-085**;
xchonnect spec 6.2 and 8.2, integration guide §5). The screens are
`docs/xchonnect-screens.md`.

## The three forms

| Form                                    | Arrives from                | Carries                      |
| --------------------------------------- | --------------------------- | ---------------------------- |
| `xchonnect:v1?r=…&s=…`                  | a QR code                   | the pairing URI's parameters |
| `https://clapandpay.com/pair#r=…&s=…`   | a universal link / app link | the same parameters          |
| `https://clapandpay.com/req#mbx=<hint>` | a universal link / app link | a mailbox hint, nothing more |

The domain is **clapandpay.com** (clapandpay decision-003 and decision-004). The spec's
examples say `klimper.app`; that domain is not ours, and the difference has to go back to
the Xchonnect spec and its dApp SDK wallet registry.

Parameters are in the **fragment** so they never reach a web server and never appear in a
server log. For `/pair` the fragment contains the pairing secret `s`, which is the
out-of-band secret that authenticates the whole handshake.

`src/klimper/xchonnectLink.ts` is the only parser, and `parseXchonnectLink` is total: a
failure is a result, not an exception, and the result is a code rather than the text that
failed. Three things follow from that, and each has a test:

- only `/pair` and `/req`, only on `clapandpay.com`, with no userinfo and no port. A QR
  code holding anything else keeps whatever meaning it already had elsewhere in the app;
- a refusal never carries the fragment. `redactLink` is the only way a link may appear in
  a message, and it prints the host and path with `#…`;
- nothing is logged. The fragment is not written to the console, to an error, or to the
  router.

## Why the link never travels in the app's URL

Klimper's router is `createHashRouter`, so the app's own URL _is_ a fragment. Handing it a
universal link's fragment would both break routing and write the pairing secret into the
history stack.

So a link is handed over in memory. `offerXchonnectLink(url)` in
`src/klimper/xchonnectLinkHandoff.ts` parses it, keeps it for exactly one read, and
navigates to `/xchonnect/connect` or `/xchonnect/request` — a bare route, with no
parameters. The screen calls `takeXchonnectLink()` once; a reload cannot replay a pairing
code.

That function is the single entry point. The QR scanner, the clipboard, a chosen image and
(once it exists) the deep-link listener all go through it.

## QR and clipboard entry, today

The pairing screen uses the app's existing `useScannerOrClipboard`, so entry matches the
rest of Klimper:

| Platform    | What the button does                                                            |
| ----------- | ------------------------------------------------------------------------------- |
| desktop     | reads the clipboard; or "choose a picture of the code", decoded locally by jsQR |
| iOS/Android | opens the camera scanner after asking for the camera permission                 |

Desktop is the demo target, and a desktop browser shows the QR on screen — so the code
usually arrives by clipboard or as a screenshot, both of which work now. The textarea
remains for a code read aloud or typed.

### The Scan tab

The pairing screen is no longer the only way in. **Scan** is a tab in the main navigation
(`src/klimper/Scanner.tsx`, behind `scanEnabled`, on by default): one camera for every code.
On a phone the tap opens the camera at once; on a computer the page offers the clipboard, a
picture and a text field. What was read is sorted by `classifyScan` (`scanTarget.ts`):

| Code                                            | Goes to                                     |
| ----------------------------------------------- | ------------------------------------------- |
| `xchonnect:v1?…`, `clapandpay.com/pair`, `/req` | `offerXchonnectLink`, so the connect screen |
| `xch1…`, `txch1…` (also behind `chia:`)         | Send for XCH, the address filled in         |
| `offer1…` (when offers are built in)            | the offer                                   |
| anything else                                   | shown on the page, to copy                  |

Anything that starts `xchonnect:` but cannot be used is reported without its text, for the
same reason the pairing screen never echoes a code. **Websites** sits beside it in the
navigation when `xchonnectEnabled`, so the connected websites are not only in Settings.

## What is still missing for a real universal link

Two pieces, and neither is in the webview.

### 1. The operating system has to hand us the URL

`tauri-plugin-deep-link` is not installed, so nothing delivers a `/pair` or `/req` URL to
the app. The wiring, when it is added:

```toml
# src-tauri/Cargo.toml
tauri-plugin-deep-link = "2"
```

```rust
// src-tauri/src/lib.rs, next to the other plugins
.plugin(tauri_plugin_deep_link::init())
```

```ts
// once, where the app starts
import { onOpenUrl } from '@tauri-apps/plugin-deep-link';
import { offerXchonnectLink } from '@/klimper/xchonnectLinkHandoff';

await onOpenUrl((urls) => urls.forEach(offerXchonnectLink));
```

`offerXchonnectLink` already handles a cold start (it stores the link and the screen reads
it on mount), a suspended app and a foreground app identically, which is what the
integration guide asks for. The plugin also needs the `deep-link:default` permission in
`src-tauri/capabilities/klimper.json`, main webview only.

### 2. The association has to be served

- **iOS.** `applinks:clapandpay.com` is in
  `src-tauri/gen/apple/sage-tauri_iOS/sage-tauri_iOS.entitlements`, next to the
  `webcredentials` entry the passkey work added. `docs/well-known/apple-app-site-association`
  is the file to serve from `https://clapandpay.com/.well-known/`, as
  `application/json`, with no redirect. It covers `/pair` and `/req` and nothing else, so
  every other path on the marketing site keeps opening in Safari. **`TEAMID` is a
  placeholder**: the Apple team id arrives with clapandpay TASK-003, and the
  `DEVELOPMENT_TEAM` in the Xcode project is still Sage's until then.
- **Android.** `docs/well-known/assetlinks.json` needs the release signing certificate's
  SHA-256 fingerprint, and the manifest needs an `android:autoVerify="true"` intent filter
  for `https://clapandpay.com` with `pathPrefix` `/pair` and `/req`. Neither is done: there
  is no Android SDK on the development machine.
- Hosting is clapandpay TASK-040, under the domain controls in TASK-058.

**This is not verifiable here.** Associated Domains only works on a device, against the
live file, with a provisioned team id — and Klimper's iOS build is blocked (TASK-009). The
entitlement and the files are configuration that a human with an iPhone and the hosting in
place has to confirm.

## Same-device round trip (TASK-085)

Spec 8.2: the dApp posts the request, opens `https://clapandpay.com/req#mbx=…`, Klimper
comes to the foreground, fetches its mailbox, shows the approval, posts the response and
returns the user to the dApp.

What works now: the `/req` path parses, routes to the approval screen, and that screen
calls `listSessions()` — which resumes every stored session's message loop, so the mailbox
is fetched. The `mbx` hint is deliberately **not** acted on: anyone can open that link, and
an approval exists only once an authenticated envelope produced one. With nothing pending
the screen says so neutrally and keeps syncing.

What is missing, both in the plugin rather than the screens:

- **The wallet's link base is not announced.** `pairing.rs` builds its `WalletMeta` with a
  name only. The dApp SDK needs `link: "https://clapandpay.com"` (the base; it appends
  `/req` itself, and it must be `https://` and under 256 bytes) or same-device requests
  fail with `no_wallet_link`. Two lines, in `reply()` and `confirm()`.
- **The return URL is not surfaced.** `VerifiedUri::return_url` exists, and the core
  already refuses an origin document whose `return_url` leaves the claimed domain — so
  pairing fails rather than handing the wallet a URL to police. But it is neither persisted
  with the session nor put on `SessionSummary`, so the webview has nothing to return to.
  `src/klimper/xchonnectReturn.ts` holds the guard and its tests: https only, byte-exact
  authority, no userinfo, no port, no trailing dot, checked against the session's verified
  ASCII domain. Until the field exists the approval screen does what the integration guide
  asks for in that case — it leaves the user with a clear "you can go back now" state
  rather than guessing a URL.
