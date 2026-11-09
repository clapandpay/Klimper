# chia-sdk-next

chia-wallet-sdk's partial offer driver (CHIP-0052), copied from xch-dev/chia-wallet-sdk at
`c803295c644a736a9cbeb4ed52f7ff2bb0c79aad` (main, 2026-11-04; PRs #505 and #506), so Klimper can use it on chia-wallet-sdk
0.37.1, the release upstream Sage builds on. chia-wallet-sdk main itself needs chia 0.53,
which Sage is not on yet.

**Delete this crate** once Sage depends on a chia-wallet-sdk release with the driver: then
`use chia_wallet_sdk::driver::{PartialOffer, …}` instead of `chia_sdk_next::…`, and nothing else
changes.

The files under `src/partial_offer/` are the SDK's, changed only where they must be:

| Here | Upstream |
| --- | --- |
| `partial_offer/puzzle.rs` | `crates/chia-sdk-types/src/puzzles/partial_offer.rs` |
| `partial_offer/info.rs` | `crates/chia-sdk-driver/src/primitives/partial_offer/partial_offer_info.rs` |
| `partial_offer/coin.rs` | `crates/chia-sdk-driver/src/primitives/partial_offer/partial_offer_coin.rs` |
| `partial_offer/bundle.rs` | `crates/chia-sdk-driver/src/primitives/partial_offer/partial_offer_bundle.rs` |
| `partial_offer/tests.rs` | `crates/chia-sdk-driver/src/primitives/partial_offer/tests.rs` |
| `text.rs` | `encode_partial_offer` / `decode_partial_offer` in `crates/chia-sdk-driver/src/offers/compress.rs` |

The changes: import paths (`crate::sdk::…`, which re-exports chia-wallet-sdk 0.37.1 and these
files), and `DriverError` is this crate's (`src/error.rs`), since 0.37.1's lacks the partial
offer variants. `diff` a file against upstream to see nothing else moved.

Licence: Apache-2.0, as chia-wallet-sdk (`LICENSE-APACHE` is its licence file).
