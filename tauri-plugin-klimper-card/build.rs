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
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
