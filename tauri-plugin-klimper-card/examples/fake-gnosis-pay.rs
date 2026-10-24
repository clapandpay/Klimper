// SPDX-License-Identifier: AGPL-3.0-or-later
//! A fake Gnosis Pay on <http://127.0.0.1:8787> for trying the card screens without a
//! sandbox registration (docs/card.md, "Running it locally"). The e-mail and phone codes are
//! always 123456. Its own controls:
//!
//! * `curl -X POST http://127.0.0.1:8787/fake/kyc-approved`: KYC passes.
//! * `curl -X POST http://127.0.0.1:8787/fake/onboarded`: skip onboarding, card and money ready.
//! * `curl -X POST http://127.0.0.1:8787/fake/pay -H 'content-type: application/json'
//!   -d '{"merchant":"Bakery","amount":"4.20"}'`: a card payment.

use tauri_plugin_klimper_card::fake::FakeGnosisPay;

#[tokio::main]
async fn main() {
    let bind = std::env::var("FAKE_GNOSIS_PAY_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let fake = FakeGnosisPay::serve(&bind).await;
    println!("fake Gnosis Pay at {}", fake.api());
    println!(
        "build with KLIMPER_CARD_API={} KLIMPER_CARD_APP_NAME={} KLIMPER_CARD_DOMAIN={}",
        fake.api(),
        tauri_plugin_klimper_card::fake::APP_NAME,
        tauri_plugin_klimper_card::fake::DOMAIN
    );
    tokio::signal::ctrl_c().await.ok();
}
