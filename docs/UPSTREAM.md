# Upstream

> **Monorepo (2026-09-20).** This fork lives in `klimper/` of the clapandpay repository, while
> upstream Sage has the same files at its root. Every path in this document is relative to
> `klimper/`. The sync script merges with `git merge -X subtree=klimper`, which shifts upstream's
> paths into this folder; a trial merge of `upstream/main` on the day of the move touched 103
> files, all inside `klimper/`, with no conflicts. Never merge upstream by hand without that
> option, and never relocate files inside `klimper/`. Upstream's own workflows sit in
> `klimper/.github/`, where GitHub does not run them; ours is `/.github/workflows/ci.yml`.

Klimper is a fork of Sage: https://github.com/xch-dev/sage (Apache-2.0, see `LICENSE-APACHE` and
`NOTICE`). Remote name `upstream`. Upstream is merged, never rebased.

|                     |                                |
| ------------------- | ------------------------------ |
| Merged version      | **v0.13.1**, merged 2026-09-20 |
| Hotfix merges since | none                           |
| Klimper version     | **0.13.1**, the merged tag     |

**Klimper's version is the version of the Sage release it is built on.** Sage's app system
(`crates/sage-apps`) checks an app's minimum Sage version against the Tauri app version
(`package_info().version`), not the engine's, so a Klimper version of its own made every app that
needs Sage 0.13 refuse to run ("You are running Sage 0.1.0"; TASK-063). The version lives in
`src-tauri/tauri.{macos,linux,windows,ios,android}.conf.json` and the committed iOS `Info.plist`;
the upgrade below bumps it. Once upstream compares against the engine version instead, Klimper
can number its own releases again.

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
   grep -h '"version"' src-tauri/tauri.*.conf.json  # the merged tag, every one (see the top)
   grep -c '^VITE_' .env                            # 11
   grep -c featureElement src/App.tsx               # 8
   grep -rn 'b""' crates/sage/src                   # nothing: new upstream call sites merge
                                                    # cleanly but pass the empty password
   grep -c 'Passkey' src/App.tsx src/pages/Settings.tsx  # 2 each
   ```

7. **Credit the people whose work this is.** `scripts/sync-upstream.sh --credits`, and update
   `NOTICE` if the list moved. Apache-2.0 asks a fork to keep attribution, and since Sage ships
   no `NOTICE` of its own, ours is the only place its authors are named. A new contributor
   upstream is a name missing from ours until someone regenerates it.
8. **Write it down.** The merged version in the table above; Klimper's version set to it in the
   five `tauri.*.conf.json` files and `src-tauri/gen/apple/sage-tauri_iOS/Info.plist`; and any
   new row in the file tables below — `--diff` tells you if one is missing.

## Our files (the whole diff against upstream)

New files never conflict. Files shared with Sage are kept to the lines listed here.

| File                                                                                                                                         | Ours                                                        | Note                                                                                                                                                                                                                                                                                                 |
| -------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `README.md`, `LICENSE`                                                                                                                       | whole file                                                  | `.gitattributes` `merge=ours`; the script sets the driver                                                                                                                                                                                                                                            |
| `LICENSE-APACHE`, `NOTICE`, `CLA.md`, `COMMERCIAL-LICENSE.md`, `TRADEMARK.md`, `CLAUDE.md`, `.gitattributes`                                 | new                                                         |                                                                                                                                                                                                                                                                                                      |
| `docs/UPSTREAM.md`, `scripts/sync-upstream.sh`, `scripts/testflight.sh`, `.github/workflows/ci.yml`                                          | new                                                         |                                                                                                                                                                                                                                                                                                      |
| `src-tauri/tauri.macos.conf.json`, `tauri.linux.conf.json`, `tauri.windows.conf.json`                                                        | new                                                         | Tauri merges the platform file over `tauri.conf.json`: product name, identifier, version, window title. `tauri.conf.json` itself is untouched. The window block is a copy (JSON merge patch replaces arrays): compare it with upstream at each merge                                                 |
| `src-tauri/tauri.ios.conf.json`, `tauri.android.conf.json`                                                                                   | 3 lines added, and `bundle.iOS`                             | product name, version, identifier; our Apple team `HTP35W93ZF` (`tauri.conf.json` keeps Sage's)                                                                                                                                                                                                      |
| `src-tauri/gen/apple/project.yml`, `sage-tauri.xcodeproj/project.pbxproj`, `sage-tauri_iOS/Info.plist`                                       | bundle id; team; iOS 15.0 minimum; product name and version | `DEVELOPMENT_TEAM` is ours, `HTP35W93ZF` (TASK-003): take ours on a conflict. Xcode 27 refuses the iOS 14.0 minimum. `tauri ios build` writes the product name and version from `tauri.ios.conf.json` into the pbxproj and Info.plist on every build, so those lines are committed as it writes them |
| `src-tauri/gen/android/app/build.gradle.kts`, `.gitignore`, `res/values/strings.xml`, `MainActivity.kt` (moved to `com/clapandpay/klimper/`) | package and app name                                        | not yet built, no Android SDK on the dev machine (2026-09-17)                                                                                                                                                                                                                                        |

### Reduced interface (TASK-007)

Klimper shows the wallet primitives only: assets, send, receive, addresses, coins, transactions,
peers, settings, and the Apps screen with its bridge approval dialogs. The rest is switched off
with build flags that extend Sage's own `VITE_DISABLE_OFFERS / OPTIONS / SWAP` pattern; nothing
is deleted, the engine and the bridge keep every capability, and every flag defaults to Sage's
behaviour.

| File                                                                          | Ours                                       | Note                                                                              |
| ----------------------------------------------------------------------------- | ------------------------------------------ | --------------------------------------------------------------------------------- |
| `.env`                                                                        | new, force-added (upstream ignores `.env`) | the nine `VITE_DISABLE_*` flags; no secrets, those go in `.env.local`             |
| `src/lib/featureElement.tsx`                                                  | new                                        | route gate: a disabled feature redirects to `/wallet`                             |
| `src/lib/features.ts`, `src/vite-env.d.ts`                                    | six flags appended                         | NFTs, profiles, token issuing, WalletConnect, RPC settings, theme gallery         |
| `src/App.tsx`                                                                 | import block, seven route `element=` lines | offers, options and swap routes were reachable by URL upstream even when disabled |
| `src/components/Nav.tsx`, `src/pages/TokenList.tsx`, `src/pages/Settings.tsx` | guards around existing blocks              |                                                                                   |

These seven files are what `scripts/sync-upstream.sh --export` writes out, and they are meant to
stop being ours: propose them to Sage as one pull request ("more build-time feature flags, and
gate the routes"). They are not the whole frontend diff — the two features below are ours to
keep.

### Our own features, behind their own flags

Added the same way and just as removable, but nothing Sage would want: they are Clap and Pay's,
not a wallet primitive. Both default to off in `features.ts`, so a build without `.env` is Sage's
behaviour.

| File                                                                                  | Ours                                 | Note                                                                                 |
| ------------------------------------------------------------------------------------- | ------------------------------------ | ------------------------------------------------------------------------------------ |
| `src/lib/handles.ts`, `src/hooks/useHandle.ts`, `src/components/HandleResolution.tsx` | new                                  | `@anna` instead of an address, through XCHandles (mainnet only)                      |
| `src/pages/Send.tsx`                                                                  | +54 lines, `handlesEnabled`          | the largest edit to a shared file; the first thing to re-apply by hand on a conflict |
| `src/pages/ClapAndPayConnect.tsx`, `src/lib/clapandpayHandoff.ts`                     | new                                  | "Use my Clap and Pay money": the passkey wallet in one Face ID                       |
| `src/pages/Login.tsx`                                                                 | +9 lines, `clapAndPayConnectEnabled` | one button on the start screen                                                       |
| `.prettierignore`                                                                     | 1 line                               | `src/lib/clapandpayHandoff.ts`, which is generated                                   |
| `package.json`                                                                        | `workspaces` array added             | for bun; pnpm reads `pnpm-workspace.yaml`. Drop it if bun goes away                  |

### Passkey login

Klimper opens the keychain with a passkey (see `docs/passkey.md`). One engine change, proposed
upstream; the rest is files of our own and one-line hooks.

| File                                                                                          | Ours                                     | Note                                                                                                                                                                          |
| --------------------------------------------------------------------------------------------- | ---------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/sage-keychain`, `crates/sage/src/{sage.rs,error.rs,endpoints/*,utils/spends.rs}`      | keychain password                        | the one engine change: lock, unlock, re-key instead of the empty password                                                                                                     |
| `tauri-plugin-klimper-passkey/`, `src/klimper/`, `src-tauri/capabilities/klimper.json`        | new                                      | envelope, commands, Swift; lock screen and settings entry                                                                                                                     |
| `Cargo.toml`, `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`                                  | workspace member, dep, one plugin        |                                                                                                                                                                               |
| `Cargo.lock`                                                                                  | the plugin, and `swift-rs` 1.0.7 → 1.0.8 | 1.0.7 builds Swift for macOS under Xcode 27, so no iOS build works; on a conflict take upstream's lock, then `cargo update -p swift-rs` unless upstream is already past 1.0.7 |
| `src/App.tsx`, `src/pages/Settings.tsx`                                                       | one import and one element each          | `<PasskeyGate />`, `<PasskeySettings />`                                                                                                                                      |
| `src-tauri/gen/apple/sage-tauri_iOS/sage-tauri_iOS.entitlements`, `.prettierignore`           | 4 lines, 1 line                          | `webcredentials:clapandpay.com`; the plugin's generated permissions                                                                                                           |
| `src-tauri/gen/android/app/src/main/AndroidManifest.xml`, `res/xml/data_extraction_rules.xml` | 2 attributes, new                        | no backup, no device transfer: the wallet data never leaves the phone (TASK-057)                                                                                              |

