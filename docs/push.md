# Push wake-ups

A phone is never connected to a relay. When a website sends a request, the relay asks
Klimper's own **push gateway** for a content-free wake-up, and Klimper then reads its
mailbox (clapandpay TASK-086 gateway, TASK-087 app). Without push, requests are only seen
while Klimper is open; the app says so when the user refuses notifications.

```
website ── relay ──▶ gateway ──▶ APNs (iPhone) / FCM (Android) ──▶ Klimper ──▶ relay (read mailbox)
```

The notification carries nothing: the app never reads it, and shows a fixed "New signing
request". Only the relay's answer is acted on.

## What is where

| Part                                               | Where                                                                                                       |
| -------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| Sealing the token, registering it, renewing it     | `tauri-plugin-klimper-xchonnect/src/push.rs`, `session_loop.rs` (`ensure_push`)                             |
| iOS: permission, APNs token, wake-ups              | `tauri-plugin-klimper-xchonnect/ios/Sources/KlimperXchonnectPlugin.swift`                                   |
| Android: permission, Firebase token, notification  | `tauri-plugin-klimper-xchonnect/android/` (`KlimperXchonnectPlugin.kt`, `KlimperMessagingService.kt`)       |
| What the user sees when notifications are refused  | `src/klimper/useXchonnectPush.ts`, `XchonnectDapps.tsx`                                                     |

The permission is asked at the **first confirmed pairing**, not at launch. Registration is
per session, with a fresh sealed token, so the relay cannot link two sessions of one phone.

## Build settings

Both are compiled in. Unset, push is off and the app polls.

| Variable                    | Meaning                                                           |
| --------------------------- | ----------------------------------------------------------------- |
| `KLIMPER_PUSH_GATEWAY_URL`  | the gateway's `https` wake URL, as the relay will call it         |
| `KLIMPER_PUSH_GATEWAY_KEY`  | the gateway's X25519 public key, base64url (`GET /v1/keys`)       |

`tauri-plugin-klimper-xchonnect/build.rs` takes them from the environment, else from the
first of two files in `klimper/`, `KEY=VALUE` a line: `.push-dev.env` (gitignored; a local
stack, written by `scripts/push-dev.sh`) and **`push-gateway.env`** (committed; the deployed
gateway). The files exist because Xcode runs cargo without the caller's environment: a
variable exported before `pnpm tauri ios dev` or `scripts/testflight.sh` never reaches the
Rust build of a phone app, and the first simulator build was made with push off that way.
Both values are public, so the deployed ones belong in `push-gateway.env`; fill it in once
the gateway runs (TASK-086). The build prints a warning when it finds neither.

## iOS

- The App ID `com.clapandpay.klimper` needs **Push Notifications** and **Time Sensitive
  Notifications**; the entitlements are in `sage-tauri_iOS.entitlements`.
- The gateway needs an APNs key (`.p8`, Apple developer account → Keys), mounted as a file.
  This is not the App Store Connect key.
- Debug builds register with the APNs sandbox, release builds with production.

## Android

- Create a Firebase project and add the Android app `com.clapandpay.klimper`. Download
  `google-services.json` into `src-tauri/gen/android/app/` (it is gitignored). Without it the
  app builds and polls.
- The gateway needs the project's service account JSON (`XCHONNECT_GATEWAY_FCM_SERVICE_ACCOUNT_FILE`).
- Android 13 and later asks for the notification permission at the first pairing.
- The Kotlin side has not been built yet (no Android SDK on the dev machine).

## Testing it locally, without Apple

`scripts/push-dev.sh` starts Xchonnect's relay and gateway on loopback and the example
website, writes the two build settings to `.push-dev.env`, and prints the steps: build for
the simulator with `pnpm tauri ios dev "iPhone 17"` (debug builds accept a loopback `http`
gateway), pair with the website, send a request. The relay then calls the gateway, which
opens the sealed token; without an Apple key it has nobody to hand it to, so the script
delivers the same notification to the simulator with `xcrun simctl push`. With the key in
the environment (`XCHONNECT_GATEWAY_APNS_TEAM_ID`, `_KEY_ID`, `_KEY_FILE`) the gateway sends
to Apple's sandbox, which a simulator on an Apple-silicon Mac receives for real.

The same chain without a screen is the live test
`a_request_wakes_the_phone_through_the_gateway` in
`tauri-plugin-klimper-xchonnect/tests/live_relay.rs`; `scripts/e2e.sh` in the Xchonnect
repository runs it with the other Klimper live tests.

## Running the pieces

The relay and the gateway are in the Xchonnect repository (`deploy/compose.yaml`, gateway
behind `--profile gateway`; `docs/operating.md`).

1. Relay and gateway reachable over public HTTPS (a tunnel for local testing).
2. The relay's `XCHONNECT_GATEWAY_ALLOWLIST` lists the gateway's URL.
3. `XCHONNECT_GATEWAY_KEYS` set on the gateway; the public key goes into
   `KLIMPER_PUSH_GATEWAY_KEY`.
4. `XCHONNECT_GATEWAY_TEST_PLATFORM=true` accepts tokens without delivering anything: it tests
   registration and the allowlist before any Apple or Firebase credential exists.
5. Pengui points at the same relay (`NEXT_PUBLIC_XCHONNECT_RELAY_URL`).
