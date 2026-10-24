// SPDX-License-Identifier: AGPL-3.0-or-later
const COMMANDS: &[&str] = &["card_status", "card_sign_in", "card_sign_out"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
