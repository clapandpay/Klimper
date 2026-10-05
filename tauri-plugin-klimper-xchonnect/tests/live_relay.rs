// SPDX-License-Identifier: AGPL-3.0-or-later
//! The real wire: Klimper's relay client and origin verification against xchonnect's own
//! reference relay and a real HTTP origin server (clapandpay TASK-074 AC #5, TASK-075
//! AC #2).
//!
//! `#[ignore]` by default, because they need a relay listening. Run them with:
//!
//! ```sh
//! # in the xchonnect checkout, into a target directory of your choosing
//! cargo build -p xchonnect-relay
//! XCHONNECT_LISTEN=127.0.0.1:18788 XCHONNECT_CREATION=pow \
//!   XCHONNECT_POW_DIFFICULTY=12 XCHONNECT_OHTTP=ephemeral \
//!   ./target/debug/xchonnect-relay &
//!
//! # in klimper/
//! cargo test -p tauri-plugin-klimper-xchonnect --test live_relay -- --ignored --test-threads=1
//! ```
//!
//! `XCHONNECT_TEST_RELAY` overrides the relay URL.
//!
//! The dApp half of the handshake is driven in-process with `xchonnect-core`'s
//! `DappPairing`, so what is under test is Klimper's HTTP: proof-of-work mailbox creation,
//! long polling, acknowledgement, deletion, the origin document rules, and a full pairing
//! through to an answered `rpc.request`.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{SystemTime, UNIX_EPOCH};

use xchonnect_core::b64;
use xchonnect_core::crypto::{Ed25519Seed, MailboxId, OsEntropy, Token};
use xchonnect_core::message::{Message, WalletMeta};
use xchonnect_core::origin::MAX_DOCUMENT_BYTES;
use xchonnect_core::pairing::{DappPairing, DappPairingParams, WalletPairing};
use xchonnect_core::rpc;
use xchonnect_core::uri::{LocalSigner, ParseOptions};

use tauri_plugin_klimper_xchonnect::error::Error;
use tauri_plugin_klimper_xchonnect::origin;
use tauri_plugin_klimper_xchonnect::relay::RelayClient;

fn relay_url() -> String {
    std::env::var("XCHONNECT_TEST_RELAY").unwrap_or_else(|_| "http://127.0.0.1:18788".to_owned())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A one-request-at-a-time HTTP server for the origin document. `respond` turns the
/// requested path into a raw HTTP response, so a test can serve a redirect or an
/// oversize body.
fn serve(respond: impl Fn(&str) -> Vec<u8> + Send + 'static) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0_u8; 2048];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let _ = stream.write_all(&respond(&path));
            let _ = stream.flush();
            let _ = stream.shutdown(std::net::Shutdown::Write);
            drop::<TcpStream>(stream);
        }
    });
    port
}

