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
//! The push test also needs a gateway, and a relay that will call one on loopback:
//!
//! ```sh
//! XCHONNECT_GATEWAY_KEYS=$(openssl rand -base64 32 | tr '+/' '-_' | tr -d '=') \
//!   XCHONNECT_GATEWAY_LISTEN=127.0.0.1:18789 ./target/debug/xchonnect-gateway &
//! # the relay as above, plus:
//! #   XCHONNECT_GATEWAY_POLICY=open XCHONNECT_DEV_ALLOW_INSECURE_GATEWAYS=true
//! XCHONNECT_TEST_GATEWAY=http://127.0.0.1:18789 cargo test ... a_request_wakes
//! ```
//!
//! Without `XCHONNECT_TEST_GATEWAY` that one test passes without doing anything.
//!
//! The dApp half of the handshake is driven in-process with `xchonnect-core`'s
//! `DappPairing`, so what is under test is Klimper's HTTP: proof-of-work mailbox creation,
//! long polling, acknowledgement, deletion, the origin document rules, and a full pairing
//! through to an answered `rpc.request`.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

use xchonnect_core::b64;
use xchonnect_core::crypto::{Ed25519Seed, MailboxId, OsEntropy, Token};
use xchonnect_core::message::{Message, RotatePhase, WalletMeta};
use xchonnect_core::origin::MAX_DOCUMENT_BYTES;
use xchonnect_core::pairing::{DappPairing, DappPairingParams, WalletPairing};
use xchonnect_core::rpc;
use xchonnect_core::session::Session;
use xchonnect_core::uri::{LocalSigner, ParseOptions};

use tauri_plugin_klimper_xchonnect::approval::{Approval, ApprovalBridge, ApprovalKind, Emitter};
use tauri_plugin_klimper_xchonnect::error::Error;
use tauri_plugin_klimper_xchonnect::origin;
use tauri_plugin_klimper_xchonnect::push::{GatewayConfig, Kind, PushHandle};
use tauri_plugin_klimper_xchonnect::relay::{RelayClient, RelayInfo};
use tauri_plugin_klimper_xchonnect::requests::{AnswerInputs, Progress};
use tauri_plugin_klimper_xchonnect::session_loop::{Answerer, SessionLoop, default_answerer};
use tauri_plugin_klimper_xchonnect::store::{
    MemorySecrets, SessionRecord, SessionStore, StoredPermissions, StoredSession,
};
use xchonnect_core::message::RpcOutcome;
use xchonnect_wallet_kit::RpcError;

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

/// Both ends of a session paired over a real relay, the SAS confirmed on both sides.
struct Paired {
    relay: RelayClient,
    info: RelayInfo,
    domain: String,
    dapp_session: Session,
    dapp_mailbox: MailboxId,
    dapp_read: Token,
    pairing_mailbox: MailboxId,
    pairing_read: Token,
    wallet_session: Session,
    wallet_mailbox: MailboxId,
    wallet_read: Token,
}

/// The handshake over a real relay, asserting every step on the way.
///
/// Klimper's side is its own [`RelayClient`] and [`origin::verify_pairing_uri`]; the dApp
/// side is `xchonnect-core` driven directly, which is what a browser dApp does through
/// the WASM SDK.
async fn pair_over_relay() -> Paired {
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

    Paired {
        relay,
        info,
        domain,
        dapp_session,
        dapp_mailbox,
        dapp_read,
        pairing_mailbox,
        pairing_read,
        wallet_session,
        wallet_mailbox,
        wallet_read,
    }
}

/// The whole handshake over a real relay, ending in an answered `rpc.request`.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn a_full_pairing_and_a_request_over_a_real_relay() {
    let Paired {
        relay,
        info,
        mut dapp_session,
        dapp_mailbox,
        dapp_read,
        pairing_mailbox,
        pairing_read,
        mut wallet_session,
        wallet_mailbox,
        wallet_read,
        ..
    } = pair_over_relay().await;

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

/// A real [`SessionLoop`] running the wallet half of a [`Paired`] session.
struct Running {
    _dir: tempfile::TempDir,
    store: Arc<SessionStore>,
    /// The `state` of every session event the loop emitted, in order.
    events: Arc<Mutex<Vec<String>>>,
    end_requested: Arc<AtomicBool>,
    interrupt: Arc<Notify>,
    approvals: Arc<ApprovalBridge>,
    task: tokio::task::JoinHandle<()>,
}

const SESSION_ID: &str = "live";

