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
    "cancel_approval",
    "push_status",
];

fn main() {
    // The push gateway is compiled in (src/push.rs); changing it must rebuild the plugin.
    println!("cargo:rerun-if-env-changed=KLIMPER_PUSH_GATEWAY_URL");
    println!("cargo:rerun-if-env-changed=KLIMPER_PUSH_GATEWAY_KEY");
    tauri_plugin::Builder::new(COMMANDS)
        .ios_path("ios")
        .android_path("android")
        .build();
}