fn http(status: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

fn origin_document(signer: &LocalSigner, name: &str) -> Vec<u8> {
    format!(
        r#"{{"v":1,"name":"{name}","origin_keys":[{{"kid":"k1","pk":"{}","not_after":"2030-01-01"}}]}}"#,
        b64::encode(&signer.public_key())
    )
    .into_bytes()
}

/// Serve a valid origin document for the dApp's key.
fn origin_server(signer: &LocalSigner) -> u16 {
    let document = origin_document(signer, "Live Test dApp");
    serve(move |path| {
        if path.starts_with("/.well-known/xchonnect.json") {
            http("200 OK", "application/json", &document)
        } else {
            http("404 Not Found", "text/plain", b"no")
        }
    })
}

fn signer() -> LocalSigner {
    LocalSigner::new(Ed25519Seed::from_bytes([7; 32]), "k1").unwrap()
}

/// The whole handshake over a real relay, ending in an answered `rpc.request`.
///
/// Klimper's side is its own [`RelayClient`] and [`origin::verify_pairing_uri`]; the dApp
/// side is `xchonnect-core` driven directly, which is what a browser dApp does through
/// the WASM SDK.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn a_full_pairing_and_a_request_over_a_real_relay() {
    let signer = signer();
    let port = origin_server(&signer);
    let domain = format!("localhost:{port}");
    let developer_mode = ParseOptions {
        developer_mode: true,
    };

    let relay = RelayClient::new(&relay_url(), true).unwrap();
    let info = relay.info().await.expect("GET /v1/info");

    // --- The dApp creates the pairing mailbox (proof-of-work, solved off the UI thread).
    let (pairing_read, pairing_write) =
        (Token::random(&mut OsEntropy), Token::random(&mut OsEntropy));
    let pairing_mailbox = relay
        .create_mailbox(&pairing_read, &pairing_write, None)
        .await
        .expect("mailbox creation with proof-of-work");

    let mut dapp = DappPairing::new(
        &mut OsEntropy,
        now(),
        &signer,
        DappPairingParams {
            relay: &relay_url(),
            domain: &domain,
            pairing_mailbox,
            pairing_write: pairing_write.clone(),
            lifetime_s: 300,
            ticket: None,
            options: developer_mode,
        },
    )
    .unwrap();
    let uri = dapp.uri().to_uri();

    // --- Klimper verifies the URI: a real HTTP fetch of the origin document, then the
    // --- expiry and the Ed25519 origin signature.
    let (verified, was_developer_mode) = origin::verify_pairing_uri(&uri, now(), true)
        .await
        .expect("origin verification");
    assert!(was_developer_mode);
    assert_eq!(verified.dapp_name(), "Live Test dApp");
    assert_eq!(verified.uri().domain, domain);

    // --- Klimper creates its own mailbox and posts the pairing reply.
    let (wallet_read, wallet_write) =
        (Token::random(&mut OsEntropy), Token::random(&mut OsEntropy));
    let wallet_mailbox = relay
        .create_mailbox(&wallet_read, &wallet_write, verified.uri().ticket.as_ref())
        .await
        .unwrap();
    let (pairing, reply) = WalletPairing::reply(
        &mut OsEntropy,
        now(),
        &verified,
        wallet_mailbox,
        wallet_read.clone(),
        wallet_write,
        Some(WalletMeta {
            name: Some("Klimper".to_owned()),
            ..WalletMeta::default()
        }),
    )
    .unwrap();
    relay.post(&reply).await.expect("post the pairing reply");

    // --- The dApp reads it and both sides derive the same six digits.
    let messages = relay
        .fetch(pairing_mailbox, &pairing_read, 5, &info)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1, "exactly the wallet's reply");
    let accepted = dapp.on_reply(now(), &messages[0].1).unwrap();
    assert_eq!(accepted.sas(), pairing.sas(), "the SAS must match");
    assert_eq!(
        accepted
            .wallet_meta()
            .and_then(|m| m.name.clone())
            .as_deref(),
        Some("Klimper")
    );
    relay
        .ack(pairing_mailbox, &pairing_read, &[messages[0].0.clone()])
        .await
        .unwrap();

    // --- The dApp creates its session mailbox and sends session.confirm.
    let (dapp_read, dapp_write) = (Token::random(&mut OsEntropy), Token::random(&mut OsEntropy));
    let dapp_mailbox = relay
        .create_mailbox(&dapp_read, &dapp_write, None)
        .await
        .unwrap();
    let (mut dapp_session, confirm) = accepted
        .confirm(
            &mut OsEntropy,
            now(),
            dapp_mailbox,
            dapp_read.clone(),
            dapp_write,
        )
        .unwrap();
    relay.post(&confirm).await.unwrap();

    // --- Klimper picks it up and has a session, which is NOT yet usable.
    let messages = relay
        .fetch(wallet_mailbox, &wallet_read, 10, &info)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    let mut wallet_session = pairing.on_confirm(now(), &messages[0].1).unwrap();
    assert!(
        !wallet_session.is_active(),
        "unusable until the user confirms the SAS"
    );
    relay
        .ack(wallet_mailbox, &wallet_read, &[messages[0].0.clone()])
        .await
        .unwrap();

    // --- The user confirms, and only now does session.ready go out.
    let ready = wallet_session
        .confirm_sas(&mut OsEntropy, now(), None)
        .unwrap()
        .expect("the wallet sends session.ready");
    assert!(wallet_session.is_active());
    relay.post(&ready).await.unwrap();

    let messages = relay
        .fetch(dapp_mailbox, &dapp_read, 10, &info)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    let inner = dapp_session
        .open(now(), &dapp_mailbox, &messages[0].1)
        .unwrap();
    assert!(matches!(inner.message, Message::SessionReady { .. }));
    dapp_session
        .confirm_sas(&mut OsEntropy, now(), None)
        .unwrap();
    assert!(dapp_session.is_active());
    relay
        .ack(dapp_mailbox, &dapp_read, &[messages[0].0.clone()])
        .await
        .unwrap();

    // --- A request, and an answer.
    let request = rpc::request("chainId", "{}").unwrap();
    let outgoing = dapp_session
        .seal(&mut OsEntropy, now(), request, 600)
        .unwrap();
    let request_id = outgoing.id;
    relay.post(&outgoing).await.unwrap();

    let messages = relay
        .fetch(wallet_mailbox, &wallet_read, 10, &info)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    let inner = wallet_session
        .open(now(), &wallet_mailbox, &messages[0].1)
        .unwrap();
    let Message::RpcRequest { method, params } = inner.message else {
        panic!("expected an rpc.request")
    };
    assert_eq!((method.as_str(), params.as_str()), ("chainId", "{}"));
    assert_eq!(inner.id, request_id);
    relay
        .ack(wallet_mailbox, &wallet_read, &[messages[0].0.clone()])
        .await
        .unwrap();

    let answer = rpc::result(inner.id, "\"testnet11\"").unwrap();
    let outgoing = wallet_session
        .seal(&mut OsEntropy, now(), answer, 600)
        .unwrap();
    relay.post(&outgoing).await.unwrap();

    let messages = relay
        .fetch(dapp_mailbox, &dapp_read, 10, &info)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    let inner = dapp_session
        .open(now(), &dapp_mailbox, &messages[0].1)
        .unwrap();
    match inner.message {
        Message::RpcResponse {
            request_id: id,
            outcome,
        } => {
            assert_eq!(id, request_id);
            assert_eq!(
                outcome,
                xchonnect_core::message::RpcOutcome::Result("\"testnet11\"".to_owned())
            );
        }
        other => panic!("expected an rpc.response, got {}", other.type_name()),
    }

    // --- Deleting the mailbox makes every later call a 404, which Klimper reports as
    // --- "this code was already used or the session is gone".
    relay.delete(wallet_mailbox, &wallet_read).await;
    let error = relay
        .fetch(wallet_mailbox, &wallet_read, 0, &info)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::PairingCodeUsed), "{error}");

    relay.delete(dapp_mailbox, &dapp_read).await;
    relay.delete(pairing_mailbox, &pairing_read).await;
}

