# Passkey login

Klimper opens the wallet with a passkey instead of a password. The passkey never signs
transactions; it unlocks the keychain that holds the recovery words and keys.

## How it works

- Sage encrypts every secret in `keys.bin` with Argon2 + AES-256-GCM, using the empty
  password. With a passkey, Klimper re-encrypts them with a random 32-byte **wallet key**.
- The wallet key is sealed in `passkey.json` next to `keys.bin`: AES-256-GCM under
  HKDF-SHA256 of the passkey's **PRF output** (WebAuthn `prf` extension, iOS 18), with the
  version, relying party, credential id and PRF salt bound as associated data.
- The PRF output only exists after Face ID on a device that holds the passkey. It goes from
  Swift (`AuthenticationServices`) straight into Rust; the webview never sees it or the
  wallet key.
- The keychain starts locked, and it locks again whenever Klimper leaves the screen. Until
  it's unlocked, every endpoint that needs a secret returns `Unauthorized`.
- The recovery words stay the backup. A new phone either restores its iCloud backup (both
  files come back, and the synced passkey opens them) or imports the recovery words.

Code: `tauri-plugin-klimper-passkey` (envelope, commands, Swift), `src/klimper/`
(lock screen and settings), and the keychain password in `crates/sage-keychain` (the one
engine change, proposed upstream).

## What a release needs

1. **Apple team** (TASK-003): the App ID `com.clapandpay.klimper` with the Associated
   Domains capability. The entitlement `webcredentials:clapandpay.com` is already in
   `src-tauri/gen/apple/sage-tauri_iOS/sage-tauri_iOS.entitlements`.
2. **`https://clapandpay.com/.well-known/apple-app-site-association`**, served as
   `application/json` without redirects (TASK-005):

   ```json
   { "webcredentials": { "apps": ["<TEAMID>.com.clapandpay.klimper"] } }
   ```

3. **iOS 18** or later on the device. On older iOS the settings entry shows, but creating
   the passkey fails with a clear message. Desktop and Android don't show the entry.

## Testing on a device

Settings → Passkey → Set up passkey → Face ID (once or twice, depending on whether the
provider returns the PRF at registration). Close and reopen Klimper: the lock screen asks
for Face ID. Remove passkey also asks for Face ID and returns to Sage's empty password.
