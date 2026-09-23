// SPDX-License-Identifier: AGPL-3.0-or-later
const COMMANDS: &[&str] = &["status", "enroll", "unlock", "lock", "remove"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).ios_path("ios").build();
}