/// Store `wallet_session` as a confirmed session for `dapp` and start its loop, with a
/// ticker standing in for the app's. `rotate` asks for a key rotation before the first
/// poll, as `rotate_session_keys` does.
fn start_loop(
    wallet_session: &Session,
    dapp: &str,
    origin_kid: Option<&str>,
    rotate: bool,
) -> Running {
    start_loop_with(wallet_session, dapp, origin_kid, rotate, default_answerer())
}

/// [`start_loop`] with requests answered by `answer`.
fn start_loop_with(
    wallet_session: &Session,
    dapp: &str,
    origin_kid: Option<&str>,
    rotate: bool,
    answer: Answerer,
) -> Running {
    start_loop_push(
        wallet_session,
        dapp,
        origin_kid,
        rotate,
        answer,
        Arc::new(PushHandle::default()),
    )
}

/// [`start_loop_with`] with the phone's push state, for the wake-up test.
fn start_loop_push(
    wallet_session: &Session,
    dapp: &str,
    origin_kid: Option<&str>,
    rotate: bool,
    answer: Answerer,
    push: Arc<PushHandle>,
) -> Running {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(SessionStore::open(dir.path(), Box::new(MemorySecrets::default())).unwrap());
    let mut stored = StoredSession {
        record: SessionRecord {
            id: SESSION_ID.to_owned(),
            fingerprint: 1,
            dapp: dapp.to_owned(),
            dapp_name: "Live Test dApp".to_owned(),
            relay: relay_url(),
            chain_id: "testnet11".to_owned(),
            created_at: now(),
            state_version: 0,
            developer_mode: true,
            sas_confirmed: true,
            needs_repair: false,
            permissions: StoredPermissions::default(),
            allow_agg_sig_unsafe: false,
            allow_unknown_contracts: false,
            origin_kid: origin_kid.map(str::to_owned),
            revoked: false,
            push: None,
            own_key: None,
        },
        session_hex: hex::encode(wallet_session.to_bytes().unwrap()),
    };
    store.save(&mut stored).unwrap();

    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = events.clone();
    let emit: Emitter = Arc::new(move |_event, payload| {
        sink.lock()
            .unwrap()
            .push(payload["state"].as_str().unwrap_or_default().to_owned());
    });
    let end_requested = Arc::new(AtomicBool::new(false));
    let interrupt = Arc::new(Notify::new());
    let (wake_tx, wake) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        while wake_tx.send(()).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });
    let approvals = Arc::new(ApprovalBridge::new(emit.clone()));
    let message_loop = SessionLoop {
        state: Arc::new(tokio::sync::Mutex::new(sage::Sage::new(dir.path(), true))),
        store: store.clone(),
        approvals: approvals.clone(),
        emit,
        relay: RelayClient::new(&relay_url(), true).unwrap(),
        info: RelayInfo::default(),
        end_requested: end_requested.clone(),
        rotate_requested: Arc::new(AtomicBool::new(rotate)),
        permissions_changed: Arc::new(AtomicBool::new(false)),
        interrupt: interrupt.clone(),
        wake,
        passed: Arc::new(Notify::new()),
        push,
        answer,
    };
    let task = tokio::spawn(message_loop.run(stored));
    Running {
        _dir: dir,
        store,
        events,
        end_requested,
        interrupt,
        approvals,
        task,
    }
}

/// Disconnecting reaches the dApp at once, not after the long poll the loop is sitting
/// in: the poll is cut short, `session.end` is posted, and this side's mailbox and stored
/// state are gone.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn a_disconnect_does_not_wait_for_the_long_poll() {
    let Paired {
        relay,
        info,
        domain,
        mut dapp_session,
        dapp_mailbox,
        dapp_read,
        pairing_mailbox,
        pairing_read,
        wallet_session,
        wallet_mailbox,
        wallet_read,
    } = pair_over_relay().await;

    // The origin document still lists `k1`, so the check the loop makes on starting
    // passes and the session stays up until the user ends it.
    let running = start_loop(&wallet_session, &domain, Some("k1"), false);
    let Running {
        store,
        events,
        end_requested,
        interrupt,
        task: running,
        ..
    } = running;

    // Let it settle into the long poll (up to 20 s on this relay), then disconnect.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let asked = Instant::now();
    end_requested.store(true, Ordering::SeqCst);
    interrupt.notify_one();
    tokio::time::timeout(Duration::from_secs(5), running)
        .await
        .expect("the loop ends without waiting for its long poll")
        .unwrap();
    assert!(asked.elapsed() < Duration::from_secs(5));
    assert_eq!(*events.lock().unwrap(), vec!["ended".to_owned()]);

    // The dApp is told.
    let messages = relay
        .fetch(dapp_mailbox, &dapp_read, 5, &info)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1, "exactly the session.end");
    let inner = dapp_session
        .open(now(), &dapp_mailbox, &messages[0].1)
        .unwrap();
    assert!(matches!(inner.message, Message::SessionEnd { .. }));

    // And nothing is left on this side.
    assert!(store.list(1).unwrap().is_empty());
    let error = relay
        .fetch(wallet_mailbox, &wallet_read, 0, &info)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::PairingCodeUsed), "{error}");

    relay.delete(dapp_mailbox, &dapp_read).await;
    relay.delete(pairing_mailbox, &pairing_read).await;
}

