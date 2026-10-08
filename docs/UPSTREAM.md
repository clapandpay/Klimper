# Upstream

> **Monorepo (2026-09-20).** This fork lives in `klimper/` of the clapandpay repository, while
> upstream Sage has the same files at its root. Every path in this document is relative to
> `klimper/`. The sync script merges with `git merge -X subtree=klimper`, which shifts upstream's
> paths into this folder; a trial merge of `upstream/main` on the day of the move touched 103
> files, all inside `klimper/`, with no conflicts. Never merge upstream by hand without that
> option, and never relocate files inside `klimper/`. Upstream's own workflows sit in
> `klimper/.github/`, where GitHub does not run them; ours is `/.github/workflows/images.yml` (named `Images`, so the disable command below leaves it alone).

Klimper is a fork of Sage: https://github.com/xch-dev/sage (Apache-2.0, see `LICENSE-APACHE` and
`NOTICE`). Remote name `upstream`. Upstream is merged, never rebased.

|                     |                                                    |
| ------------------- | -------------------------------------------------- |
| Merged version      | **v0.13.1**, merged 2026-09-20                     |
| Hotfix merges since | none                                               |
| Last checked        | 2026-10-08: `upstream/main` = v0.13.1, nothing new |
| Klimper version     | **0.1.0**, its own (`scripts/set-version.sh`)      |

**Klimper numbers its own releases; the version is not Sage's** (TASK-106).
`scripts/set-version.sh <major.minor.patch>` writes it to
`src-tauri/tauri.{macos,linux,windows,ios,android}.conf.json`, the committed iOS project
(`gen/apple/project.yml`, `Info.plist`) and `src/klimper/version.json`, and
`scripts/set-version.sh --check` fails when they disagree. That number is what the bundle, the
stores and TestFlight show. A Sage merge does not touch it.

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

`--diff` compares upstream's tree with our `klimper/` subtree directly, so every path it prints
is already spelled the way Sage spells it. If it lists a shared file that the table below does
not, either the table is stale or the change belongs behind a flag — settle it then, not at the
next merge. `--export` writes the rows marked _propose upstream_ as one patch; it applies to a
clean checkout of the merged tag with `git apply`.

A fix that cannot be merged because the commits before it are not wanted yet:
`git cherry-pick -x <commit>` on a `hotfix/` branch; the next tag merge resolves it.

## Staying current

Pulling from Sage comes before pushing to Sage. A patch we carry (the keychain password, the
flags) costs some work at each merge; a Sage release we have not merged is a wallet, keychain
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
and one element at most. The keychain password is the exception, because a flag cannot switch it
off. It touches `crates/sage-keychain` and the places the empty password used to be passed
(`crates/sage/src/sage.rs`, `error.rs`, `utils/spends.rs`, `endpoints/{actions,keys,offers,
wallet_connect}.rs`). When one of those conflicts:

1. Take upstream's version of the file.
2. Re-apply our change, which is always the same: where Sage passes `b""` as the keychain
   password, ask the keychain for it (`self.keychain.password()?`), and map a locked keychain to
   `Unauthorized`.
3. `cargo test -p sage-keychain -p tauri-plugin-klimper-passkey`, then
   `grep -rn 'b""' crates/sage/src`. The grep must find no keychain call: a new call site from
   upstream that still passes the empty password would fail silently once a passkey is set.

Proposing the keychain change to Sage stays worth doing, because it would make those merges
free. But it is optional, and no upgrade waits for it.

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
   or `scripts/sync-upstream.sh <tag>` for a particular one. It merges with
   `-X subtree=klimper`; never merge upstream by hand without that.
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
   grep -c '^VITE_' .env.production                 # 13
   grep -c 'klimperRouter\|KlimperRoot' src/App.tsx # 3: the import, the router, the root
   grep -rn 'b""' crates/sage/src                   # nothing: new upstream call sites merge
                                                    # cleanly but pass the empty password
   grep -c 'Passkey' src/pages/Settings.tsx         # 2
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
  `GATED_CHILDREN`); Klimper's own screens (`/clapandpay`, `/xchonnect/*`, `/scanner`) are
  appended. A new upstream route needs nothing; a new gated feature is one table line.
  `klimperRouter(create)` wraps Sage's own factory, so neither the indentation of the block
  nor Sage's imports change. Tested in `routes.test.tsx`.
