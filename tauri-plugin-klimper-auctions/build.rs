// SPDX-License-Identifier: AGPL-3.0-or-later
const COMMANDS: &[&str] = &[
    "view_auction",
    "start_auction",
    "bid_auction",
    "end_auction",
    "cancel_auction",
    "get_auctions",
    "delete_auction",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