/// Spec 9.2.1 with Klimper as the initiator: the user renews the keys, the dApp accepts,
/// both sides switch to epoch 1, the session keeps answering on the new mailboxes, and the
/// wallet's old mailbox is drained and deleted.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn a_wallet_key_rotation_keeps_the_session_working() {
    let Paired {
        relay,
        info,
        domain,
        mut dapp_session,
        dapp_mailbox,
        dapp_read,
        pairing_mailbox,
        pairing_read,
        wallet_session,
        wallet_mailbox,
        wallet_read,
    } = pair_over_relay().await;
    let running = start_loop(&wallet_session, &domain, Some("k1"), true);

    // The offer arrives on the dApp's current mailbox, under epoch 0.
    let messages = relay
        .fetch(dapp_mailbox, &dapp_read, 10, &info)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1, "exactly the rotation offer");
    let inner = dapp_session
        .open(now(), &dapp_mailbox, &messages[0].1)
        .unwrap();
    let Message::SessionRotate(offer) = inner.message else {
        panic!("expected session.rotate, got {}", inner.message.type_name())
    };
    assert_eq!((offer.phase, offer.epoch), (RotatePhase::Offer, 1));
    relay
        .ack(dapp_mailbox, &dapp_read, &[messages[0].0.clone()])
        .await
        .unwrap();

    // The dApp accepts with a mailbox of its own and switches.
    let (new_read, new_write) = (Token::random(&mut OsEntropy), Token::random(&mut OsEntropy));
    let new_mailbox = relay
        .create_mailbox(&new_read, &new_write, None)
        .await
        .unwrap();
    let (accept, abandoned) = dapp_session
        .accept_rotation(
            &mut OsEntropy,
            now(),
            &offer,
            new_mailbox,
            new_read.clone(),
            new_write,
        )
        .unwrap();
    assert!(abandoned.is_none());
    relay.post(&accept).await.unwrap();
    assert_eq!(dapp_session.epoch(), 1);

    // Klimper switches on the accept and pings the dApp's new mailbox (spec 9.2.1 step 4),
    // which is what lets the dApp give up its old one.
    let messages = relay
        .fetch(new_mailbox, &new_read, 10, &info)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1, "the wallet's ping under the new keys");
    let inner = dapp_session
        .open(now(), &new_mailbox, &messages[0].1)
        .unwrap();
    assert!(matches!(inner.message, Message::SessionPing));
    relay
        .ack(new_mailbox, &new_read, &[messages[0].0.clone()])
        .await
        .unwrap();
    let retired = dapp_session.finish_drain().expect("the dApp may retire");
    relay.delete(retired.mailbox, &retired.read_token).await;

    // The session still works, now in epoch 1, in both directions.
    let ping = dapp_session
        .seal(&mut OsEntropy, now(), Message::SessionPing, 60)
        .unwrap();
    relay.post(&ping).await.unwrap();
    let messages = relay
        .fetch(new_mailbox, &new_read, 10, &info)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);
    let inner = dapp_session
        .open(now(), &new_mailbox, &messages[0].1)
        .unwrap();
    assert!(matches!(inner.message, Message::SessionPong));

    // The new keys were persisted, and the wallet's epoch-0 mailbox was drained and
    // deleted (one more pass of the loop at most).
    let stored = running.store.load(1, SESSION_ID).unwrap();
    let saved = Session::from_bytes(&stored.session_bytes().unwrap()).unwrap();
    assert_eq!(saved.epoch(), 1);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match relay.fetch(wallet_mailbox, &wallet_read, 0, &info).await {
            Err(Error::PairingCodeUsed) => break,
            _ if Instant::now() > deadline => panic!("the old mailbox was never deleted"),
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    let events = running.events.lock().unwrap().clone();
    assert_eq!(events, vec!["rotating".to_owned(), "rotated".to_owned()]);

    running.end_requested.store(true, Ordering::SeqCst);
    running.interrupt.notify_one();
    tokio::time::timeout(Duration::from_secs(5), running.task)
        .await
        .unwrap()
        .unwrap();
    relay.delete(new_mailbox, &new_read).await;
    relay.delete(pairing_mailbox, &pairing_read).await;
}

