# Contributing to Klimper

Thank you for helping. Issues and pull requests are welcome; for anything larger than a fix,
open an issue first so we can agree on the approach before you spend the time.

## Sign-off

Every commit carries a `Signed-off-by:` line (`git commit -s`). It certifies the
[Developer Certificate of Origin 1.1](https://developercertificate.org/): you wrote the change,
or otherwise have the right to submit it under this repository's licence. There is no CLA and
no copyright assignment: you keep the copyright of what you write.

## Licence

Contributions are accepted under the licence of the file they change. Klimper's own files are
AGPL-3.0-or-later with the App Store exception (`LICENSE`, `LICENSE-APP-STORE-EXCEPTION.md`).
Files that come from Sage stay under the Apache-2.0 (`LICENSE-APACHE`, `NOTICE`); a change to
one of them is listed in `docs/UPSTREAM.md` and carries the line
`Modified by Klimper (2026): see docs/UPSTREAM.md`.

## Before you open a pull request

CI runs no tests. Run the checks yourself and say in the pull request that they pass:

```sh
pnpm prettier:check && pnpm lint && pnpm test:unit
cargo fmt --check && cargo clippy && cargo test   # for a change in the engine
cargo deny --locked check licenses                # for a new or updated Rust dependency
```

A change that belongs in Sage itself (the wallet engine, the app bridge, a shared component)
goes upstream to [Sage](https://github.com/xch-dev/sage) first; `AGENTS.md` and
`docs/UPSTREAM.md` explain why the diff against Sage is kept small.

Security issues are not reported here: see [`SECURITY.md`](SECURITY.md).
