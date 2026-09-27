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
   grep -c '^VITE_' .env                            # 11
   grep -c featureElement src/App.tsx               # 8
   ```

7. **Credit the people whose work this is.** `scripts/sync-upstream.sh --credits`, and update
   `NOTICE` if the list moved. Apache-2.0 asks a fork to keep attribution, and since Sage ships
   no `NOTICE` of its own, ours is the only place its authors are named. A new contributor
   upstream is a name missing from ours until someone regenerates it.
8. **Write it down.** The merged version in the table above, and any new row in the file tables
   below — `--diff` tells you if one is missing.

## Our files (the whole diff against upstream)

New files never conflict. Files shared with Sage are kept to the lines listed here.

| File                                                                                                                                         | Ours                                                  | Note                                                                                                                                                                                                                                                                                   |
| -------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `README.md`, `LICENSE`                                                                                                                       | whole file                                            | `.gitattributes` `merge=ours`; the script sets the driver                                                                                                                                                                                                                              |
| `LICENSE-APACHE`, `NOTICE`, `CLA.md`, `COMMERCIAL-LICENSE.md`, `TRADEMARK.md`, `CLAUDE.md`, `.gitattributes`                                 | new                                                   |                                                                                                                                                                                                                                                                                        |
| `docs/UPSTREAM.md`, `scripts/sync-upstream.sh`, `.github/workflows/ci.yml`                                                                   | new                                                   |                                                                                                                                                                                                                                                                                        |
| `src-tauri/tauri.macos.conf.json`, `tauri.linux.conf.json`, `tauri.windows.conf.json`                                                        | new                                                   | Tauri merges the platform file over `tauri.conf.json`: product name, identifier, version, window title. `tauri.conf.json` itself is untouched. The window block is a copy (JSON merge patch replaces arrays): compare it with upstream at each merge                                   |
| `src-tauri/tauri.ios.conf.json`, `tauri.android.conf.json`                                                                                   | 3 lines added                                         | product name, version, identifier                                                                                                                                                                                                                                                      |
| `src-tauri/gen/apple/project.yml`, `sage-tauri.xcodeproj/project.pbxproj`, `sage-tauri_iOS/Info.plist`                                       | bundle id; iOS 15.0 minimum; product name and version | `DEVELOPMENT_TEAM` is still Sage's; ours arrives with TASK-003. Xcode 27 refuses the iOS 14.0 minimum. `tauri ios build` writes the product name and version from `tauri.ios.conf.json` into the pbxproj and Info.plist on every build, so those lines are committed as it writes them |
| `src-tauri/gen/android/app/build.gradle.kts`, `.gitignore`, `res/values/strings.xml`, `MainActivity.kt` (moved to `com/clapandpay/klimper/`) | package and app name                                  | not yet built, no Android SDK on the dev machine (2026-09-17)                                                                                                                                                                                                                          |

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

### Look: Beidwerk UI v2 (TASK-062)

The design guide is `docs/design-guide.md`. It is applied as two themes and one stylesheet scoped
to them, so choosing a Sage theme gives Sage unchanged and no shared component is edited.

| File                                                                                                             | Ours       | Note                                                                                                    |
| ---------------------------------------------------------------------------------------------------------------- | ---------- | ------------------------------------------------------------------------------------------------------- |
| `docs/design-guide.md`, `src/themes/klimper-light/`, `src/themes/klimper-dark/`                                  | new        | the themes are ordinary theme-o-rama themes, tagged `featured`                                          |
| `src/lib/klimperTheme.ts`, `src/klimper.css`                                                                     | new        | first-launch default by system appearance; focus, 44px controls, tabular numbers, motion, nav marker    |
| `src/App.tsx`                                                                                                    | one import | `import '@/lib/klimperTheme'`, before the `ThemeProvider` reads the saved theme                         |
| `src/icon-dark.png`, `src/icon-light.png`, `src-tauri/icons/*`, the iOS `AppIcon.appiconset`, Android `mipmap-*` | replaced   | the Klimper logo; regenerate with `pnpm tauri icon` from the logo on `#F2ECE5`. Take ours on a conflict |
| `src-tauri/gen/android/app/src/main/res/values/ic_launcher_background.xml`                                       | one colour | `#F2ECE5`                                                                                               |

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
GitHub repository: `gh workflow disable Build && gh workflow disable NPM`. Ours is
`.github/workflows/ci.yml`.