/// Spec 6.1: the dApp removed the origin key the pairing code was signed with. The loop's
/// fresh fetch of the origin document finds it gone, ends the session, erases its secrets
/// and keeps the record marked as revoked.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn a_removed_origin_key_revokes_the_session() {
    let Paired {
        relay,
        info,
        mut dapp_session,
        dapp_mailbox,
        dapp_read,
        pairing_mailbox,
        pairing_read,
        wallet_session,
        wallet_mailbox,
        wallet_read,
        ..
    } = pair_over_relay().await;
    // The same dApp after rotating its origin keys: `k1` is gone, only `k2` is listed.
    let rotated_document = format!(
        r#"{{"v":1,"name":"Live Test dApp","origin_keys":[{{"kid":"k2","pk":"{}","not_after":"2030-01-01"}}]}}"#,
        b64::encode(&signer().public_key())
    )
    .into_bytes();
    let port = serve(move |path| {
        if path.starts_with("/.well-known/xchonnect.json") {
            http("200 OK", "application/json", &rotated_document)
        } else {
            http("404 Not Found", "text/plain", b"no")
        }
    });
    let running = start_loop(
        &wallet_session,
        &format!("localhost:{port}"),
        Some("k1"),
        false,
    );

    tokio::time::timeout(Duration::from_secs(10), running.task)
        .await
        .expect("a revoked session's loop ends")
        .unwrap();
    assert_eq!(*running.events.lock().unwrap(), vec!["revoked".to_owned()]);

    // Marked, with nothing secret left.
    let stored = running.store.load(1, SESSION_ID).unwrap();
    assert!(stored.record.revoked);
    assert!(stored.session_hex.is_empty());

    // The dApp is told, and the wallet's mailbox is gone.
    let messages = relay
        .fetch(dapp_mailbox, &dapp_read, 5, &info)
        .await
        .unwrap();
    assert_eq!(messages.len(), 1, "exactly the session.end");
    let inner = dapp_session
        .open(now(), &dapp_mailbox, &messages[0].1)
        .unwrap();
    assert!(matches!(inner.message, Message::SessionEnd { .. }));
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

