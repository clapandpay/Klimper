// SPDX-License-Identifier: AGPL-3.0-or-later
const COMMANDS: &[&str] = &["card_status"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