### Look: Klimper Coin (TASK-062)

The design guide (Klimper Coin) is `docs/design-guide.md`. It is applied as two themes and one stylesheet scoped
to them. The themes are named `light` and `dark`, so they replace Sage's core themes of that name;
Sage's other themes stay selectable and unchanged, and no shared component is edited.

| File                                                                                                             | Ours       | Note                                                                                                                   |
| ---------------------------------------------------------------------------------------------------------------- | ---------- | ---------------------------------------------------------------------------------------------------------------------- |
| `docs/design-guide.md`, `src/themes/klimper-light/`, `src/themes/klimper-dark/`                                  | new        | ordinary theme-o-rama themes named `light` and `dark`: theme-o-rama keys themes by name, so they replace its built-ins |
| `src/lib/klimperTheme.ts`, `src/klimper.css`                                                                     | new        | default theme once per install; ink sidebar, pill buttons, coins, total band, focus, 44px controls, motion             |
| `src/App.tsx`                                                                                                    | one import | `import '@/lib/klimperTheme'`, before the `ThemeProvider` reads the saved theme                                        |
| `src/icon-dark.png`, `src/icon-light.png`, `src-tauri/icons/*`, the iOS `AppIcon.appiconset`, Android `mipmap-*` | replaced   | the Klimper logo; every app icon is regenerated by `scripts/update-logo.sh` (new, ours). Take ours on a conflict       |
| `src-tauri/gen/android/app/src/main/res/values/ic_launcher_background.xml`                                       | one colour | set by `scripts/update-logo.sh` from the logo's corner                                                                 |

