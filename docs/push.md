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

Both are read at compile time. Unset, push is off and the app polls.

| Variable                    | Meaning                                                           |
| --------------------------- | ----------------------------------------------------------------- |
| `KLIMPER_PUSH_GATEWAY_URL`  | the gateway's `https` wake URL, as the relay will call it         |
| `KLIMPER_PUSH_GATEWAY_KEY`  | the gateway's X25519 public key, base64url (`GET /v1/keys`)       |

`scripts/testflight.sh` reads them from `~/.appstoreconnect/klimper.env`. Keep one value pair
per target: local (a tunnel to your Mac) and deployed. Nothing else differs between the two.

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
