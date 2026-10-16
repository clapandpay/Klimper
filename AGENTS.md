# AGENTS.md: klimper

Klimper, the wallet app (App Store name Klimper, tagline "Clap and Pay"): a fork of Sage
(xch-dev/sage) reduced to the wallet core, with in-app apps as the growth path. Rust, Tauri v2,
React; pnpm 10. Repository-wide rules (CI, git workflow): `../AGENTS.md`.

- Read the wiki `architecture/fork-strategy.md` and `docs/UPSTREAM.md`.
- Upstream Sage has these files at its root; here they are under `klimper/`. Merges use git's
  subtree option and only ever through `scripts/sync-upstream.sh`. Never move files
  inside this folder away from where upstream has them: that is what keeps merges clean.
- Never edit the app bridge or capability model (`crates/sage-apps`,
  `packages/sage-app-sdk`); every change there goes upstream to Sage.
- Keep the diff against upstream small: flags, theme, branding, identifiers, `docs/`. Shared
  components are not edited; add hiding points upstream.
- Licence mix: our files carry the AGPL-3.0-or-later header; Sage's Apache-2.0 notices stay
  (`LICENSE-APACHE`, `NOTICE`).
- **CI runs no tests** (`../AGENTS.md`, CI). Run `pnpm prettier:check`, `pnpm lint`, `pnpm test:unit`, and
  for the engine `cargo fmt`, `cargo clippy` and `cargo test`, locally, and say so in the pull
  request, especially for a change under `crates/`, `src-tauri/`, `tauri-plugin-sage/`,
  `migrations/` or `Cargo.lock`.
- Klimper is not built in CI: it is archived and uploaded to TestFlight and the App Store from a
  Mac (`scripts/testflight.sh`). Release tags are `klimper-v*`; never a plain `v*` (upstream's).
- This file is ours, not upstream's: Sage has no AGENTS.md. If it ever adds one, keep ours.