### Xchonnect dApp connections

Klimper answers CHIP-0002 requests from outside websites over Xchonnect (decision-008,
milestone m-4). Same shape as the passkey work: one crate of our own, screens of our own,
and single-line hooks in shared files. `docs/xchonnect.md` has the detail.

| File                                                                                     | Ours                              | Note                                                                                                                                                               |
| ---------------------------------------------------------------------------------------- | --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `tauri-plugin-klimper-xchonnect/`, `src/klimper/xchonnect*.{ts,tsx}`, `useXchonnect*.ts` | new                               | the plugin, the typed command wrappers, the screens and their tests. `docs/xchonnect-screens.md` and `docs/xchonnect-links.md` are the design and the entry points |
| `Cargo.toml`, `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs`                             | workspace member, dep, one plugin | exactly the three lines the passkey plugin needed                                                                                                                  |
| `src-tauri/capabilities/klimper.json`                                                    | one permission                    | `klimper-xchonnect:default`, main webview only — app webviews must never reach it                                                                                  |
| `src/App.tsx`                                                                            | one import, one element           | `{xchonnectRoutes}` — the five routes live in `src/klimper/XchonnectRoutes.tsx`, each behind `featureElement`                                                      |
| `src/pages/Settings.tsx`                                                                 | one import, one element           | `<XchonnectSettings />`, which renders nothing when the flag is off. The only entry point: before it, `/xchonnect` was reachable by typed URL alone                |
| `src/components/Nav.tsx`                                                                 | one import, one element           | `<KlimperNavLinks />`: the Scan tab (`scanEnabled`) and Websites (`xchonnectEnabled`); every flag is checked inside it                                             |
| `src/App.tsx`, `src/lib/features.ts`, `src/vite-env.d.ts`                                | one element, one flag             | `{scannerRoutes}` (`/scanner`, `src/klimper/Scanner.tsx`) and `VITE_DISABLE_SCAN`                                                                                  |
| `src-tauri/gen/apple/sage-tauri_iOS/sage-tauri_iOS.entitlements`                         | 1 line                            | `applinks:clapandpay.com`, next to the passkey work's `webcredentials` entry. Serve `docs/well-known/apple-app-site-association` (TASK-040)                        |
| `src-tauri/Info.plist`                                                                   | 2 lines                           | `NSCameraUsageDescription`: the desktop pairing screen reads QR codes from the camera (`src/klimper/QrCamera.tsx`)                                                 |
| `src/lib/features.ts`, `src/vite-env.d.ts`, `.env`                                       | one flag                          | `VITE_ENABLE_XCHONNECT`, off, and commented out in `.env` until TASK-094 has reviewed the flow                                                                     |
| `.prettierignore`                                                                        | 1 line                            | the plugin's generated permissions                                                                                                                                 |
| `Cargo.lock`                                                                             | the plugin and its dependencies   | `keyring` 3, and `xchonnect-core` / `xchonnect-wallet-kit` as **path** dependencies (see below). On a conflict take upstream's lock and re-run `cargo check`       |

The two xchonnect crates are deliberately unpublished (xchonnect TASK-67 publishes after
its audit), so they are consumed by path:
`../../../../xchonnect/xchonnect/crates/{core,wallet-kit}`, which resolves the same from
this repository and from a `clapandpay-wt-*` worktree beside it. **That is a local
development arrangement and a release blocker**: it must become a git-revision pin or a
published version before anything ships (clapandpay TASK-071 AC #1). Nothing in CI
compiles Rust, so the path does not break the pipeline; a `cargo` command on a machine
without the xchonnect checkout will fail to resolve the workspace.

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
`.github/workflows/ci.yml`.