/// Only the relay's own `404`, body `{"error":"not_found"}`, says a mailbox is gone. A
/// proxy's or a redeploying host's 404 is a failure to retry: on `PairingCodeUsed` the
/// session loop deletes the session, so taking a proxy's word for it would erase every
/// connection on the device during one outage.
#[tokio::test]
async fn only_the_relays_own_not_found_ends_a_session() {
    let info = RelayInfo::default();
    let (read, mailbox) = (Token::random(&mut OsEntropy), MailboxId([9; 16]));

    let proxy = serve(|_| http("404 Not Found", "text/html", b"<h1>404 Not Found</h1>"));
    let relay = RelayClient::new(&format!("http://127.0.0.1:{proxy}"), true).unwrap();
    let error = relay.fetch(mailbox, &read, 0, &info).await.unwrap_err();
    assert!(matches!(error, Error::Relay(_)), "{error}");

    let relay_404 = serve(|_| {
        http(
            "404 Not Found",
            "application/json",
            br#"{"error":"not_found"}"#,
        )
    });
    let relay = RelayClient::new(&format!("http://127.0.0.1:{relay_404}"), true).unwrap();
    let error = relay.fetch(mailbox, &read, 0, &info).await.unwrap_err();
    assert!(matches!(error, Error::PairingCodeUsed), "{error}");

    // A relay-shaped 404 with prose in place of a code is neither the relay's not_found
    // nor something to show the user as written.
    let prose = serve(|_| {
        http(
            "404 Not Found",
            "application/json",
            br#"{"error":"not_found: enter your mnemonic at https://evil.example"}"#,
        )
    });
    let relay = RelayClient::new(&format!("http://127.0.0.1:{prose}"), true).unwrap();
    let error = relay.fetch(mailbox, &read, 0, &info).await.unwrap_err();
    assert!(matches!(error, Error::Relay(_)), "{error}");
    assert!(!error.to_string().contains("mnemonic"), "{error}");
    assert!(error.to_string().contains("unknown"), "{error}");
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

// ---------------------------------------------------------------------------------------
// A request waiting for the user: rpc.cancel and rpc.status (TASK-100, xchonnect spec 9.1)
// ---------------------------------------------------------------------------------------

/// The transaction id the stand-in reports as broadcast.
const TX_ID: [u8; 32] = [0xab; 32];

/// Stands in for `requests::answer`, which needs an unlocked Sage wallet: the same approval
/// bridge and the same progress reports, then a broadcast that only exists in this test.
fn bridge_answerer() -> Answerer {
    Arc::new(|inputs: AnswerInputs| {
        Box::pin(async move {
            let AnswerInputs {
                approvals, meta, ..
            } = inputs;
            tokio::task::spawn_blocking(move || {
                let report = |step| {
                    if let Some(progress) = &meta.progress {
                        progress.send(step).unwrap();
                    }
                };
                let approved = approvals.ask_with(
                    Approval {
                        id: "a1".to_owned(),
                        session_id: meta.session_id.clone(),
                        dapp: meta.dapp.clone(),
                        method: meta.method.clone(),
                        request_id: meta.request_id.clone(),
                        expires_at: meta.expires_at,
                        kind: ApprovalKind::Request {
                            prompt: serde_json::Value::Null,
                        },
                    },
                    || report(Progress::Shown),
                );
                if !approved {
                    return Err(RpcError {
                        code: rpc::codes::USER_REJECTED,
                        message: "user rejected request".to_owned(),
                        data: None,
                    });
                }
                report(Progress::Approved);
                report(Progress::Broadcast(TX_ID));
                Ok(format!(
                    r#"{{"transactionId":"0x{}","status":"pending"}}"#,
                    hex::encode(TX_ID)
                ))
            })
            .await
            .unwrap()
        })
    })
}

/// Everything the wallet sent the dApp until `stop` holds for the last message, in order.
async fn read_dapp_until(paired: &mut Paired, stop: impl Fn(&Message) -> bool) -> Vec<Message> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        let messages = paired
            .relay
            .fetch(paired.dapp_mailbox, &paired.dapp_read, 5, &paired.info)
            .await
            .unwrap();
        for (id, envelope) in messages {
            let inner = paired
                .dapp_session
                .open(now(), &paired.dapp_mailbox, &envelope)
                .unwrap();
            paired
                .relay
                .ack(paired.dapp_mailbox, &paired.dapp_read, &[id])
                .await
                .unwrap();
            let done = stop(&inner.message);
            seen.push(inner.message);
            if done {
                return seen;
            }
        }
    }
    panic!(
        "the wallet never finished: {:?}",
        seen.iter().map(Message::type_name).collect::<Vec<_>>()
    );
}

