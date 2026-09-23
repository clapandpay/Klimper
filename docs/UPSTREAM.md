# Upstream

Sage: https://github.com/xch-dev/sage (Apache-2.0), remote `upstream`.

| Merged tag | Date       | By                                                    |
| ---------- | ---------- | ----------------------------------------------------- |
| v0.13.1    | 2026-09-23 | TASK-006 (first merge, `--allow-unrelated-histories`) |

Routine: `scripts/sync-upstream.sh <tag>` (see the wiki `architecture/fork-strategy.md`).

## Files that are ours

Resolve conflicts in these by keeping our side; everything else takes upstream.

- `LICENSE` (AGPL-3.0; Sage's licence lives in `LICENSE-APACHE`), `README.md`, `NOTICE`,
  `CLAUDE.md`, `CLA.md`, `COMMERCIAL-LICENSE.md`, `TRADEMARK.md`, `docs/UPSTREAM.md`
- `.env.production`, `.env.development` (feature flags)
- `.github/workflows/klimper.yml` (our CI)
- identifiers: `src-tauri/tauri.conf.json` (`productName`, `identifier`, window title, no
  development team), `src-tauri/gen/apple/project.yml` and `project.pbxproj`
  (`com.clapandpay.klimper`, product name Klimper, team from `APPLE_DEVELOPMENT_TEAM`),
  `index.html` title

Upstream's `.github/workflows/build.yml` and `npm.yml` need Sage's secrets and runners; they
are disabled in the GitHub Actions settings of this repository, not deleted, so merges stay
clean.

Known leftovers of the Sage identity (upstream code, left alone): `sage-cli` and `sage-client`
look for the GUI data under `com.rigidnetwork.sage`; the Android project keeps
`com.rigidnetwork.sage` until an Android build is in scope.