/// A wrong read token is a 404, byte-identical to an unknown mailbox, and Klimper does
/// not distinguish them either.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn a_wrong_token_is_indistinguishable_from_a_missing_mailbox() {
    let relay = RelayClient::new(&relay_url(), true).unwrap();
    let info = relay.info().await.unwrap();
    let (read, write) = (Token::random(&mut OsEntropy), Token::random(&mut OsEntropy));
    let mailbox = relay.create_mailbox(&read, &write, None).await.unwrap();

    let wrong = Token::random(&mut OsEntropy);
    let with_wrong_token = relay.fetch(mailbox, &wrong, 0, &info).await.unwrap_err();
    let unknown = relay
        .fetch(MailboxId([9; 16]), &read, 0, &info)
        .await
        .unwrap_err();
    assert!(matches!(with_wrong_token, Error::PairingCodeUsed));
    assert!(matches!(unknown, Error::PairingCodeUsed));
    assert_eq!(with_wrong_token.to_string(), unknown.to_string());

    relay.delete(mailbox, &read).await;
}

/// The origin document rules, against a real HTTP server: a redirect is not followed, an
/// oversize body is refused while it streams, and a 404 is a failure. None of them has a
/// way to continue (spec 6.1, 6.3 step 2).
#[tokio::test]
#[ignore = "starts a local HTTP server; grouped with the live tests"]
async fn the_origin_document_rules_hold_against_a_real_server() {
    let developer_mode = ParseOptions {
        developer_mode: true,
    };

    // A 302 to a valid document elsewhere: refused, because the signature is bound to the
    // domain in the URI and not to wherever it points.
    let good = origin_server(&signer());
    let redirect_port = serve(move |_| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: http://localhost:{good}/.well-known/xchonnect.json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    });
    let error = origin::fetch(&format!("localhost:{redirect_port}"), developer_mode)
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Origin(ref m) if m.contains("redirected")),
        "{error}"
    );

    // One byte over the cap.
    let oversize_port = serve(|_| {
        http(
            "200 OK",
            "application/json",
            &vec![b'x'; MAX_DOCUMENT_BYTES + 1],
        )
    });
    let error = origin::fetch(&format!("localhost:{oversize_port}"), developer_mode)
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Origin(ref m) if m.contains("more than")),
        "{error}"
    );

    // No document at all.
    let missing_port = serve(|_| http("404 Not Found", "text/plain", b"no"));
    let error = origin::fetch(&format!("localhost:{missing_port}"), developer_mode)
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Origin(ref m) if m.contains("404")),
        "{error}"
    );

    // A document that is valid JSON but names no usable key.
    let empty_port = serve(|_| http("200 OK", "application/json", br#"{"v":1,"name":"x"}"#));
    assert!(
        origin::fetch(&format!("localhost:{empty_port}"), developer_mode)
            .await
            .is_err()
    );

    // And the good one still parses, so the failures above are the rules and not the
    // server being broken.
    let document = origin::fetch(&format!("localhost:{good}"), developer_mode)
        .await
        .unwrap();
    assert_eq!(document.name, "Live Test dApp");
}

/// A release build refuses a developer-mode URI even when the relay and the origin
/// document are perfectly reachable (TASK-074 AC #5).
#[tokio::test]
#[ignore = "starts a local HTTP server; grouped with the live tests"]
async fn developer_mode_is_refused_when_it_is_not_allowed() {
    let signer = signer();
    let port = origin_server(&signer);
    let dapp = DappPairing::new(
        &mut OsEntropy,
        now(),
        &signer,
        DappPairingParams {
            relay: "http://127.0.0.1:18788",
            domain: &format!("localhost:{port}"),
            pairing_mailbox: MailboxId([1; 16]),
            pairing_write: Token::from_bytes([2; 32]),
            lifetime_s: 300,
            ticket: None,
            options: ParseOptions {
                developer_mode: true,
            },
        },
    )
    .unwrap();
    let error = origin::verify_pairing_uri(&dapp.uri().to_uri(), now(), false)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::DeveloperModeRefused), "{error}");
}
