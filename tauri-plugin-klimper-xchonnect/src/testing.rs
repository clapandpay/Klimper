// SPDX-License-Identifier: AGPL-3.0-or-later
//! Test fixtures: a complete dApp/wallet handshake driven with `xchonnect-core`, so the
//! Klimper side can be exercised without a relay or a webview.
#![cfg(test)]
#![allow(clippy::unwrap_used)]

use xchonnect_core::crypto::{Ed25519Seed, MailboxId, OsEntropy, Token};
use xchonnect_core::origin::OriginDocument;
use xchonnect_core::pairing::{DappPairing, DappPairingParams, VerifiedUri, WalletPairing};
use xchonnect_core::session::Session;
use xchonnect_core::uri::{LocalSigner, PairingUri, ParseOptions};

pub const NOW: u64 = 1_790_000_000;
pub const DOMAIN: &str = "pengui.xyz";

pub struct Fixture {
    /// `xchonnect-core`'s deterministic `TestEntropy` is gated behind its own
    /// `test-vectors` feature, which must never be enabled from here, so these fixtures
    /// use real randomness. Nothing in them depends on a fixed byte stream.
    pub rng: OsEntropy,
    pub signer: LocalSigner,
    pub doc: OriginDocument,
}

pub fn fixture() -> Fixture {
    let signer = LocalSigner::new(Ed25519Seed::from_bytes([1; 32]), "k1").unwrap();
    let json = format!(
        r#"{{"v":1,"name":"Pengui","origin_keys":[{{"kid":"k1","pk":"{}","not_after":"2030-01-01"}}]}}"#,
        xchonnect_core::b64::encode(&signer.public_key())
    );
    Fixture {
        rng: OsEntropy,
        signer,
        doc: OriginDocument::parse(json.as_bytes()).unwrap(),
    }
}

/// The dApp's signed pairing URI as it would appear in a QR code.
pub fn pairing_uri(f: &mut Fixture) -> (DappPairing, String) {
    let dapp = DappPairing::new(
        &mut f.rng,
        NOW,
        &f.signer,
        DappPairingParams {
            relay: "https://relay.example",
            domain: DOMAIN,
            pairing_mailbox: MailboxId([1; 16]),
            pairing_write: Token::from_bytes([2; 32]),
            lifetime_s: 300,
            ticket: None,
            options: ParseOptions::default(),
        },
    )
    .unwrap();
    let uri = dapp.uri().to_uri();
    (dapp, uri)
}

/// Wallet side of the handshake up to (but not including) the SAS confirmation.
///
/// Returns the dApp's session, the wallet's session and the wallet's own mailbox id. At
/// this point `session.ready` has **not** been sent: the wallet's session must be
/// unusable (xchonnect spec 6.3 step 8).
pub fn handshake_without_sas(f: &mut Fixture) -> (Session, Session, MailboxId) {
    let (mut dapp, uri) = pairing_uri(f);
    let parsed = PairingUri::parse(&uri, ParseOptions::default()).unwrap();
    let verified = VerifiedUri::new(parsed, &f.doc, NOW + 5).unwrap();
    let wallet_mailbox = MailboxId([10; 16]);
    let (pairing, reply) = WalletPairing::reply(
        &mut f.rng,
        NOW + 5,
        &verified,
        wallet_mailbox,
        Token::from_bytes([10; 32]),
        Token::from_bytes([11; 32]),
        None,
    )
    .unwrap();
    let accepted = dapp.on_reply(NOW + 6, &reply.envelope).unwrap();
    assert_eq!(accepted.sas(), pairing.sas());
    let (dapp_session, confirm) = accepted
        .confirm(
            &mut f.rng,
            NOW + 7,
            MailboxId([20; 16]),
            Token::from_bytes([21; 32]),
            Token::from_bytes([22; 32]),
        )
        .unwrap();
    let wallet_session = pairing.on_confirm(NOW + 8, &confirm.envelope).unwrap();
    assert!(!wallet_session.is_active());
    (dapp_session, wallet_session, wallet_mailbox)
}

/// A fully paired pair of sessions (both sides confirmed the SAS).
pub fn paired_sessions() -> (Session, Session) {
    let mut f = fixture();
    let (mut dapp_session, mut wallet_session, _) = handshake_without_sas(&mut f);
    let ready = wallet_session
        .confirm_sas(&mut f.rng, NOW + 9, None)
        .unwrap()
        .unwrap();
    dapp_session
        .open(NOW + 10, &MailboxId([20; 16]), &ready.envelope)
        .unwrap();
    dapp_session
        .confirm_sas(&mut f.rng, NOW + 10, None)
        .unwrap();
    assert!(dapp_session.is_active() && wallet_session.is_active());
    (dapp_session, wallet_session)
}
