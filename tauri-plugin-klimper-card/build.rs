// SPDX-License-Identifier: AGPL-3.0-or-later
const COMMANDS: &[&str] = &[
    "card_status",
    "card_sign_in",
    "card_sign_out",
    "card_onboarding",
    "card_send_email_code",
    "card_register",
    "card_accept_terms",
    "card_answer_source_of_funds",
    "card_create_account",
    "card_send_phone_code",
    "card_verify_phone",
    "card_order_card",
    "card_account",
    "card_transactions",
    "card_set_frozen",
    "card_details",
];

/// The card settings are read with `option_env!` (`src/issuer.rs`, `from_build`, and the
/// issuer's own `Settings::from_build`): a change to any of them must rebuild the plugin.
const SETTINGS: &[&str] = &[
    "KLIMPER_CARD_ISSUER",
    "KLIMPER_CARD_API",
    "KLIMPER_CARD_APP_NAME",
    "KLIMPER_CARD_DOMAIN",
    "KLIMPER_CARD_URI",
    "KLIMPER_CARD_CHAIN_ID",
];

fn main() {
    for name in SETTINGS {
        println!("cargo:rerun-if-env-changed={name}");
    }
    tauri_plugin::Builder::new(COMMANDS).build();
}