- `src/klimper/KlimperRoot.tsx` — everything rendered next to the app (`PasskeyGate`, the
  Xchonnect approval overlay) and the `klimperTheme` import.
- `src/main.tsx` — two lines (TASK-105): `<App />` is wrapped in `AppRecovery`
  (`src/klimper/KlimperErrorBoundary.tsx`). Sage has no error boundary, so one component
  that throws while rendering unmounts everything and leaves a black window. With it, a
  thrown render shows a recovery screen with "Back to my wallet" and "Reload";
  `withKlimper` gives every route the same screen as its `errorElement`, and `KlimperRoot`
  puts the approval overlay behind a boundary of its own (the passkey gate deliberately not:
  a failed gate must not leave the wallet open).

Sage's toast container is outside `I18nProvider`: content passed to `toast.*` uses the `t`
macro, never the `Trans` component, which throws there (TASK-105: the "sent" toast blacked
out the app after every transaction a website asked for).

New Klimper screens and app-wide pieces go there, never into `App.tsx`. The rest of the shared
frontend diff is single elements wrapped in a flag (`Nav.tsx`, `Settings.tsx`,
`TokenList.tsx`, `Login.tsx`) and the @handle field in `Send.tsx` — all rows marked _propose
upstream_ below, so they leave the diff once Sage takes the hiding points.

## Our files (the whole diff against upstream)

New files never conflict. Files shared with Sage are kept to the lines listed here.

