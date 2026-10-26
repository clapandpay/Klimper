# Upstream

> **Own repository (2026-10-24).** Klimper lived in `klimper/` of the clapandpay monorepo from
> 2026-09-20 until it moved here, with Sage's commits and ours intact. Upstream Sage has the same
> files at the same paths, so every path in this document is relative to the repository root
> and the sync script merges with a plain `git merge`. Never relocate files away from where
> upstream has them. Upstream's own workflows are in `.github/workflows/` and disabled here
> ("Sage's workflows", below); Klimper itself is not built in CI.

Klimper is a fork of Sage: https://github.com/xch-dev/sage (Apache-2.0, see `LICENSE-APACHE` and
`NOTICE`). Remote name `upstream`. Upstream is merged, never rebased.

|                     |                                                                                                                                                                                                                                                                                                                      |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Merged version      | **v0.13.1**, merged 2026-09-20                                                                                                                                                                                                                                                                                       |
| Hotfix merges since | 2026-10-23: `upstream/main` up to `63291148` (#888 sandbox tooltips, #893 WalletConnect `chia_updateCat`), an early merge of non-security commits by owner decision, before v0.13.2. `chia_updateCat` is unreachable in Klimper: WalletConnect is off (`VITE_DISABLE_WALLETCONNECT`) and Xchonnect does not grant it |
| Last checked        | 2026-10-23: `upstream/main` = `63291148` (v0.13.1 + 5), merged                                                                                                                                                                                                                                                       |
| Klimper version     | **1.0.0**, its own (`scripts/set-version.sh`)                                                                                                                                                                                                                                                                        |

**Klimper numbers its own releases; the version is not Sage's** (TASK-106).
`scripts/set-version.sh <major.minor.patch>` writes it to
`src-tauri/tauri.{macos,linux,windows,ios,android}.conf.json`, the committed iOS project
(`gen/apple/project.yml`, `Info.plist`) and `src/klimper/version.json`, and
`scripts/set-version.sh --check` fails when they disagree. That number is what the bundle, the
stores and TestFlight show. A Sage merge does not touch it.

The App Store Connect version records stay at **1.0** while TestFlight builds are 0.x (owner's
decision of 2026-10-07, TASK-109). App Store Connect attaches a build only to the version with
the same CFBundleShortVersionString, so the first store submission starts with
`scripts/set-version.sh 1.0.0` (or with renaming the record to the build's version).

Sage's app system (`crates/sage-apps`) checks an app's minimum Sage version against the running
app's version (`package_info().version`), not the engine's, so a Klimper number there made every
app that needs Sage 0.13 refuse to run ("You are running Sage 0.1.0"; TASK-063). So the running
app reports Sage's version instead: `context()` at the end of `src-tauri/src/lib.rs` sets it to
the version of the `sage-tauri` crate (`src-tauri/Cargo.toml`), which is upstream's line and
moves with each merge. `getVersion()` in the webview therefore answers with the Sage version;
Settings shows both, from `src/klimper/version.ts` ("Version 0.1.0 (Sage 0.13.1)"). Once
upstream compares against the engine version, `context()` can go.

## Routine

```sh
scripts/sync-upstream.sh --check    # what is new upstream, security-relevant paths listed
scripts/sync-upstream.sh            # merge the newest release tag
scripts/sync-upstream.sh <commit>   # hotfix: merge upstream/main up to a fix commit
```

And the other direction. The size of the next merge is exactly the number of shared files we
have touched, so that list is worth watching rather than remembering:

```sh
scripts/sync-upstream.sh --diff     # every file that differs from the merged tag, ours vs shared
scripts/sync-upstream.sh --export   # the flag work as a patch that applies to a clean Sage tree
```

`--diff` compares upstream's tree with ours directly, so every path it prints is spelled the
way Sage spells it. If it lists a shared file that the table below does
not, either the table is stale or the change belongs behind a flag — settle it then, not at the
next merge. `--export` writes the rows marked _propose upstream_ as one patch; it applies to a
clean checkout of the merged tag with `git apply`.

A fix that cannot be merged because the commits before it are not wanted yet:
`git cherry-pick -x <commit>` on a `hotfix/` branch; the next tag merge resolves it.

## Staying current

Pulling from Sage comes before pushing to Sage. A patch we carry (the flags, the iOS build
shims) costs some work at each merge; a Sage release we have not merged is a wallet, keychain
or bridge fix our users do not have. So we accept carrying patches, and we never delay a merge
waiting for upstream to take one.

**How we find out.** No clock runs anywhere (AGENTS.md, "CI"), so it is people who look:

- Watch `xch-dev/sage` on GitHub with _Custom → Releases_ and _Security alerts_. A release or an
  advisory then arrives as a notification.
- Run `scripts/sync-upstream.sh --check` at the start of every work week, and before every build
  that leaves the team. It lists the new upstream commits touching the watched paths: the wallet,
  keychain, apps and database crates, the engine, `src-tauri`, `tauri-plugin-sage` and the
  lockfiles.

**How fast.**

| What is new upstream                                                      | Merge within    | How                                                                                                                       |
| ------------------------------------------------------------------------- | --------------- | ------------------------------------------------------------------------------------------------------------------------- |
| A security fix or advisory, or a fix in a watched path marked as security | the same day    | `scripts/sync-upstream.sh <commit>` on `hotfix/sage-<short sha>`; `cherry-pick -x` if the commits before it are not ready |
| A release tag                                                             | one week        | "The upgrade, step by step" below, on `upstream/sage-<tag>`                                                               |
| Other commits on `upstream/main`                                          | at the next tag | nothing; releases carry them                                                                                              |

A hotfix goes through the same checks as an upgrade (step 5) and gets a row in "Hotfix merges
since" above. When a store build is in review, the fix still merges; the build is replaced.

**What makes a merge slow, and what to do about it.** Only the shared files in `--diff` can
conflict. Keep that list short: new code goes in files of our own, a shared file gets one import
and one element at most. Sage's engine and its key storage carry no change of ours: the keys
sit in Sage's `keys.bin` as upstream keeps them, and Sage's own "Biometric Authentication"
setting is what guards them (Klimper's passkey login was removed on 2026-10-09).

## The upgrade, step by step

Staying on the newest Sage is the point of the fork: the engine and the app bridge are theirs,
and a release we have not merged is a bridge fix we do not have. Do this on its own branch —
`upstream/sage-v0.13.2` — never mixed with other work, so a bad merge is one branch to drop.

1. **Look.** `scripts/sync-upstream.sh --check` — what is new, and which of it touches the
   watched paths. It lists only those paths, so it is not a changelog: a pure frontend fix will
   not appear, though the merge still brings it.
2. **Know what it will cost.** `scripts/sync-upstream.sh --diff`. The size of the merge is the
   number of shared files we have touched, and nothing else.
3. **Branch and merge.** `git switch -c upstream/sage-<tag>` then `scripts/sync-upstream.sh`,
   or `scripts/sync-upstream.sh <tag>` for a particular one. Never merge upstream by hand: the
   script also sets the `merge=ours` driver for the files that are wholly ours.
4. **Resolve** by the playbook below. A conflict in a file this document lists is ours to keep,
   line by line; a conflict anywhere else means we edited a shared file, and upstream wins while
   our change moves behind a flag or into a file of our own.
5. **Prove it still works**, in this order, cheapest first:

   ```sh
   pnpm install
   cargo fmt --all -- --files-with-diff --check
   cargo clippy --workspace --all-features --all-targets
   cargo test --workspace --all-features
   pnpm prettier:check && pnpm exec tsc -b && pnpm lint
   pnpm run build:frontend
   pnpm tauri:build
   ```

   Then the bridge smoke test by hand: install mempoolxch.space and sage-permission-probe, grant
   the capabilities, make one bridge call. CI will not catch a broken bridge.

6. **Check nothing of ours was quietly reverted.** An auto-merge can take upstream's side of a
   line we own without ever reporting a conflict:

   ```sh
   grep -h identifier src-tauri/tauri.*.conf.json   # com.clapandpay.klimper, every one
   scripts/set-version.sh --check                   # Klimper's version, not the merged tag
   grep -c 'fn context' src-tauri/src/lib.rs        # 1: apps are told the Sage version
   grep -c '^VITE_' .env.production                 # 12
   grep -c 'klimperRouter\|KlimperRoot' src/App.tsx # 3: the import, the router, the root
   ```

7. **Credit the people whose work this is.** `scripts/sync-upstream.sh --credits`, and update
   `NOTICE` if the list moved. Apache-2.0 asks a fork to keep attribution, and since Sage ships
   no `NOTICE` of its own, ours is the only place its authors are named. A new contributor
   upstream is a name missing from ours until someone regenerates it.
8. **Write it down.** The merged version in the table above (Klimper's own version stays as it
   is: a merge is not a Klimper release) and any new row in the file tables below — `--diff`
   tells you if one is missing.

## Hook points

`src/App.tsx` is where every Klimper feature used to land — a flag around seven routes, three
route groups, three elements, two imports — so any upstream change to Sage's route block was a
merge conflict. Since 2026-10-05 it differs from Sage's in **four lines**, and Sage's route
block is byte for byte upstream's:

```tsx
import { KlimperRoot, klimperRouter } from '@/klimper/KlimperRoot'; // + a comment line
const router = klimperRouter(createHashRouter)(   // was: createHashRouter(
        <KlimperRoot />                           // inside <I18nProvider>
```

- `src/klimper/routes.tsx` — `withKlimper(routes)`: a feature switched off in `features.ts`
  turns its route into a redirect, looked up **by path** in one table (`GATED`,
  `GATED_CHILDREN`); Klimper's own screens (`/xchonnect/*`, `/scanner`, `/card`) are
  appended. A new upstream route needs nothing; a new gated feature is one table line.
  `klimperRouter(create)` wraps Sage's own factory, so neither the indentation of the block
  nor Sage's imports change. Tested in `routes.test.tsx`.
- `src/klimper/KlimperRoot.tsx` — everything rendered next to the app (the Xchonnect
  approval overlay, the coin sound) and the `klimperTheme` import.
- `src/main.tsx` — two lines (TASK-105): `<App />` is wrapped in `AppRecovery`
  (`src/klimper/KlimperErrorBoundary.tsx`). Sage has no error boundary, so one component
  that throws while rendering unmounts everything and leaves a black window. With it, a
  thrown render shows a recovery screen with "Back to my wallet" and "Reload";
  `withKlimper` gives every route the same screen as its `errorElement`, and `KlimperRoot`
  puts the approval overlay behind a boundary of its own.

Sage's toast container is outside `I18nProvider`: content passed to `toast.*` uses the `t`
macro, never the `Trans` component, which throws there (TASK-105: the "sent" toast blacked
out the app after every transaction a website asked for).

New Klimper screens and app-wide pieces go there, never into `App.tsx`. The rest of the shared
frontend diff is single elements wrapped in a flag (`Nav.tsx`, `Settings.tsx`,
`TokenList.tsx`, `Login.tsx`) and the @handle field in `Send.tsx` — all rows marked _propose
upstream_ below, so they leave the diff once Sage takes the hiding points.

## Our files (the whole diff against upstream)

New files never conflict. Files shared with Sage are kept to the lines listed here.

Each shared file we changed also carries one line near its top, in its own comment syntax:
`Modified by Klimper (2026): see docs/UPSTREAM.md` (Apache-2.0 §4(b)). That line and the
tables below together are Klimper's record of its modifications. Files that cannot take a
comment, or that a tool rewrites, are recorded by the tables alone: the `package.json`s,
`Cargo.toml`s, `Cargo.lock`, `pnpm-lock.yaml`, `tauri*.conf.json`, the `Info.plist`s and
entitlements, `project.pbxproj`, the `messages.po` catalogues and the images. A new edit to a
shared file adds the line; a file that becomes upstream's again loses it. `--diff` also lists
files that differ from the merged tag only because a hotfix merge brought upstream's own later
commits (see the table at the top): those are not ours and carry no line.

| File                                                                                                                                          | Ours                                                          | Note                                                                                                                                                                                                                                                                                                 |
| --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `README.md`, `LICENSE`                                                                                                                        | whole file                                                    | `.gitattributes` `merge=ours`; the script sets the driver                                                                                                                                                                                                                                            |
| `LICENSE-APACHE`, `NOTICE`, `deny.toml`, `LICENSE-APP-STORE-EXCEPTION.md`, `TRADEMARK.md`, `CONTRIBUTING.md`, `SECURITY.md`, `.gitattributes` | new                                                           |                                                                                                                                                                                                                                                                                                      |
| `docs/UPSTREAM.md`, `scripts/sync-upstream.sh`, `scripts/testflight.sh`, `.github/workflows/*.yml` (repository root)                          | new                                                           |                                                                                                                                                                                                                                                                                                      |
| `src-tauri/tauri.macos.conf.json`, `tauri.linux.conf.json`, `tauri.windows.conf.json`                                                         | new                                                           | Tauri merges the platform file over `tauri.conf.json`: product name, identifier, version, window title. `tauri.conf.json` itself is untouched. The window block is a copy (JSON merge patch replaces arrays): compare it with upstream at each merge                                                 |
| `src-tauri/tauri.ios.conf.json`, `tauri.android.conf.json`                                                                                    | 3 lines added, and `bundle.iOS`                               | product name, version, identifier; our Apple team `HTP35W93ZF` (`tauri.conf.json` keeps Sage's)                                                                                                                                                                                                      |
| `src-tauri/gen/apple/project.yml`, `sage-tauri.xcodeproj/project.pbxproj`, `sage-tauri_iOS/Info.plist`                                        | bundle id; team; iOS 15.0 minimum; product name and version   | `DEVELOPMENT_TEAM` is ours, `HTP35W93ZF` (TASK-003): take ours on a conflict. Xcode 27 refuses the iOS 14.0 minimum. `tauri ios build` writes the product name and version from `tauri.ios.conf.json` into the pbxproj and Info.plist on every build, so those lines are committed as it writes them |
| `src-tauri/gen/android/app/build.gradle.kts`, `.gitignore`, `res/values/strings.xml`, `MainActivity.kt` (moved to `com/clapandpay/klimper/`)  | package and app name                                          | not yet built, no Android SDK on the dev machine (2026-09-17)                                                                                                                                                                                                                                        |
| `scripts/set-version.sh`, `src/klimper/version.json`, `src/klimper/version.ts`                                                                | new                                                           | Klimper's own version (TASK-106, see the top): the script writes it, Settings reads it                                                                                                                                                                                                               |
| `src-tauri/src/lib.rs`, `src/pages/Settings.tsx`                                                                                              | `context()` appended and one call changed; one import changed | the running app reports Sage's version, which the app system checks apps against; Settings takes `getVersion` from `src/klimper/version.ts`                                                                                                                                                          |

### Reduced interface (TASK-007)

Klimper shows the wallet primitives only: assets, send, receive, addresses, coins, transactions,
peers, settings, and the Apps screen with its bridge approval dialogs. The rest is switched off
with build flags that extend Sage's own `VITE_DISABLE_OFFERS / OPTIONS / SWAP` pattern; nothing
is deleted, the engine and the bridge keep every capability, and every flag defaults to Sage's
behaviour.

| File                                                                          | Ours                          | Note                                                                                                                                                                        |
| ----------------------------------------------------------------------------- | ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `.env.production`, `.env.example`                                             | new                           | the release flags (`vite build` reads `.env.production`, which wins over a developer's `.env`) and the development template; no secrets, those go in `.env.local`           |
| `src/lib/featureElement.tsx`                                                  | new                           | route gate: a disabled feature redirects to `/wallet`                                                                                                                       |
| `src/lib/features.ts`, `src/vite-env.d.ts`                                    | seven flags appended          | NFTs, NFT minting, profiles, token issuing, WalletConnect, RPC settings, theme gallery                                                                                      |
| `src/klimper/routes.tsx` (`GATED`, `GATED_CHILDREN`)                          | new                           | offers, options, swap, NFTs, NFT minting, profiles, token issuing and the theme gallery redirect by path when off; `src/App.tsx` is only the hook point (see "Hook points") |
| `src/components/Nav.tsx`, `src/pages/TokenList.tsx`, `src/pages/Settings.tsx` | guards around existing blocks |                                                                                                                                                                             |

These seven files are what `scripts/sync-upstream.sh --export` writes out, and they are meant to
stop being ours: propose them to Sage as one pull request ("more build-time feature flags, and
gate the routes"). They are not the whole frontend diff — the feature below is ours to keep.

### Our own features, behind their own flags

Added the same way and just as removable, but nothing Sage would want: it is Clap and Pay's,
not a wallet primitive. It defaults to off in `features.ts`, so a build without `.env` is Sage's
behaviour.

| File                                                                                  | Ours                        | Note                                                                                                                                                                                                                                               |
| ------------------------------------------------------------------------------------- | --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/lib/handles.ts`, `src/hooks/useHandle.ts`, `src/components/HandleResolution.tsx` | new                         | `@anna` instead of an address, through XCHandles (mainnet only). The address is taken from api.xchandles.com as it answers (owner, 2026-10-08): replace these with Sage's verified resolver when it lands upstream (xch-dev/sage #867, #870, #871) |
| `src/pages/Send.tsx`                                                                  | +60 lines, `handlesEnabled` | the largest edit to a shared file; the first thing to re-apply by hand on a conflict                                                                                                                                                               |
| `src/components/TransferDialog.tsx`                                                   | +32 lines, `handlesEnabled` | `@anna` for NFT, DID and option transfers; shares `recheckHandle` with Send. Sage plans it (#871)                                                                                                                                                  |
| `package.json`                                                                        | `workspaces` array added    | for bun; pnpm reads `pnpm-workspace.yaml`. Drop it if bun goes away                                                                                                                                                                                |

### Coins (TASK-124)

| File                                               | Ours                                    | Note                                                                                                    |
| -------------------------------------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `src/components/OwnedCoinsCard.tsx`                | imports, an effect, the submit handlers | coins locked in an offer can be combined and split after a warning; the coin pouch shares the selection |
| `src/components/AmountCell.tsx`                    | one import, the label                   | privacy mode hides the amount from the accessibility label too                                          |
| `crates/sage-wallet/src/wallet/coin_management.rs` | one test appended                       | combining a coin locked in an offer cancels the offer; a test only, no engine change                    |

### Phone data and push

Klimper's keys are Sage's: `keys.bin` under Sage's empty password, guarded by Sage's own
"Biometric Authentication" setting. The passkey login that once sealed them was removed on
2026-10-09 (TASK-126); a phone that had it enrolled re-imports its recovery words. The one
deliberate difference from Sage: the wallet data never goes into a phone backup, because a
`keys.bin` under the empty password protects nothing once it leaves the phone. On iOS
`tauri-plugin-klimper-storage` flags the app data directory `isExcludedFromBackup` at start;
Android turns backup and device transfer off in the manifest. Check on a phone after a
release build with a wallet in it: Settings → [name] → iCloud → Manage Account Storage →
Backups → this iPhone lists Klimper at a few KB, not the size of its wallet databases; a
failure is logged as `could not keep … out of backups` (success as `… is kept out of
backups`), about 15 seconds after start, once Sage's logging is up. Sage writes `keys.bin` in
place (`fs::write`), as upstream; with no backup, a write torn by a crash leaves only the
recovery words, which is accepted for engine parity.

| File                                                                                                                                             | Ours                             | Note                                                                                                                                                                               |
| ------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tauri-plugin-klimper-storage/`, `src-tauri/src/lib.rs` (one `.plugin()` line)                                                                   | new, 1 line                      | iOS: the app data directory is excluded from iCloud and device backups (TASK-057, TASK-126). No commands, nothing in the webview                                                   |
| `src-tauri/gen/android/app/src/main/AndroidManifest.xml`, `res/xml/data_extraction_rules.xml`                                                    | 2 attributes, new                | no backup, no device transfer: the wallet data never leaves the phone (TASK-057)                                                                                                   |
| `src-tauri/gen/apple/sage-tauri_iOS/sage-tauri_iOS.entitlements`, `src-tauri/gen/android/{build.gradle.kts,app/build.gradle.kts,app/.gitignore}` | 4 lines, 1 line, 4 lines, 1 line | push wake-ups (TASK-087): `aps-environment` and time-sensitive notifications; the `google-services` Gradle plugin, applied only when `google-services.json` exists. `docs/push.md` |
| `src-tauri/gen/apple/sage-tauri_iOS/Info.plist`                                                                                                  | usage strings                    | `NSFaceIDUsageDescription` says what Sage's biometric check guards: payments and the recovery words                                                                                |

### iOS build on Xcode 27 (TASK-003)

Under Xcode 27 swift-rs makes each Swift package's own `@_cdecl` functions public again, but
not its own runtime (`retain_object`, `release_object`, `string_from_bytes`, `data_from_bytes`),
which every package archive embeds, so no iOS app links. Our copy of swift-rs promotes that
runtime in the Tauri archive alone. `scripts/testflight.sh` carries the build settings Xcode 27
needs on top (unstripped build-time macros, `llvm-tools`).

| File                       | Ours                          | Note                                                                                                                                                                          |
| -------------------------- | ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `vendor/swift-rs/`         | new: swift-rs 1.0.8, 1 change | `src-rs/build.rs`, the "Klimper patch" comment. Drop the directory and the patch line once a swift-rs release exports its runtime; propose it upstream                        |
| `Cargo.toml`, `Cargo.lock` | 2 lines; swift-rs from a path | the `swift-rs` line under `[patch.crates-io]`; on a lock conflict keep the path source                                                                                        |
| `Cargo.lock`               | `swift-rs` 1.0.7 → 1.0.8      | 1.0.7 builds Swift for macOS under Xcode 27, so no iOS build works; on a conflict take upstream's lock, then `cargo update -p swift-rs` unless upstream is already past 1.0.7 |
| `.prettierignore`          | 1 line                        | `vendor`                                                                                                                                                                      |

### iOS release build (TASK-003)

Apps linked against the iOS 27 SDK must use the UIScene life cycle or UIKit refuses to launch
them; tao 0.34 (Sage's pinned Tauri) has none. A shim of our own bridges it until tao 0.37.

| File                                                                                | Ours                   | Note                                                                                                |
| ----------------------------------------------------------------------------------- | ---------------------- | --------------------------------------------------------------------------------------------------- |
| `src-tauri/gen/apple/Sources/sage-tauri/KlimperScene.m`, `src-tauri/Info.ios.plist` | new                    | the scene delegate and its manifest; drop both once Sage's Tauri uses tao 0.37 or later             |
| `src-tauri/gen/apple/sage-tauri.xcodeproj/project.pbxproj`                          | `KlimperScene.m`       | registered as a source                                                                              |
| `src-tauri/tauri.ios.conf.json`                                                     | `bundle.iOS.infoPlist` | points Tauri at `Info.ios.plist`, which it does not pick up by itself                               |
| `src/locales/*/messages.po`                                                         | regenerated            | `pnpm extract` after a merge: a production build shows a message id, not text, for anything missing |

### Look: Klimper Coin (TASK-062)

The design guide (Klimper Coin) is `docs/design-guide.md`. It is applied as two themes and one stylesheet scoped
to them. The themes are named `light` and `dark`, so they replace Sage's core themes of that name;
Sage's other themes stay selectable and unchanged, except Colorful, which is deleted.

| File                                                                                                             | Ours                  | Note                                                                                                                                                       |
| ---------------------------------------------------------------------------------------------------------------- | --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/design-guide.md`, `src/themes/klimper-light/`, `src/themes/klimper-dark/`                                  | new                   | ordinary theme-o-rama themes named `light` and `dark`: theme-o-rama keys themes by name, so they replace its built-ins                                     |
| `src/lib/klimperTheme.ts`, `src/klimper.css`                                                                     | new                   | default theme once per install; ink sidebar, pill buttons, coins, total band, focus, 44px controls, motion                                                 |
| `src/klimper/KlimperRoot.tsx`                                                                                    | new                   | `import '@/lib/klimperTheme'`, evaluated before the `ThemeProvider` reads the saved theme                                                                  |
| `src/icon-dark.png`, `src/icon-light.png`, `src-tauri/icons/*`, the iOS `AppIcon.appiconset`, Android `mipmap-*` | replaced              | the Klimper logo; every app icon is regenerated by `scripts/update-logo.sh` (new, ours). Take ours on a conflict                                           |
| `src-tauri/gen/android/app/src/main/res/values/ic_launcher_background.xml`                                       | one colour            | set by `scripts/update-logo.sh` from the logo's corner                                                                                                     |
| `src/themes/colorful/`, `src/components/ThemeSelector.tsx`                                                       | deleted, one fallback | Colorful is gone, and with it the picker's fallback to it; `klimperTheme.ts` sends an install that had it back to the default. On a merge, keep it deleted |

### Xchonnect dApp connections

Klimper answers CHIP-0002 requests from outside websites over Xchonnect (decision-008,
milestone m-4). Same shape as the other features: one crate of our own, screens of our own,
and single-line hooks in shared files. `docs/xchonnect.md` has the detail.

| File                                                                                                                                          | Ours                                                                                     | Note                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| --------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tauri-plugin-klimper-xchonnect/`, `src/klimper/xchonnect*.{ts,tsx}`, `useXchonnect*.ts`                                                      | new                                                                                      | the plugin, the typed command wrappers, the screens and their tests. `docs/xchonnect-screens.md` and `docs/xchonnect-links.md` are the design and the entry points                                                                                                                                                                                                                                                                                                                                                        |
| `Cargo.toml`, `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`                                                                                  | workspace member, dep, one plugin                                                        | three lines                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `src-tauri/capabilities/klimper.json`                                                                                                         | one permission                                                                           | `klimper-xchonnect:default`, main webview only — app webviews must never reach it                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `src/klimper/routes.tsx`                                                                                                                      | new                                                                                      | `xchonnectRoutes` appended by `withKlimper`; the routes live in `src/klimper/xchonnect/XchonnectRoutes.tsx`, each behind `featureElement`; the approval overlay is in `KlimperRoot`                                                                                                                                                                                                                                                                                                                                       |
| `src/pages/Settings.tsx`                                                                                                                      | one import, one element                                                                  | `<XchonnectSettings />`, which renders nothing when the flag is off. The only entry point: before it, `/xchonnect` was reachable by typed URL alone                                                                                                                                                                                                                                                                                                                                                                       |
| `src/components/Nav.tsx`                                                                                                                      | one import, one element                                                                  | `<KlimperNavLinks />`: the Scan tab (`scanEnabled`) and Websites (`xchonnectEnabled`); every flag is checked inside it                                                                                                                                                                                                                                                                                                                                                                                                    |
| `src/lib/features.ts`, `src/vite-env.d.ts`                                                                                                    | one flag                                                                                 | `{scannerRoutes}` (`/scanner`, `src/klimper/Scanner.tsx`) and `VITE_DISABLE_SCAN`                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `src-tauri/gen/apple/sage-tauri_iOS/sage-tauri_iOS.entitlements`                                                                              | 1 line                                                                                   | `applinks:clapandpay.com`. Serve `docs/well-known/apple-app-site-association` (TASK-040)                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `src-tauri/Info.plist`                                                                                                                        | 2 lines                                                                                  | `NSCameraUsageDescription`: the desktop pairing screen reads QR codes from the camera (`src/klimper/QrCamera.tsx`)                                                                                                                                                                                                                                                                                                                                                                                                        |
| `src-tauri/Info.ios.plist`, `capabilities/klimper.json`, `lib.rs`, both `Cargo.toml`, `package.json`                                          | `CFBundleURLTypes`, permission, plugin, deps                                             | the `xchonnect` URL scheme and `tauri-plugin-deep-link` `~2.4` (the last line for Tauri 2.10; TASK-072.01). No `plugins.deep-link` in any Tauri conf: its build script would rewrite the hand-maintained iOS entitlements                                                                                                                                                                                                                                                                                                 |
| `tauri-plugin-klimper-jingle/`, `src/klimper/sound/`, `scripts/klimper-sound.py`, `docs/sound.md`                                             | new                                                                                      | the Klimper sound (decision-009, TASK-111): coin jingle, haptics, shake; behind `VITE_ENABLE_KLIMPER_COINS` (with the coin pouch, `src/klimper/pouch/`) and a Settings switch. Its `feel` command is the slide-to-approve haptic (`src/klimper/haptics.ts`)                                                                                                                                                                                                                                                               |
| `tauri-plugin-klimper-card/`, `src/klimper/card/`, `docs/card.md`, `preview-card.html`, `src/preview/card*.ts*`                               | new                                                                                      | the card (milestone card-payment), from the issuer named by `KLIMPER_CARD_ISSUER` (Gnosis Pay first): plugin, screens, tests, browser preview. Behind `VITE_ENABLE_CARD`; one `.plugin()` line in `src-tauri/src/lib.rs`, one permission (`klimper-card:default`) in `capabilities/klimper.json`, one route in `routes.tsx`, one entry in `KlimperNavLinks.tsx`                                                                                                                                                           |
| `KlimperRoot.tsx`, `pages/Settings.tsx`, `src/klimper/KlimperSettings.tsx`, `lib.rs`, `Cargo.toml` x2, `klimper.json`                         | lazy element, one element first in Settings, plugin, dep, permission, flag               | the hook points of the sound (and the flag in `.env*`); `KlimperSettings` is Klimper's own card at the top of Settings: 'Enable coins sound and feel' and privacy mode                                                                                                                                                                                                                                                                                                                                                    |
| `src/components/NumberFormat.tsx`, `src/components/ui/dialog.tsx`, `src/klimper/privacy.tsx`                                                  | 1 line each; new                                                                         | privacy mode: `NumberFormat` shows ••••• while it is on, and every dialog (confirmations included) shows amounts again (`RevealAmounts` around `DialogContent`'s children)                                                                                                                                                                                                                                                                                                                                                |
| `src/pages/TokenList.tsx`, `src/klimper/WalletTotal.tsx`                                                                                      | the total block replaced by one element at the top; new                                  | the total balance as a card on top of the assets, with its XCH value; 'an estimate' behind an info dot                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `pages/TokenList.tsx` (`hasTokens` on the total), `src/klimper/WalletTotal.tsx`, `src/klimper/card/useCardBalance.ts`, `src/klimper/price.ts` | one prop on the existing element; new                                                    | the card in the wallet's total (TASK-158): wallet and card together, split beneath, top-ups on the way, a link to top up; the euro rate from the price source. Only with `VITE_ENABLE_CARD` and a signed-in card                                                                                                                                                                                                                                                                                                          |
| `pages/Token.tsx`, `pages/TokenList.tsx`, `src/klimper/pageValue.ts`                                                                          | one `usePageValue` line each; new                                                        | the shake weighs what the open page shows: the pages publish the dollar value they already compute (decision-009, TASK-111); the registry is outside the sound chunk so the import is a few lines                                                                                                                                                                                                                                                                                                                         |
| `src/lib/features.ts`, `src/vite-env.d.ts`, `.env`                                                                                            | one flag                                                                                 | `VITE_ENABLE_XCHONNECT`, on in `.env.production`, so release builds include it; unset means off                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `src/pages/Login.tsx`, `index.html`                                                                                                           | 1 line each                                                                              | the welcome headline and the page title say Klimper, not Sage (App Review 2.3: the name the user sees is the app's)                                                                                                                                                                                                                                                                                                                                                                                                       |
| `src-tauri/Info{,.ios}.plist`, `gen/apple/{project.yml,sage-tauri.xcodeproj,sage-tauri_iOS}`, `PrivacyInfo.xcprivacy`                         | purpose strings, entitlements, privacy manifest, device family                           | no photo-library string; display name, Face ID and NFC strings in `Info.ios.plist`; no NFC entitlement (Offers is off); the unused Group Activities entitlement stays, since tauri-plugin-sharesheet's build script adds it on every Mac build; iPhone only, portrait; product name Klimper and team HTP35W93ZF in `project.yml` and `project.pbxproj`; the privacy manifest (required-reason APIs, the push token as its one collected item) is registered in `project.pbxproj` (TASK-109)                               |
| `src/contexts/PriceContext.tsx`, `src/klimper/price.ts`                                                                                       | three fetches became one call; new                                                       | one price source (TASK-113): nodexch.com `/api/prices` on mainnet (TASK-112), CoinGecko + dexie as the fallback, 0 USD on testnet; the sound reads the same prices. On a conflict keep upstream's structure and re-add the one call                                                                                                                                                                                                                                                                                       |
| `src/pages/Transaction.tsx`, `src/klimper/explorer.ts`                                                                                        | 3 imports, 3 lines, one button                                                           | "View on mempoolxch.space": the transaction's block on the explorer (a wallet transaction is one block height)                                                                                                                                                                                                                                                                                                                                                                                                            |
| `components/AssetCoin.tsx`, `components/CoinList.tsx`, `pages/{Nft,Option,CollectionMetaData}.tsx`, `lib/urls.ts`                             | one import and the link each; Spacescan helpers and logo removed                         | every explorer link goes to mempoolxch.space (`src/klimper/explorer.ts`): coins to `/coin/0x…`, the NFT button to `/nft/0x<launcher id>`, the option's coin likewise. Collections lose their Spacescan button, since the explorer has no collection page; MintGarden's stays, shown only with offers on (TASK-138). On a merge, take upstream's Spacescan changes into these spots as mempoolxch.space links                                                                                                              |
| `components/AssetCoin.tsx`, `pages/Transaction.tsx`, `src/klimper/{CoinAddress,AddressText}.tsx`                                              | an optional `children` slot rendered under the coin; one element per coin and one import | the transaction page shows each coin's address (Sage has `address` and `address_kind` per coin and showed neither): shortened, the whole on a tap, a copy button, and whose it is (Yours, Burned, Offer, Launcher, Other). The explorer row wraps on a phone (`flex-wrap`). `AddressText` is also the address in website requests                                                                                                                                                                                         |
| `pages/TokenList.tsx`, `src/klimper/{MyHandles.tsx,ownedHandles.ts}`                                                                          | one import, one element under the total                                                  | the wallet's XCHandles as chips that copy `@handle`: the NFTs of the XCHandles collection in the wallet, each checked against `api.xchandles.com` (expired ones marked, leftovers whose name now points elsewhere dropped). Mainnet and `VITE_ENABLE_HANDLES` only                                                                                                                                                                                                                                                        |
| `pages/{Nft,NftList}.tsx`, `components/{NftCard,NftGroupCard,MultiSelectActions}.tsx`                                                         | one import, conditions around existing elements                                          | NFTs on, minting and the rest off (TASK-138): "Mint NFT" behind `nftMintingEnabled`; "Assign/Edit Profile" behind `profilesEnabled`; the "View on MintGarden" buttons behind `offersEnabled` (no route to buying outside in-app purchase, guideline 3.1.1). On a merge, keep upstream's elements and put the conditions back around them                                                                                                                                                                                  |
| `src-tauri/tauri.conf.json`                                                                                                                   | `security.csp`                                                                           | a Content Security Policy for the main webview (scripts only from the app; images, media and fonts from the web; fetch over https, IPC and the WalletConnect relay only; no frames, plugins or form posts; TASK-114). `withGlobalTauri` stays on: Sage's app sandbox probe (`crates/sage-apps/src/sandbox/probes/isolation.rs`) and the app SDKs use `window.__TAURI__`. Desktop dev loads `devUrl`, so no CSP there. Sage apps set their own CSP in `crates/sage-apps`. _Propose upstream_; on a conflict keep the block |
| `vite.preview.config.ts`, `preview-*.html`, `src/preview/`                                                                                    | new                                                                                      | the approval screens rendered with mock data in a plain browser, for screenshots and headless checks; never part of `vite build` (own config)                                                                                                                                                                                                                                                                                                                                                                             |
| `.prettierignore`                                                                                                                             | 1 line                                                                                   | the plugin's generated permissions                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `Cargo.lock`                                                                                                                                  | the plugin and its dependencies                                                          | `keyring` 3, and `xchonnect-core` / `xchonnect-wallet-kit` from crates.io (see below). On a conflict take upstream's lock and re-run `cargo check`                                                                                                                                                                                                                                                                                                                                                                        |

The two xchonnect crates come from crates.io at an exact pre-release version
(`=0.1.0-rc.2`, clapandpay TASK-071 AC #1), so the workspace resolves on a machine without
an xchonnect checkout. Cargo never selects a pre-release for a plain requirement, and the
kit requires the core at exactly its own version: bump both lines in
`tauri-plugin-klimper-xchonnect/Cargo.toml` together, then `cargo check` to refresh
`Cargo.lock`. The xchonnect checkout is still needed for the development relay and
gateway (`scripts/push-dev.sh`, `docs/xchonnect.md`), not for building Klimper.

Nothing under `crates/` (one test aside, see "Coins"), `crates/sage-apps` or `packages/sage-app-sdk` is touched. The
plugin reads the Sage database through `sage-database`'s public API and signs through
`sage-keychain`, the same way `crates/sage/src/endpoints/wallet_connect.rs` does.

Identifier: `com.clapandpay.klimper`. Klimper therefore has its own data directory and never
opens a Sage wallet's data. Crate, package and binary names stay `sage-*`.

Known leftovers, deliberately not patched:

- `crates/sage-cli` and `crates/sage-client` look for the RPC certificates in the
  `com.rigidnetwork.sage` data directory. Propose an override upstream before relying on the CLI.

## Conflict playbook

1. Conflict in one of the files above: keep our lines, take everything else from upstream.
2. Conflict anywhere else means we edited a shared file. Take upstream, then re-apply our change
   as a flag, a theme value or a file of our own; if the hiding point is missing, propose it
   upstream first.
3. Never resolve anything under `crates/sage-apps` or `packages/sage-app-sdk` in our favour.

## CI

Upstream's workflows (`build.yml`, `npm.yml`) stay in the tree unchanged so they never conflict,
but they need Sage's runners and secrets and would publish to npm. Disable them once in the
GitHub repository: `gh workflow disable Build && gh workflow disable NPM`. Ours are
`.github/workflows/green-envelope.yml` and `clapandpay-web.yml`, named `Image …` so that command
does not touch them.

## Sage's workflows

`.github/workflows/build.yml` and `npm.yml` are Sage's. They stay in the tree, because the sync
merges them, but GitHub would run them on every push, and `npm.yml` would publish Sage's
packages. They are disabled in this repository; a fresh fork disables them before its first push
of a branch:

```sh
gh workflow disable Build
gh workflow disable NPM
```
