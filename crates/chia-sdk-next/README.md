# chia-sdk-next

chia-wallet-sdk's partial offer driver (CHIP-0052, PRs #505 and #506) and auction driver (PR #507), copied from xch-dev/chia-wallet-sdk at
`c803295c644a736a9cbeb4ed52f7ff2bb0c79aad` (main, 2026-11-04), so Klimper can use it on chia-wallet-sdk
0.37.1, the release upstream Sage builds on. chia-wallet-sdk main itself needs chia 0.53,
which Sage is not on yet.

**Delete this crate** once Sage depends on a chia-wallet-sdk release with the drivers (features
`chip-0052` and `auction`): then `use chia_wallet_sdk::driver::{PartialOffer, Auction, …}`
instead of `chia_sdk_next::…`, drop the `AuctionExt` / `LauncherAuctionExt` imports, and
nothing else changes.

The files under `src/partial_offer/` and `src/auction/` are the SDK's, changed only where they
must be:

| Here                                       | Upstream                                                                                           |
| ------------------------------------------ | -------------------------------------------------------------------------------------------------- |
| `partial_offer/puzzle.rs`                  | `crates/chia-sdk-types/src/puzzles/partial_offer.rs`                                               |
| `partial_offer/info.rs`                    | `crates/chia-sdk-driver/src/primitives/partial_offer/partial_offer_info.rs`                        |
| `partial_offer/coin.rs`                    | `crates/chia-sdk-driver/src/primitives/partial_offer/partial_offer_coin.rs`                        |
| `partial_offer/bundle.rs`                  | `crates/chia-sdk-driver/src/primitives/partial_offer/partial_offer_bundle.rs`                      |
| `partial_offer/tests.rs`                   | `crates/chia-sdk-driver/src/primitives/partial_offer/tests.rs`                                     |
| `text.rs`                                  | `encode_partial_offer` / `decode_partial_offer` in `crates/chia-sdk-driver/src/offers/compress.rs` |
| `auction/mod.rs`                           | `crates/chia-sdk-driver/src/primitives/auction.rs`                                                 |
| `auction/auction_*.rs`, `auction/tests.rs` | `crates/chia-sdk-driver/src/primitives/auction/` (same names)                                      |
| `auction/puzzles/mod.rs`                   | `crates/chia-sdk-types/src/puzzles/auction.rs`                                                     |
| `auction/puzzles/*.rs`                     | `crates/chia-sdk-types/src/puzzles/auction/` (same names)                                          |

The changes: import paths (`crate::sdk::…`, which re-exports chia-wallet-sdk 0.37.1 and these
files); `DriverError` is this crate's (`src/error.rs`), since 0.37.1's lacks the new variants,
so a few tail expressions convert their errors (`.map_err(Into::into)`); and upstream's inherent
methods on chia-wallet-sdk's own types (`impl Auction`, i.e. `Singleton<AuctionInfo>`, and
`impl Launcher { launch_auction }`) are the traits `AuctionExt` and `LauncherAuctionExt`, with the
same names and bodies, since a crate can't add inherent methods to another crate's type.
`diff` a file against upstream to see nothing else moved. All of the SDK's partial offer and
auction tests run here and pass.

Licence: Apache-2.0, as chia-wallet-sdk (`LICENSE-APACHE` is its licence file).
