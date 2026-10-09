// SPDX-License-Identifier: AGPL-3.0-or-later
// No commands: the webview never calls this plugin.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).ios_path("ios").build();
}
