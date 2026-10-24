# Klimper

**Clap and Pay.** Paying with your assets should be as easy as clapping. clapandpay.com

Klimper is the product and App Store name (from German _klimpern_, the jingle of coins in a
pocket); "Clap and Pay" is the tagline; the repositories and the domain keep the clapandpay name.

A minimal, fun, non-custodial Chia wallet for the phone, built on the
[Sage](https://github.com/xch-dev/sage) engine. It holds your assets and lets you pay with them;
everything else arrives as an app you install inside the wallet, with capabilities you grant.

Status: **Klimper 1.0.0**, the first public release (tag `klimper-v1.0.0`, notes in
[`docs/releases/1.0.0.md`](docs/releases/1.0.0.md)), on the Sage v0.13.1 engine. The iPhone app
is built and shipped through TestFlight from a Mac (`scripts/testflight.sh`); desktop and Android
build from source. Klimper's own screens: the wallet's total, the Scan tab, website connections
over [Xchonnect](https://github.com/xchonnect/xchonnect), push wake-ups, the coin sound and coin
pouch, and privacy mode. Planning is kept in a private wiki and backlog for now.

## The stack

| Layer                        | Project                                                                       |
| ---------------------------- | ----------------------------------------------------------------------------- |
| Wallet                       | Klimper (this repository)                                                     |
| Explorer, also an in-app app | [mempool-xch](https://github.com/maximedogawa/mempool-xch) · mempoolxch.space |
| Node provider                | nodexch · nodexch.com                                                         |

## How this repo will be shaped

Fork of Sage (Apache-2.0), `upstream` merged at every Sage release. Our differences: a features
config that hides offers, swaps, minting, options and DIDs from the GUI, a theme package, branding
and identifiers, and first-party apps in their own repos. The app bridge is never changed, so every
Sage app works here and every Klimper app works in Sage. What we changed and how
upstream is merged: [`docs/UPSTREAM.md`](docs/UPSTREAM.md).

## Build

Rust (the version in `rust-toolchain.toml`), Node and pnpm; platform prerequisites as for
[Tauri v2](https://v2.tauri.app/start/prerequisites/).

```sh
pnpm install
pnpm tauri:dev      # run
pnpm tauri:build    # bundle: target/release/bundle/
```

## Licence

AGPL-3.0-or-later for Klimper's own code, with an additional permission for distribution through
app stores (`LICENSE-APP-STORE-EXCEPTION.md`); the Sage engine keeps its Apache-2.0 licence (see
`NOTICE` and `LICENSE-APACHE`). Contributions are signed off under the DCO, no CLA
(`CONTRIBUTING.md`); vulnerabilities are reported privately (`SECURITY.md`). "Klimper", the
tagline "Clap and Pay" and the logo are trademarks and may not be used by forks (`TRADEMARK.md`). Not affiliated with Chia Network, Inc.
or the Sage project.
