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

`src/klimper/xchonnect/xchonnectLink.ts` is the only parser, and `parseXchonnectLink` is total: a
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
`src/klimper/xchonnect/xchonnectLinkHandoff.ts` parses it, keeps it for exactly one read, and
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
(`src/klimper/Scanner.tsx`, behind `scanEnabled`, on by default): one camera for every code,
and nothing else: no clipboard, no picture, no text field. The camera opens as soon as the
tab is shown, on a phone and on a computer alike. It is the webview's own camera
(`getUserMedia`), decoded frame by frame by the open-source `jsQR` in `QrCamera.tsx`; the
tab does not use the native barcode-scanner plugin. On iOS this needs
`NSCameraUsageDescription` in `src-tauri/Info.plist`. What was read is sorted by
`classifyScan` (`scanTarget.ts`):

| Code                                            | Goes to                                     |
| ----------------------------------------------- | ------------------------------------------- |
| `xchonnect:v1?…`, `clapandpay.com/pair`, `/req` | `offerXchonnectLink`, so the connect screen |
| `xch1…`, `txch1…` (also behind `chia:`)         | Send for XCH, the address filled in         |
| `offer1…` (when offers are built in)            | the offer                                   |
| anything else                                   | shown on the page, to copy                  |

Anything that starts `xchonnect:` but cannot be used is reported without its text, for the
same reason the pairing screen never echoes a code. **Websites** sits beside it in the
navigation when `xchonnectEnabled`, so the connected websites are not only in Settings.

## How a link reaches the app (TASK-072.01)

```
Safari / a dApp's "open in wallet"
  → KlimperSceneDelegate (gen/apple/Sources/sage-tauri/KlimperScene.m)
    scene:openURLContexts:, scene:continueUserActivity:, and the URLContexts /
    userActivities of a cold start's connection options
  → tao's application delegate: application:openURL:options:,
    application:continueUserActivity:restorationHandler:
  → Tauri RunEvent::Opened { urls }
  → tauri-plugin-deep-link: stores the URLs (getCurrent) and emits deep-link://new-url
  → src/klimper/xchonnect/xchonnectDeepLink.ts: listenForXchonnectLinks (mounted once by KlimperRoot)
  → offerXchonnectLink → the connect or request screen
```

Two forms arrive, and any website can use either; Klimper allowlists nobody and verifies
the dApp's origin document during pairing instead:

| Form                                     | Registered by                                     | Needs hosting |
| ---------------------------------------- | ------------------------------------------------- | ------------- |
| `xchonnect:v1?r=…&s=…` (the pairing URI) | `CFBundleURLTypes` in `src-tauri/Info.ios.plist`  | no            |
| `https://clapandpay.com/pair`, `/req`    | `applinks:clapandpay.com` in the iOS entitlements | yes, the AASA |

The scheme is what makes "connect to mobile" work today: the dApp SDK opens the pairing URI
itself on a device where no wallet has announced a link base (Pengui's
`openPairingInWallet`), iOS asks "Open in Klimper?", and the connect screen runs
`inspectPairingUri` on the handed-over code at once, so the user's next tap is Approve and
then the SAS comparison. A universal link skips the question but needs the association file
(below).

Three rules of the wiring:

- **The plugin's own config is not used.** `plugins.deep-link` in a Tauri conf file makes
  the plugin's build script rewrite `gen/apple/sage-tauri_iOS/sage-tauri_iOS.entitlements`
  and `Info.plist` on every iOS build; it would replace `com.apple.developer.associated-domains`
  with whatever the plugin config lists. The scheme and the domains are therefore
  hand-maintained in `Info.ios.plist` and the entitlements, and `docs/UPSTREAM.md` lists both.
- **`WalletMeta.link` is still not announced** (`WALLET_LINK` in `pairing.rs`). Once a
  dApp SDK has seen it, it opens `<link>/pair#…` and `<link>/req#…` for every later pairing
  and request; until the AASA is served those go to Safari and a dead site, so announcing it
  early would break the second pairing on a phone. It becomes `Some("https://clapandpay.com")`
  with TASK-085 when the file is live.
- **The listener never reads a URL.** `listenForXchonnectLinks` subscribes before it reads the
  launch URL, delivers a link that came through both paths at start-up once, and hands
  everything to `offerXchonnectLink`, which parses, redacts and routes. Outside a Tauri window
  the plugin has no IPC and the listener is a no-op.

The webview needs `deep-link:default` (`get_current`) in `src-tauri/capabilities/klimper.json`,
main webview only; the event comes with `core:event`.

On a device: with the TestFlight build installed, a tap on `xchonnect:v1?…` in Safari (or on
Pengui's "connect to mobile") has to open Klimper on the connect screen with the website's
domain shown. That is the owner's check; it cannot be made on this Mac (no simulator builds,
and Associated Domains only resolve on a device against the live file).

Android has the parser and the handoff but no delivery: `tauri-plugin-deep-link` would take
`plugins.deep-link.mobile` in `tauri.android.conf.json` for the manifest's intent filters,
which only an Android build can verify.

## What is still missing for a real universal link

### 2. The association has to be served

- **iOS.** `applinks:clapandpay.com` is in
  `src-tauri/gen/apple/sage-tauri_iOS/sage-tauri_iOS.entitlements`.
  `clapandpay-web/public/.well-known/apple-app-site-association`
  is the file to serve from `https://clapandpay.com/.well-known/` (the clapandpay-web image does), as
  `application/json`, with no redirect. It covers `/pair` and `/req` and nothing else, so
  every other path on the marketing site keeps opening in Safari. The Apple team id is in
  the file (TASK-005); check that the `DEVELOPMENT_TEAM` in the Xcode project is ours too.
- **Android.** `clapandpay-web/public/.well-known/assetlinks.json` needs the release signing certificate's
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
  `src/klimper/xchonnect/xchonnectReturn.ts` holds the guard and its tests: https only, byte-exact
  authority, no userinfo, no port, no trailing dot, checked against the session's verified
  ASCII domain. Until the field exists the approval screen does what the integration guide
  asks for in that case — it leaves the user with a clear "you can go back now" state
  rather than guessing a URL.