| File                                                                                                                                         | Ours                                                          | Note                                                                                                                                                                                                                                                                                                 |
| -------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `README.md`, `LICENSE`                                                                                                                       | whole file                                                    | `.gitattributes` `merge=ours`; the script sets the driver                                                                                                                                                                                                                                            |
| `LICENSE-APACHE`, `NOTICE`, `CLA.md`, `COMMERCIAL-LICENSE.md`, `TRADEMARK.md`, `CLAUDE.md`, `.gitattributes`                                 | new                                                           |                                                                                                                                                                                                                                                                                                      |
| `docs/UPSTREAM.md`, `scripts/sync-upstream.sh`, `scripts/testflight.sh`, `.github/workflows/images.yml`                                      | new                                                           |                                                                                                                                                                                                                                                                                                      |
| `src-tauri/tauri.macos.conf.json`, `tauri.linux.conf.json`, `tauri.windows.conf.json`                                                        | new                                                           | Tauri merges the platform file over `tauri.conf.json`: product name, identifier, version, window title. `tauri.conf.json` itself is untouched. The window block is a copy (JSON merge patch replaces arrays): compare it with upstream at each merge                                                 |
| `src-tauri/tauri.ios.conf.json`, `tauri.android.conf.json`                                                                                   | 3 lines added, and `bundle.iOS`                               | product name, version, identifier; our Apple team `HTP35W93ZF` (`tauri.conf.json` keeps Sage's)                                                                                                                                                                                                      |
| `src-tauri/gen/apple/project.yml`, `sage-tauri.xcodeproj/project.pbxproj`, `sage-tauri_iOS/Info.plist`                                       | bundle id; team; iOS 15.0 minimum; product name and version   | `DEVELOPMENT_TEAM` is ours, `HTP35W93ZF` (TASK-003): take ours on a conflict. Xcode 27 refuses the iOS 14.0 minimum. `tauri ios build` writes the product name and version from `tauri.ios.conf.json` into the pbxproj and Info.plist on every build, so those lines are committed as it writes them |
| `src-tauri/gen/android/app/build.gradle.kts`, `.gitignore`, `res/values/strings.xml`, `MainActivity.kt` (moved to `com/clapandpay/klimper/`) | package and app name                                          | not yet built, no Android SDK on the dev machine (2026-09-17)                                                                                                                                                                                                                                        |
| `scripts/set-version.sh`, `src/klimper/version.json`, `src/klimper/version.ts`                                                               | new                                                           | Klimper's own version (TASK-106, see the top): the script writes it, Settings reads it                                                                                                                                                                                                               |
| `src-tauri/src/lib.rs`, `src/pages/Settings.tsx`                                                                                             | `context()` appended and one call changed; one import changed | the running app reports Sage's version, which the app system checks apps against; Settings takes `getVersion` from `src/klimper/version.ts`                                                                                                                                                          |

### Reduced interface (TASK-007)

Klimper shows the wallet primitives only: assets, send, receive, addresses, coins, transactions,
peers, settings, and the Apps screen with its bridge approval dialogs. The rest is switched off
with build flags that extend Sage's own `VITE_DISABLE_OFFERS / OPTIONS / SWAP` pattern; nothing
is deleted, the engine and the bridge keep every capability, and every flag defaults to Sage's
behaviour.

| File                                                                          | Ours                          | Note                                                                                                                                                              |
| ----------------------------------------------------------------------------- | ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `.env.production`, `.env.example`                                             | new                           | the release flags (`vite build` reads `.env.production`, which wins over a developer's `.env`) and the development template; no secrets, those go in `.env.local` |
| `src/lib/featureElement.tsx`                                                  | new                           | route gate: a disabled feature redirects to `/wallet`                                                                                                             |
| `src/lib/features.ts`, `src/vite-env.d.ts`                                    | six flags appended            | NFTs, profiles, token issuing, WalletConnect, RPC settings, theme gallery                                                                                         |
| `src/klimper/routes.tsx` (`GATED`, `GATED_CHILDREN`)                          | new                           | offers, options, swap, NFTs, profiles, token issuing and the theme gallery redirect by path when off; `src/App.tsx` is only the hook point (see "Hook points")    |
| `src/components/Nav.tsx`, `src/pages/TokenList.tsx`, `src/pages/Settings.tsx` | guards around existing blocks |                                                                                                                                                                   |

These seven files are what `scripts/sync-upstream.sh --export` writes out, and they are meant to
stop being ours: propose them to Sage as one pull request ("more build-time feature flags, and
gate the routes"). They are not the whole frontend diff — the two features below are ours to
keep.

### Our own features, behind their own flags

Added the same way and just as removable, but nothing Sage would want: they are Clap and Pay's,
not a wallet primitive. Both default to off in `features.ts`, so a build without `.env` is Sage's
behaviour.

| File                                                                                  | Ours                                 | Note                                                                                                                                                                                                                                               |
| ------------------------------------------------------------------------------------- | ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/lib/handles.ts`, `src/hooks/useHandle.ts`, `src/components/HandleResolution.tsx` | new                                  | `@anna` instead of an address, through XCHandles (mainnet only). The address is taken from api.xchandles.com as it answers (owner, 2026-10-08): replace these with Sage's verified resolver when it lands upstream (xch-dev/sage #867, #870, #871) |
| `src/pages/Send.tsx`                                                                  | +44 lines, `handlesEnabled`          | the largest edit to a shared file; the first thing to re-apply by hand on a conflict                                                                                                                                                               |
| `src/components/TransferDialog.tsx`                                                   | +32 lines, `handlesEnabled`          | `@anna` for NFT, DID and option transfers; shares `recheckHandle` with Send. Sage plans it (#871)                                                                                                                                                  |
| `src/klimper/ClapAndPayConnect.tsx`, `src/lib/clapandpayHandoff.ts`                   | new                                  | "Use my Clap and Pay money": the passkey wallet in one Face ID                                                                                                                                                                                     |
| `src/pages/Login.tsx`                                                                 | +9 lines, `clapAndPayConnectEnabled` | one button on the start screen                                                                                                                                                                                                                     |
| `.prettierignore`                                                                     | 1 line                               | `src/lib/clapandpayHandoff.ts`, which is generated                                                                                                                                                                                                 |
| `package.json`                                                                        | `workspaces` array added             | for bun; pnpm reads `pnpm-workspace.yaml`. Drop it if bun goes away                                                                                                                                                                                |

### Passkey login

Klimper opens the keychain with a passkey (see `docs/passkey.md`). One engine change, proposed
upstream; the rest is files of our own and one-line hooks.

| File                                                                                                                                             | Ours                                     | Note                                                                                                                                                                               |
| ------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/sage-keychain`, `crates/sage/src/{sage.rs,error.rs,endpoints/*,utils/spends.rs}`                                                         | keychain password                        | the one engine change: lock, unlock, re-key instead of the empty password                                                                                                          |
| `tauri-plugin-klimper-passkey/`, `src/klimper/`, `src-tauri/capabilities/klimper.json`                                                           | new                                      | envelope, commands, Swift; lock screen and settings entry                                                                                                                          |
| `Cargo.toml`, `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`                                                                                     | workspace member, dep, one plugin        |                                                                                                                                                                                    |
| `Cargo.lock`                                                                                                                                     | the plugin, and `swift-rs` 1.0.7 → 1.0.8 | 1.0.7 builds Swift for macOS under Xcode 27, so no iOS build works; on a conflict take upstream's lock, then `cargo update -p swift-rs` unless upstream is already past 1.0.7      |
| `src/pages/Settings.tsx`                                                                                                                         | one import and one element               | `<PasskeySettings />`; `<PasskeyGate />` is in `src/klimper/KlimperRoot.tsx`                                                                                                       |
| `src-tauri/gen/apple/sage-tauri_iOS/sage-tauri_iOS.entitlements`, `.prettierignore`                                                              | 4 lines, 1 line                          | `webcredentials:clapandpay.com`; the plugin's generated permissions                                                                                                                |
| `src-tauri/gen/android/app/src/main/AndroidManifest.xml`, `res/xml/data_extraction_rules.xml`                                                    | 2 attributes, new                        | no backup, no device transfer: the wallet data never leaves the phone (TASK-057)                                                                                                   |
| `src-tauri/gen/apple/sage-tauri_iOS/sage-tauri_iOS.entitlements`, `src-tauri/gen/android/{build.gradle.kts,app/build.gradle.kts,app/.gitignore}` | 4 lines, 1 line, 4 lines, 1 line         | push wake-ups (TASK-087): `aps-environment` and time-sensitive notifications; the `google-services` Gradle plugin, applied only when `google-services.json` exists. `docs/push.md` |

### iOS build on Xcode 27 (TASK-003)

Under Xcode 27 swift-rs makes each Swift package's own `@_cdecl` functions public again, but
not its own runtime (`retain_object`, `release_object`, `string_from_bytes`, `data_from_bytes`),
which every package archive embeds, so no iOS app links. Our copy of swift-rs promotes that
runtime in the Tauri archive alone. `scripts/testflight.sh` carries the build settings Xcode 27
needs on top (unstripped build-time macros, `llvm-tools`).

| File                       | Ours                          | Note                                                                                                                                                   |
| -------------------------- | ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `vendor/swift-rs/`         | new: swift-rs 1.0.8, 1 change | `src-rs/build.rs`, the "Klimper patch" comment. Drop the directory and the patch line once a swift-rs release exports its runtime; propose it upstream |
| `Cargo.toml`, `Cargo.lock` | 2 lines; swift-rs from a path | the `swift-rs` line under `[patch.crates-io]`; on a lock conflict keep the path source                                                                 |
| `.prettierignore`          | 1 line                        | `vendor`                                                                                                                                               |

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
Sage's other themes stay selectable and unchanged, and no shared component is edited.

| File                                                                                                             | Ours       | Note                                                                                                                   |
| ---------------------------------------------------------------------------------------------------------------- | ---------- | ---------------------------------------------------------------------------------------------------------------------- |
| `docs/design-guide.md`, `src/themes/klimper-light/`, `src/themes/klimper-dark/`                                  | new        | ordinary theme-o-rama themes named `light` and `dark`: theme-o-rama keys themes by name, so they replace its built-ins |
| `src/lib/klimperTheme.ts`, `src/klimper.css`                                                                     | new        | default theme once per install; ink sidebar, pill buttons, coins, total band, focus, 44px controls, motion             |
| `src/klimper/KlimperRoot.tsx`                                                                                    | new        | `import '@/lib/klimperTheme'`, evaluated before the `ThemeProvider` reads the saved theme                              |
| `src/icon-dark.png`, `src/icon-light.png`, `src-tauri/icons/*`, the iOS `AppIcon.appiconset`, Android `mipmap-*` | replaced   | the Klimper logo; every app icon is regenerated by `scripts/update-logo.sh` (new, ours). Take ours on a conflict       |
| `src-tauri/gen/android/app/src/main/res/values/ic_launcher_background.xml`                                       | one colour | set by `scripts/update-logo.sh` from the logo's corner                                                                 |

### Xchonnect dApp connections

Klimper answers CHIP-0002 requests from outside websites over Xchonnect (decision-008,
milestone m-4). Same shape as the passkey work: one crate of our own, screens of our own,
and single-line hooks in shared files. `docs/xchonnect.md` has the detail.

| File                                                                                                                 | Ours                                            | Note                                                                                                                                                                                                                                                                                                             |
| -------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tauri-plugin-klimper-xchonnect/`, `src/klimper/xchonnect*.{ts,tsx}`, `useXchonnect*.ts`                             | new                                             | the plugin, the typed command wrappers, the screens and their tests. `docs/xchonnect-screens.md` and `docs/xchonnect-links.md` are the design and the entry points                                                                                                                                               |
| `Cargo.toml`, `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`                                                         | workspace member, dep, one plugin               | exactly the three lines the passkey plugin needed                                                                                                                                                                                                                                                                |
| `src-tauri/capabilities/klimper.json`                                                                                | one permission                                  | `klimper-xchonnect:default`, main webview only — app webviews must never reach it                                                                                                                                                                                                                                |
| `src/klimper/routes.tsx`                                                                                             | new                                             | `xchonnectRoutes` appended by `withKlimper`; the routes live in `src/klimper/XchonnectRoutes.tsx`, each behind `featureElement`; the approval overlay is in `KlimperRoot`                                                                                                                                        |
| `src/pages/Settings.tsx`                                                                                             | one import, one element                         | `<XchonnectSettings />`, which renders nothing when the flag is off. The only entry point: before it, `/xchonnect` was reachable by typed URL alone                                                                                                                                                              |
| `src/components/Nav.tsx`                                                                                             | one import, one element                         | `<KlimperNavLinks />`: the Scan tab (`scanEnabled`) and Websites (`xchonnectEnabled`); every flag is checked inside it                                                                                                                                                                                           |
| `src/lib/features.ts`, `src/vite-env.d.ts`                                                                           | one flag                                        | `{scannerRoutes}` (`/scanner`, `src/klimper/Scanner.tsx`) and `VITE_DISABLE_SCAN`                                                                                                                                                                                                                                |
| `src-tauri/gen/apple/sage-tauri_iOS/sage-tauri_iOS.entitlements`                                                     | 1 line                                          | `applinks:clapandpay.com`, next to the passkey work's `webcredentials` entry. Serve `docs/well-known/apple-app-site-association` (TASK-040)                                                                                                                                                                      |
| `src-tauri/Info.plist`                                                                                               | 2 lines                                         | `NSCameraUsageDescription`: the desktop pairing screen reads QR codes from the camera (`src/klimper/QrCamera.tsx`)                                                                                                                                                                                               |
| `src-tauri/Info.ios.plist`, `capabilities/klimper.json`, `lib.rs`, both `Cargo.toml`, `package.json`                 | `CFBundleURLTypes`, permission, plugin, deps    | the `xchonnect` URL scheme and `tauri-plugin-deep-link` `~2.4` (the last line for Tauri 2.10; TASK-072.01). No `plugins.deep-link` in any Tauri conf: its build script would rewrite the iOS entitlements and drop `webcredentials`                                                                              |
| `tauri-plugin-klimper-jingle/`, `src/klimper/sound/`, `scripts/klimper-sound.py`, `docs/sound.md`                    | new                                             | the Klimper sound (decision-009, TASK-111): coin jingle, haptics, shake; behind `VITE_ENABLE_KLIMPER_SOUND` and a Settings switch. Its `feel` command is the slide-to-approve haptic (`src/klimper/haptics.ts`)                                                                                                  |
| `KlimperRoot.tsx`, `pages/Settings.tsx`, `KlimperSoundSettings.tsx`, `lib.rs`, `Cargo.toml` x2, `klimper.json`       | lazy element x2, plugin, dep, permission, flag  | the hook points of the sound (and the flag in `.env*`); `KlimperSoundSettings.tsx` is a shim that loads the card lazily so Settings.tsx keeps one import                                                                                                                                                         |
| `pages/Token.tsx`, `pages/TokenList.tsx`, `src/klimper/pageValue.ts`                                                 | one `usePageValue` line each; new               | the shake weighs what the open page shows: the pages publish the dollar value they already compute (decision-009, TASK-111); the registry is outside the sound chunk so the import is a few lines                                                                                                                |
| `src/lib/features.ts`, `src/vite-env.d.ts`, `.env`                                                                   | one flag                                        | `VITE_ENABLE_XCHONNECT`, on in `.env.production`, so release builds include it; unset means off                                                                                                                                                                                                                  |
| `src/pages/Login.tsx`, `index.html`                                                                                  | 1 line each                                     | the welcome headline and the page title say Klimper, not Sage (App Review 2.3: the name the user sees is the app's)                                                                                                                                                                                              |
| `src-tauri/Info.plist`, `gen/apple/sage-tauri_iOS/{Info.plist,sage-tauri_iOS.entitlements}`, `PrivacyInfo.xcprivacy` | purpose strings, entitlements, privacy manifest | no photo-library string (nothing saves photos), no Group Activities entitlement (nothing uses it), specific Face ID and NFC strings, and the privacy manifest with its required-reason APIs; the manifest is registered in `project.pbxproj`                                                                     |
| `src/contexts/PriceContext.tsx`, `src/klimper/price.ts`                                                              | three fetches became one call; new              | one price source (TASK-113): nodexch.space `/api/prices` on mainnet (TASK-112), CoinGecko + dexie as the fallback, 0 USD on testnet; the sound reads the same prices. On a conflict keep upstream's structure and re-add the one call                                                                            |
| `src/pages/Transaction.tsx`, `src/klimper/explorer.ts`                                                               | 3 imports, 3 lines, one button                  | "View on mempoolxch.space": the transaction's block on the explorer (a wallet transaction is one block height). Coin links in `AssetCoin.tsx` stay Sage's Spacescan                                                                                                                                              |
| `src-tauri/tauri.conf.json`                                                                                          | `security.csp`, `devCsp`, `withGlobalTauri`     | a Content Security Policy for the main webview (scripts only from the app; images, media and fonts from the web; fetch over https and IPC only; no frames, no plugins) and no `window.__TAURI__` (TASK-114). Sage apps set their own CSP in `crates/sage-apps`. _Propose upstream_; on a conflict keep the block |
| `vite.preview.config.ts`, `preview-*.html`, `src/preview/`                                                           | new                                             | the approval screens rendered with mock data in a plain browser, for screenshots and headless checks; never part of `vite build` (own config)                                                                                                                                                                    |
| `.prettierignore`                                                                                                    | 1 line                                          | the plugin's generated permissions                                                                                                                                                                                                                                                                               |
| `Cargo.lock`                                                                                                         | the plugin and its dependencies                 | `keyring` 3, and `xchonnect-core` / `xchonnect-wallet-kit` from crates.io (see below). On a conflict take upstream's lock and re-run `cargo check`                                                                                                                                                               |

The two xchonnect crates come from crates.io at an exact pre-release version
(`=0.1.0-rc.2`, clapandpay TASK-071 AC #1), so the workspace resolves on a machine without
an xchonnect checkout. Cargo never selects a pre-release for a plain requirement, and the
kit requires the core at exactly its own version: bump both lines in
`tauri-plugin-klimper-xchonnect/Cargo.toml` together, then `cargo check` to refresh
`Cargo.lock`. The xchonnect checkout is still needed for the development relay and
gateway (`scripts/push-dev.sh`, `docs/xchonnect.md`), not for building Klimper.

Nothing under `crates/`, `crates/sage-apps` or `packages/sage-app-sdk` is touched. The
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
   upstream first. The one exception is the keychain password, which cannot be a flag: re-apply
   it as "Staying current" describes.
3. Never resolve anything under `crates/sage-apps` or `packages/sage-app-sdk` in our favour.

## CI

Upstream's workflows (`build.yml`, `npm.yml`) stay in the tree unchanged so they never conflict,
but they need Sage's runners and secrets and would publish to npm. Disable them once in the
GitHub repository: `gh workflow disable Build && gh workflow disable NPM`. Ours is
`.github/workflows/images.yml`, named `Images` so that command does not touch it.