/// Send a payment request from the dApp; returns its id.
async fn post_send(paired: &mut Paired) -> [u8; 16] {
    let request =
        rpc::request("chia_send", r#"{"address":"txch1","amount":"1","fee":"0"}"#).unwrap();
    let outgoing = paired
        .dapp_session
        .seal(&mut OsEntropy, now(), request, 600)
        .unwrap();
    paired.relay.post(&outgoing).await.unwrap();
    outgoing.id
}

fn is_response(message: &Message) -> bool {
    matches!(message, Message::RpcResponse { .. })
}

fn is_shown(message: &Message) -> bool {
    matches!(message, Message::RpcStatus { state, .. } if state == "shown")
}

fn error_code(message: &Message) -> Option<i64> {
    match message {
        Message::RpcResponse {
            outcome: RpcOutcome::Error(error),
            ..
        } => Some(error.code),
        _ => None,
    }
}

async fn stop(running: Running, paired: Paired) {
    running.end_requested.store(true, Ordering::SeqCst);
    running.interrupt.notify_one();
    tokio::time::timeout(Duration::from_secs(10), running.task)
        .await
        .unwrap()
        .unwrap();
    paired
        .relay
        .delete(paired.dapp_mailbox, &paired.dapp_read)
        .await;
    paired
        .relay
        .delete(paired.pairing_mailbox, &paired.pairing_read)
        .await;
}

/// The user approves: the dApp sees received, shown, approved and broadcast (with the
/// transaction id), in that order, and then the answer.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn progress_reaches_the_dapp_before_the_answer() {
    let mut paired = pair_over_relay().await;
    let running = start_loop_with(
        &paired.wallet_session,
        &paired.domain,
        Some("k1"),
        false,
        bridge_answerer(),
    );
    let request_id = post_send(&mut paired).await;

    read_dapp_until(&mut paired, is_shown).await;
    running.approvals.resolve("a1", true).unwrap();
    let seen = read_dapp_until(&mut paired, is_response).await;

    let states: Vec<&str> = seen
        .iter()
        .filter_map(|m| match m {
            Message::RpcStatus { state, .. } => Some(state.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(states, ["approved", "broadcast"]);
    assert!(seen.iter().any(|m| matches!(
        m,
        Message::RpcStatus { request_id: id, tx_id: Some(tx), .. } if *id == request_id && *tx == TX_ID
    )));
    match seen.last().unwrap() {
        Message::RpcResponse {
            request_id: id,
            outcome: RpcOutcome::Result(result),
        } => {
            assert_eq!(*id, request_id);
            assert!(result.contains(&hex::encode(TX_ID)));
        }
        other => panic!("expected the result, got {}", other.type_name()),
    }
    stop(running, paired).await;
}

/// The dApp withdraws a request the user is looking at: the prompt goes away at once and
/// the answer is 4102, not a rejection.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn a_dapp_cancel_withdraws_the_prompt_and_is_answered_4102() {
    let mut paired = pair_over_relay().await;
    let running = start_loop_with(
        &paired.wallet_session,
        &paired.domain,
        Some("k1"),
        false,
        bridge_answerer(),
    );
    let request_id = post_send(&mut paired).await;

    read_dapp_until(&mut paired, is_shown).await;
    assert_eq!(running.approvals.outstanding().len(), 1);
    let cancel = paired
        .dapp_session
        .seal(
            &mut OsEntropy,
            now(),
            Message::RpcCancel { request_id },
            600,
        )
        .unwrap();
    paired.relay.post(&cancel).await.unwrap();

    let seen = read_dapp_until(&mut paired, is_response).await;
    assert_eq!(
        error_code(seen.last().unwrap()),
        Some(rpc::codes::REQUEST_CANCELLED)
    );
    assert!(
        running.approvals.outstanding().is_empty(),
        "the prompt is gone"
    );
    assert!(
        !seen.iter().any(|m| matches!(m, Message::RpcCancel { .. })),
        "the dApp knows it cancelled"
    );

    // The session keeps working afterwards.
    let ping = paired
        .dapp_session
        .seal(&mut OsEntropy, now(), Message::SessionPing, 60)
        .unwrap();
    paired.relay.post(&ping).await.unwrap();
    read_dapp_until(&mut paired, |m| matches!(m, Message::SessionPong)).await;
    stop(running, paired).await;
}

/// TASK-125.05: a mailbox the relay no longer knows ends the connection only after it said
/// so three times, a pause apart, not at the first answer (a relay being redeployed).
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn a_gone_mailbox_ends_the_connection_only_after_three_answers() {
    let paired = pair_over_relay().await;
    let running = start_loop(&paired.wallet_session, &paired.domain, Some("k1"), false);
    // Let the first pass happen, then take the wallet's mailbox away.
    tokio::time::sleep(Duration::from_millis(300)).await;
    paired
        .relay
        .delete(paired.wallet_mailbox, &paired.wallet_read)
        .await;
    let started = Instant::now();
    tokio::time::timeout(Duration::from_secs(30), running.task)
        .await
        .expect("the loop ends")
        .unwrap();
    assert!(
        started.elapsed() >= Duration::from_secs(9),
        "not at the first not_found: {:?}",
        started.elapsed()
    );
    assert_eq!(*running.events.lock().unwrap(), vec!["ended".to_owned()]);
}

/// TASK-125.03: a website's read is answered at once while a signing request still waits for
/// the user, so its wallet info never waits behind a prompt.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn a_read_is_answered_while_a_prompt_waits() {
    let mut paired = pair_over_relay().await;
    let bridge = bridge_answerer();
    let answer: Answerer = Arc::new(move |inputs: AnswerInputs| {
        if inputs.meta.method == "chainId" {
            Box::pin(async { Ok(r#""testnet11""#.to_owned()) })
        } else {
            bridge(inputs)
        }
    });
    let running = start_loop_with(
        &paired.wallet_session,
        &paired.domain,
        Some("k1"),
        false,
        answer,
    );
    let send_id = post_send(&mut paired).await;
    read_dapp_until(&mut paired, is_shown).await;

    let read = paired
        .dapp_session
        .seal(
            &mut OsEntropy,
            now(),
            rpc::request("chainId", "{}").unwrap(),
            600,
        )
        .unwrap();
    paired.relay.post(&read).await.unwrap();
    let seen = read_dapp_until(&mut paired, is_response).await;
    assert!(
        matches!(
            seen.last().unwrap(),
            Message::RpcResponse { request_id, outcome: RpcOutcome::Result(result) }
                if *request_id == read.id && result == r#""testnet11""#
        ),
        "the read is answered first"
    );
    assert_eq!(
        running.approvals.outstanding().len(),
        1,
        "the send still waits for the user"
    );

    running.approvals.cancel("a1").unwrap();
    read_dapp_until(
        &mut paired,
        |m| matches!(m, Message::RpcResponse { request_id, .. } if *request_id == send_id),
    )
    .await;
    stop(running, paired).await;
}

/// The user cancels from the requests page: the dApp is told with `rpc.cancel`, then 4102.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn the_user_cancelling_tells_the_dapp() {
    let mut paired = pair_over_relay().await;
    let running = start_loop_with(
        &paired.wallet_session,
        &paired.domain,
        Some("k1"),
        false,
        bridge_answerer(),
    );
    let request_id = post_send(&mut paired).await;

    read_dapp_until(&mut paired, is_shown).await;
    running.approvals.cancel("a1").unwrap();
    let seen = read_dapp_until(&mut paired, is_response).await;
    let n = seen.len();
    assert!(n >= 2);
    assert!(matches!(seen[n - 2], Message::RpcCancel { request_id: id } if id == request_id));
    assert_eq!(
        error_code(&seen[n - 1]),
        Some(rpc::codes::REQUEST_CANCELLED)
    );
    stop(running, paired).await;
}

/// A second request that arrives while the first waits is held, and a cancel for it is
/// answered 4102 at once, without it ever reaching the user. The first is still answered.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn a_queued_request_can_be_cancelled_before_it_is_shown() {
    let mut paired = pair_over_relay().await;
    let running = start_loop_with(
        &paired.wallet_session,
        &paired.domain,
        Some("k1"),
        false,
        bridge_answerer(),
    );
    let first = post_send(&mut paired).await;
    read_dapp_until(&mut paired, is_shown).await;

    let second = post_send(&mut paired).await;
    let cancel = paired
        .dapp_session
        .seal(
            &mut OsEntropy,
            now(),
            Message::RpcCancel { request_id: second },
            600,
        )
        .unwrap();
    paired.relay.post(&cancel).await.unwrap();
    let seen = read_dapp_until(&mut paired, is_response).await;
    assert!(
        matches!(seen.last().unwrap(), Message::RpcResponse { request_id, .. } if *request_id == second)
    );
    assert_eq!(
        error_code(seen.last().unwrap()),
        Some(rpc::codes::REQUEST_CANCELLED)
    );
    assert_eq!(
        running.approvals.outstanding().len(),
        1,
        "the first still waits"
    );

    running.approvals.resolve("a1", false).unwrap();
    let seen = read_dapp_until(&mut paired, is_response).await;
    assert!(
        matches!(seen.last().unwrap(), Message::RpcResponse { request_id, .. } if *request_id == first)
    );
    assert_eq!(
        error_code(seen.last().unwrap()),
        Some(rpc::codes::USER_REJECTED)
    );
    stop(running, paired).await;
}

/// The dApp ends the session while the user is looking at its request: the prompt goes
/// away and the loop ends, without trying to answer a session that no longer exists.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn a_session_ended_mid_prompt_closes_the_prompt_and_the_loop() {
    let mut paired = pair_over_relay().await;
    let running = start_loop_with(
        &paired.wallet_session,
        &paired.domain,
        Some("k1"),
        false,
        bridge_answerer(),
    );
    post_send(&mut paired).await;
    read_dapp_until(&mut paired, is_shown).await;

    let end = paired
        .dapp_session
        .end(&mut OsEntropy, now(), Some("closed the tab".to_owned()))
        .unwrap();
    paired.relay.post(&end).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), running.task)
        .await
        .expect("the loop ends")
        .unwrap();
    assert!(
        running.approvals.outstanding().is_empty(),
        "the prompt is gone"
    );
    assert!(running.events.lock().unwrap().contains(&"ended".to_owned()));
    assert!(running.store.list(1).unwrap().is_empty());
    paired
        .relay
        .delete(paired.pairing_mailbox, &paired.pairing_read)
        .await;
}

