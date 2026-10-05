// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::Path;

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

/// The push gateway's wake URL and public key, compiled in (src/push.rs). Read from the
/// environment, else from the first of these files in `klimper/`, `KEY=VALUE` a line:
/// `.push-dev.env` (gitignored, written by `scripts/push-dev.sh` for a local stack) and
/// `push-gateway.env` (committed: the deployed gateway, a URL and a public key). Xcode
/// runs cargo without the caller's environment, so the file is the way a phone build
/// gets them. Neither present means push is off and the app polls.
const PUSH_VARS: [&str; 2] = ["KLIMPER_PUSH_GATEWAY_URL", "KLIMPER_PUSH_GATEWAY_KEY"];

fn push_settings() {
    let klimper = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let files = [
        klimper.join(".push-dev.env"),
        klimper.join("push-gateway.env"),
    ];
    for var in PUSH_VARS {
        println!("cargo:rerun-if-env-changed={var}");
    }
    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    if PUSH_VARS
        .iter()
        .all(|var| std::env::var(var).is_ok_and(|v| !v.is_empty()))
    {
        return;
    }
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let mut found = 0;
        for line in text.lines() {
            let line = line.trim();
            if let Some((key, value)) = line.split_once('=')
                && PUSH_VARS.contains(&key.trim())
                && !value.trim().is_empty()
            {
                println!("cargo:rustc-env={}={}", key.trim(), value.trim());
                found += 1;
            }
        }
        if found == PUSH_VARS.len() {
            return;
        }
    }
    println!(
        "cargo:warning=push is off in this build: no gateway in the environment, .push-dev.env or push-gateway.env"
    );
}

fn main() {
    push_settings();
    tauri_plugin::Builder::new(COMMANDS)
        .ios_path("ios")
        .android_path("android")
        .build();
}
