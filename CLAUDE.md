# CLAUDE.md

Klimper (product and App Store name; tagline "Clap and Pay"; repository clapandpay) is a fork of Sage (xch-dev/sage) reduced to the wallet core, with a minimalist
interface and in-app apps as the growth path. Sibling repos: `../clapandpay-backlog` (tasks,
Backlog.md CLI, run `backlog` there) and `../clapandpay-wiki` (documentation; GitHub name clap-and-pay-wiki); the other layers
are `../mempool-xch` (explorer, in-app app) and `../nodexch` (node provider).

- Read the wiki `product/vision.md` and `architecture/fork-strategy.md` first.
- Never edit the app bridge or capability model (`crates/sage-apps`, `packages/sage-app-sdk`);
  every change there goes upstream to Sage.
- Keep the diff against upstream small: flags, theme, branding, identifiers, `docs/`. Shared
  components are not edited; add hiding points upstream.
- Licence mix: our files carry the AGPL-3.0-or-later header; Sage's Apache-2.0 notices stay.
