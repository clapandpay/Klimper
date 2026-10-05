// SPDX-License-Identifier: AGPL-3.0-or-later
const COMMANDS: &[&str] = &[
    "status",
    "inspect_pairing_uri",
    "approve_pairing",
    "cancel_pairing",
    "confirm_sas",
    "list_sessions",
    "end_session",
    "rotate_session_keys",
    "set_dapp_policy",
    "pending_approvals",
    "resolve_approval",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
