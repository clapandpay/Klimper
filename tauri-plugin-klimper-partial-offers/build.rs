// SPDX-License-Identifier: AGPL-3.0-or-later
const COMMANDS: &[&str] = &[
    "view_partial_offer",
    "take_partial_offer",
    "make_partial_offer",
    "cancel_partial_offer",
    "get_partial_offers",
    "delete_partial_offer",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
