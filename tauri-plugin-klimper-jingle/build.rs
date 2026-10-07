// SPDX-License-Identifier: AGPL-3.0-or-later
const COMMANDS: &[&str] = &["play", "set_shake_enabled"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .ios_path("ios")
        .android_path("android")
        .build();
}