/// A cancel that comes after the user approved changes nothing: the transaction was
/// already sent, and the dApp gets its result.
#[tokio::test]
#[ignore = "needs a reference relay on XCHONNECT_TEST_RELAY"]
async fn a_cancel_after_approval_does_not_undo_the_broadcast() {
    let mut paired = pair_over_relay().await;
    let running = start_loop_with(
        &paired.wallet_session,
        &paired.domain,
        Some("k1"),
        false,
        bridge_answerer(),
    );
    let request_id = post_send(&mut paired).await;
    read_dapp_until(&mut paired, is_shown).await;

    running.approvals.resolve("a1", true).unwrap();
    let cancel = paired
        .dapp_session
        .seal(
            &mut OsEntropy,
            now(),
            Message::RpcCancel { request_id },
            600,
        )
        .unwrap();
    paired.relay.post(&cancel).await.unwrap();

    let seen = read_dapp_until(&mut paired, is_response).await;
    assert!(
        seen.iter()
            .any(|m| matches!(m, Message::RpcStatus { state, .. } if state == "broadcast"))
    );
    assert!(matches!(
        seen.last().unwrap(),
        Message::RpcResponse {
            outcome: RpcOutcome::Result(_),
            ..
        }
    ));
    stop(running, paired).await;
}

