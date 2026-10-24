# AGENTS.md: klimper

Klimper, the wallet app (App Store name Klimper, tagline "Clap and Pay"): a fork of Sage
(xch-dev/sage) reduced to the wallet core, with in-app apps as the growth path. Rust, Tauri v2,
React; pnpm 10. CI and git rules: below.

- Read `docs/UPSTREAM.md`.
- Upstream Sage has these files where they are here, and this history shares Sage's commits.
  Merge upstream only through `scripts/sync-upstream.sh`. Never move a file away from where
  upstream has it: that is what keeps merges clean.
- Never edit the app bridge or capability model (`crates/sage-apps`,
  `packages/sage-app-sdk`); every change there goes upstream to Sage.
- Keep the diff against upstream small: flags, theme, branding, identifiers, `docs/`. Shared
  components are not edited; add hiding points upstream.
- Licence mix: our files carry the AGPL-3.0-or-later header; Sage's Apache-2.0 notices stay
  (`LICENSE-APACHE`, `NOTICE`).
- **CI runs no tests** (CI, below). Run `pnpm prettier:check`, `pnpm lint`, `pnpm test:unit`, and
  for the engine `cargo fmt`, `cargo clippy` and `cargo test`, locally, and say so in the pull
  request, especially for a change under `crates/`, `src-tauri/`, `tauri-plugin-sage/`,
  `migrations/` or `Cargo.lock`.
- The iPhone app is not built in CI: it is archived and uploaded to TestFlight and the App Store
  from a Mac (`scripts/testflight.sh`). The desktop installers are built by the Release workflow when
  a release is published (CI, below). Release tags are `klimper-v*`; never a plain `v*` (upstream's).
- This file is ours, not upstream's: Sage has no AGENTS.md. If it ever adds one, keep ours.

## CI

**Never put a schedule on a workflow.** No `schedule:`, no `cron:`, not commented out and not
"just for now": a job on a clock is a failure nobody reads, and a bill in every private fork.
Anything that wants to run periodically is a script run by hand; the upstream check is
`scripts/sync-upstream.sh --check`.

There is one workflow, `.github/workflows/release.yml` ("Release"), and it runs only when a person
publishes a GitHub release tagged `klimper-v<version>`. It builds the desktop installers (macOS,
Linux and Windows, x64 and ARM64) and attaches them to that release; macOS is signed and
notarized with the Apple secrets listed at its top (TASK-161, TASK-163). It never runs on a push,
a pull request or a clock. Keep it the only workflow, and keep it short enough to read top to
bottom.

Sage's own workflows (`build.yml`, `npm.yml`) are deleted here: they need Sage's runners and
secrets, and `npm.yml` publishes Sage's packages. `scripts/sync-upstream.sh` removes any Sage
workflow a merge brings back.

## Git workflow

**Never touch `main` directly.** Not a commit, a push, a force push, a rebase, a tag or a history
rewrite: `main` changes only through a pull request that a person merges. If a command would
write to it, do not run it and say so instead.

**One branch per task**, from an up-to-date `main`:

```sh
git fetch origin && git switch -c task-042-short-slug origin/main
```

`TASK-nnn` and `decision-nnn` in branch names, code and docs refer to the maintainers' backlog,
which is not public. Keep a branch to its task's scope. Sign off every commit (`git commit -s`,
`CONTRIBUTING.md`).
