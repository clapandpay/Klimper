// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bridging wallet-kit's synchronous approval and signing hooks to the asynchronous
//! approval UI (clapandpay TASK-079, TASK-078).
//!
//! `xchonnect_wallet_kit::handle` calls [`xchonnect_wallet_kit::Approver::approve`] and
//! [`xchonnect_wallet_kit::Signer::sign`] inline, so the whole call runs on a blocking
//! thread and both hooks park on a channel until the webview answers. Two consequences
//! that matter:
//!
//! * **Nothing is signed without an answer.** No answer within [`DECISION_TIMEOUT`] is a
//!   rejection, never an approval; a dropped channel (the webview closed) is a rejection
//!   too.
//! * **The prompt the user sees is the prompt that was approved.** The JSON handed to the
//!   webview is `serde_json::to_value(prompt)` of the exact `Prompt` wallet-kit built from
//!   the local simulation, so a dApp's own labels and amounts never reach the screen
//!   (spec 11.1 item 2, integration guide §5).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};

/// How long a prompt waits for the user. A signing request's own `exp` is usually
/// shorter; whichever comes first wins, and both mean "rejected".
pub const DECISION_TIMEOUT: Duration = Duration::from_mins(5);

/// Event emitted when an approval starts waiting.
pub const EVENT_APPROVAL: &str = "klimper-xchonnect://approval";
/// Event emitted when an approval is answered, times out or is withdrawn.
pub const EVENT_APPROVAL_DONE: &str = "klimper-xchonnect://approval-resolved";

/// What the user is being asked.
///
/// `Deserialize` exists so the integration tests can read back exactly what the webview
/// would receive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ApprovalKind {
    /// The whole request: wallet-kit's `Prompt`, serialised exactly as it was built.
    Request {
        /// `Prompt` as JSON. Render `summary` and `plan`; never anything the dApp sent.
        prompt: Value,
    },
}

/// An approval the webview has to answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    /// Id to pass back to `resolve_approval`.
    pub id: String,
    pub session_id: String,
    /// Verified dApp domain.
    pub dapp: String,
    /// CHIP-0002 method.
    pub method: String,
    /// Unix seconds when this approval stops waiting.
    pub expires_at: u64,
    #[serde(flatten)]
    pub kind: ApprovalKind,
}

struct Waiting {
    approval: Approval,
    answer: mpsc::Sender<bool>,
}

/// Emits a Tauri event. Injected so the bridge is testable without a webview.
pub type Emitter = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// The registry of outstanding approvals.
pub struct ApprovalBridge {
    registry: Mutex<Registry>,
    emit: Emitter,
}

#[derive(Default)]
struct Registry {
    waiting: BTreeMap<String, Waiting>,
    /// Sessions that were withdrawn. Ids are random and never reused, so an entry only
    /// ever stops a request that was already on its way when the session ended.
    closed: BTreeSet<String>,
}

impl std::fmt::Debug for ApprovalBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovalBridge").finish_non_exhaustive()
    }
}

impl ApprovalBridge {
    pub fn new(emit: Emitter) -> Self {
        ApprovalBridge {
            registry: Mutex::new(Registry::default()),
            emit,
        }
    }

    /// Ask the user and block until the answer arrives. Called from a blocking thread
    /// inside `handle()`; `false` means "do not sign".
    pub fn ask(&self, approval: Approval) -> bool {
        let id = approval.id.clone();
        let (tx, rx) = mpsc::channel();
        let payload = serde_json::to_value(&approval).unwrap_or(Value::Null);
        {
            let Ok(mut registry) = self.registry.lock() else {
                tracing::error!("approval registry poisoned; refusing the request");
                return false;
            };
            // The session ended while this request was being simulated: nothing to ask.
            if registry.closed.contains(&approval.session_id) {
                return false;
            }
            registry.waiting.insert(
                id.clone(),
                Waiting {
                    approval,
                    answer: tx,
                },
            );
        }
        (self.emit)(EVENT_APPROVAL, payload);

        // A timeout and a closed channel are both rejections: the only path to `true` is
        // the user pressing approve.
        let decision = matches!(rx.recv_timeout(DECISION_TIMEOUT), Ok(true));
        if let Ok(mut registry) = self.registry.lock() {
            registry.waiting.remove(&id);
        }
        (self.emit)(
            EVENT_APPROVAL_DONE,
            serde_json::json!({ "id": id, "approved": decision }),
        );
        decision
    }