/// One counter of the gateway's `/metrics` (`xchonnect_gateway_<name>_total`).
async fn gateway_counter(gateway: &str, name: &str) -> u64 {
    let text = reqwest::get(format!("{gateway}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let prefix = format!("xchonnect_gateway_{name}_total ");
    text.lines()
        .find_map(|line| line.strip_prefix(&prefix)?.trim().parse().ok())
        .unwrap_or_else(|| panic!("no {name} counter in the gateway's metrics"))
}

/// Push without Apple, every hop but the last: the session loop seals the phone's device
/// token to the gateway's key and registers it with the relay; a request from the dApp
/// makes the relay call the gateway; the gateway opens the token with its secret key. A
/// gateway without an Apple key has nobody to hand it to, so the wake-up is counted as
/// `failed`, not `invalid`: that difference is the proof that what Klimper sealed is what
/// the gateway can open. The last hop, Apple to the phone, needs the key and a device.
#[tokio::test]
#[ignore = "needs a relay on XCHONNECT_TEST_RELAY and a gateway on XCHONNECT_TEST_GATEWAY"]
async fn a_request_wakes_the_phone_through_the_gateway() {
    let Ok(gateway) = std::env::var("XCHONNECT_TEST_GATEWAY") else {
        eprintln!("skipped: XCHONNECT_TEST_GATEWAY is not set");
        return;
    };
    let keys = reqwest::get(format!("{gateway}/v1/keys"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let keys: serde_json::Value = serde_json::from_str(&keys).unwrap();
    let public_key: [u8; 32] = b64::decode(keys["keys"][0].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let push = Arc::new(PushHandle::new(Some(GatewayConfig {
        url: format!("{gateway}/v1/wake"),
        public_key,
    })));
    // A fresh token each run: the gateway allows one wake-up per device in ten seconds.
    let device_token = hex::encode(xchonnect_core::crypto::random_array::<32>(&mut OsEntropy));
    assert!(push.set_device_token(Kind::Apns, &device_token));

    let mut paired = pair_over_relay().await;
    let running = start_loop_push(
        &paired.wallet_session,
        &paired.domain,
        None,
        false,
        default_answerer(),
        push,
    );

    // The loop registers on its first pass, and stores what it registered: the expiry and
    // a tag of the token, never the token.
    let deadline = Instant::now() + Duration::from_secs(10);
    let registration = loop {
        if let Some(registration) = running.store.load(1, SESSION_ID).unwrap().record.push {
            break registration;
        }
        assert!(
            Instant::now() < deadline,
            "the loop never registered for push"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert!(registration.expires_at > now());
    assert!(!registration.token_tag.is_empty());
    assert_ne!(registration.token_tag, device_token);

    let before = (
        gateway_counter(&gateway, "requests").await,
        gateway_counter(&gateway, "invalid").await,
        gateway_counter(&gateway, "failed").await,
        gateway_counter(&gateway, "delivered").await,
    );
    post_send(&mut paired).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while gateway_counter(&gateway, "requests").await == before.0 {
        assert!(
            Instant::now() < deadline,
            "the relay never called the gateway"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Delivery runs after the gateway has answered the relay: give it a moment.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let handled = gateway_counter(&gateway, "failed").await
            + gateway_counter(&gateway, "delivered").await;
        if handled > before.2 + before.3 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the gateway took the wake-up but could not open the sealed token"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(gateway_counter(&gateway, "invalid").await, before.1);

    stop(running, paired).await;
}
