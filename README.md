# Klimper

**Clap and Pay.** Paying with your assets should be as easy as clapping. clapandpay.com

Klimper is the product and App Store name (from German *klimpern*, the jingle of coins in a
pocket); "Clap and Pay" is the tagline; the repositories and the domain keep the clapandpay name.

A minimal, fun, non-custodial Chia wallet for the phone, built on the
[Sage](https://github.com/xch-dev/sage) engine. It holds your assets and lets you pay with them;
everything else arrives as an app you install inside the wallet, with capabilities you grant.

Status: groundwork (2026-09-17). The vision is in
[clap-and-pay-wiki](https://github.com/maximedogawa/clap-and-pay-wiki) (`product/vision.md`); tasks in
[clapandpay-backlog](https://github.com/maximedogawa/clapandpay-backlog). Milestone 1 turns this
repository into the Sage core fork for iOS, shipped as Klimper.

## The stack

| Layer | Project |
| --- | --- |
| Wallet | Klimper (this repo, clapandpay) |
| Explorer, also an in-app app | [mempool-xch](https://github.com/maximedogawa/mempool-xch) · mempoolxch.space |
| Node provider | [nodexch](https://github.com/maximedogawa/nodexch) · nodexch.space |

## How this repo will be shaped

Fork of Sage (Apache-2.0), `upstream` merged at every Sage release. Our differences: a features
config that hides offers, swaps, minting, options and DIDs from the GUI, a theme package, branding
and identifiers, and first-party apps in their own repos. The app bridge is never changed, so every
Sage app works here and every Klimper app works in Sage. Details: wiki
`architecture/fork-strategy.md`; the fork setup is backlog TASK-006.

## Licence

AGPL-3.0-or-later for Klimper's own code; the Sage engine keeps its Apache-2.0 licence (see
`NOTICE` once the fork lands). Commercial licences without the copyleft obligation are available,
see `COMMERCIAL-LICENSE.md`; contributions require the CLA. "Klimper", the tagline "Clap and Pay" and the logo are
trademarks and may not be used by forks (`TRADEMARK.md`). Not affiliated with Chia Network, Inc.
or the Sage project.