    /// The webview answered.
    pub fn resolve(&self, id: &str, approved: bool) -> Result<()> {
        let sender = {
            let registry = self.registry.lock().map_err(Error::internal)?;
            registry.waiting.get(id).map(|w| w.answer.clone())
        };
        let sender = sender.ok_or(Error::UnknownApproval)?;
        // The waiting thread removes its own entry; a send failure means it already gave
        // up, which is itself a rejection.
        sender.send(approved).map_err(|_| Error::UnknownApproval)
    }

    /// Everything still waiting, so a reloaded webview can rebuild its screen.
    pub fn outstanding(&self) -> Vec<Approval> {
        self.registry
            .lock()
            .map(|r| r.waiting.values().map(|x| x.approval.clone()).collect())
            .unwrap_or_default()
    }

    /// The session ended: reject everything it is waiting on, and anything it would still
    /// ask, so a request that was mid-simulation when the user disconnected cannot open a
    /// prompt for a connection that no longer exists.
    pub fn withdraw_session(&self, session_id: &str) {
        let senders: Vec<mpsc::Sender<bool>> = self
            .registry
            .lock()
            .map(|mut r| {
                r.closed.insert(session_id.to_owned());
                r.waiting
                    .values()
                    .filter(|x| x.approval.session_id == session_id)
                    .map(|x| x.answer.clone())
                    .collect()
            })
            .unwrap_or_default();
        for sender in senders {
            let _ = sender.send(false);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn bridge() -> (Arc<ApprovalBridge>, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let emit: Emitter = Arc::new(move |event, _payload| {
            sink.lock().unwrap().push(event.to_owned());
        });
        (Arc::new(ApprovalBridge::new(emit)), seen)
    }

    fn approval(id: &str) -> Approval {
        Approval {
            id: id.to_owned(),
            session_id: "s1".to_owned(),
            dapp: "pengui.xyz".to_owned(),
            method: "signCoinSpends".to_owned(),
            expires_at: 0,
            kind: ApprovalKind::Request {
                prompt: Value::Null,
            },
        }
    }

    #[test]
    fn an_answer_reaches_the_waiting_thread() {
        let (bridge, seen) = bridge();
        let worker = {
            let bridge = bridge.clone();
            std::thread::spawn(move || bridge.ask(approval("a")))
        };
        // Wait for the prompt to be registered, then answer it.
        for _ in 0..200 {
            if bridge.resolve("a", true).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(worker.join().unwrap(), "approved");
        let events = seen.lock().unwrap().clone();
        assert_eq!(events, vec![EVENT_APPROVAL, EVENT_APPROVAL_DONE]);
    }

    #[test]
    fn a_rejection_and_a_withdrawn_session_both_mean_do_not_sign() {
        let (bridge, _) = bridge();
        let worker = {
            let bridge = bridge.clone();
            std::thread::spawn(move || bridge.ask(approval("b")))
        };
        for _ in 0..200 {
            if bridge.resolve("b", false).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!worker.join().unwrap());

        let worker = {
            let bridge = bridge.clone();
            std::thread::spawn(move || bridge.ask(approval("c")))
        };
        for _ in 0..200 {
            if !bridge.outstanding().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        bridge.withdraw_session("s1");
        assert!(!worker.join().unwrap());
    }

    #[test]
    fn a_withdrawn_session_cannot_ask_again() {
        let (bridge, seen) = bridge();
        bridge.withdraw_session("s1");
        // Refused at once rather than after DECISION_TIMEOUT, and the webview never sees it.
        assert!(!bridge.ask(approval("d")));
        assert!(bridge.outstanding().is_empty());
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn an_unknown_approval_cannot_be_answered() {
        let (bridge, _) = bridge();
        assert!(matches!(
            bridge.resolve("nope", true),
            Err(Error::UnknownApproval)
        ));
    }
}
