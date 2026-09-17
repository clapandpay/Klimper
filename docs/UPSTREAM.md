# Upstream

Klimper is a fork of Sage: https://github.com/xch-dev/sage (Apache-2.0, see `LICENSE-APACHE` and
`NOTICE`). Remote name `upstream`. Upstream is merged, never rebased.

|                     |                                                                  |
| ------------------- | ---------------------------------------------------------------- |
| Merged version      | **v0.13.0** (`a98ca34b`, released 2026-08-19), merged 2026-09-17 |
| Hotfix merges since | none                                                             |

## Routine

```sh
scripts/sync-upstream.sh --check    # what is new upstream, security-relevant paths listed
scripts/sync-upstream.sh            # merge the newest release tag
scripts/sync-upstream.sh <commit>   # hotfix: merge upstream/main up to a fix commit
```

After a merge: `pnpm install`, `cargo check --workspace`, `pnpm tauri:build`, the bridge smoke
test (install mempoolxch.space and sage-permission-probe, grant capabilities, one bridge call),
then update the table above. A fix that cannot be merged because the commits before it are not
wanted yet: `git cherry-pick -x <commit>` on a `hotfix/` branch; the next tag merge resolves it.

## Our files (the whole diff against upstream)

New files never conflict. Files shared with Sage are kept to the lines listed here.

| File                                                                                                                                         | Ours                 | Note                                                                                                                                                                                                                                                 |
| -------------------------------------------------------------------------------------------------------------------------------------------- | -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `README.md`, `LICENSE`                                                                                                                       | whole file           | `.gitattributes` `merge=ours`; the script sets the driver                                                                                                                                                                                            |
| `LICENSE-APACHE`, `NOTICE`, `CLA.md`, `COMMERCIAL-LICENSE.md`, `TRADEMARK.md`, `CLAUDE.md`, `.gitattributes`                                 | new                  |                                                                                                                                                                                                                                                      |
| `docs/UPSTREAM.md`, `scripts/sync-upstream.sh`, `.github/workflows/klimper.yml`                                                              | new                  |                                                                                                                                                                                                                                                      |
| `src-tauri/tauri.macos.conf.json`, `tauri.linux.conf.json`, `tauri.windows.conf.json`                                                        | new                  | Tauri merges the platform file over `tauri.conf.json`: product name, identifier, version, window title. `tauri.conf.json` itself is untouched. The window block is a copy (JSON merge patch replaces arrays): compare it with upstream at each merge |
| `src-tauri/tauri.ios.conf.json`, `tauri.android.conf.json`                                                                                   | 3 lines added        | product name, version, identifier                                                                                                                                                                                                                    |
| `src-tauri/gen/apple/project.yml`, `sage-tauri.xcodeproj/project.pbxproj`                                                                    | bundle id            | `DEVELOPMENT_TEAM` is still Sage's; ours arrives with TASK-003                                                                                                                                                                                       |
| `src-tauri/gen/android/app/build.gradle.kts`, `.gitignore`, `res/values/strings.xml`, `MainActivity.kt` (moved to `com/clapandpay/klimper/`) | package and app name | not yet built, no Android SDK on the dev machine (2026-09-17)                                                                                                                                                                                        |

Identifier: `com.clapandpay.klimper`. Klimper therefore has its own data directory and never
opens a Sage wallet's data. Crate, package and binary names stay `sage-*`.

Known leftovers, deliberately not patched:

- `crates/sage-cli` and `crates/sage-client` look for the RPC certificates in the
  `com.rigidnetwork.sage` data directory. Propose an override upstream before relying on the CLI.
- App icons are still Sage's (`src-tauri/icons`, mobile asset catalogues). They must be replaced
  before any build leaves the team (TASK-004, TASK-008; trademark).
- Sage already reads `VITE_DISABLE_OFFERS`, `VITE_DISABLE_OPTIONS` and `VITE_DISABLE_SWAP` at
  build time (used for its App Store build). Start TASK-007 from these.

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
`.github/workflows/klimper.yml`.
