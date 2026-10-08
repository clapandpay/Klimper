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
//! * **A withdrawn request is never signed.** `rpc.cancel` from the dApp, or the user
//!   cancelling it from the requests page, withdraws it (xchonnect spec 9.1): a prompt on
//!   screen is answered "no", and a request still being simulated never opens one. The
//!   session loop then answers 4102 instead of 4002.

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
/// Event emitted when Klimper broadcast a transaction for a website:
/// `{ dapp, method, transactionId, status }`.
pub const EVENT_SENT: &str = "klimper-xchonnect://sent";

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
    /// The Xchonnect request this approval belongs to (base64url of its 16-byte id).
    pub request_id: String,
    /// Unix seconds when this approval stops waiting.
    pub expires_at: u64,
    #[serde(flatten)]
    pub kind: ApprovalKind,
}

/// How long an approval may still wait: until its deadline, and never past
/// [`DECISION_TIMEOUT`]. `expires_at` 0 means no deadline of its own.
fn time_left(expires_at: u64) -> Duration {
    if expires_at == 0 {
        return DECISION_TIMEOUT;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Duration::from_secs(expires_at.saturating_sub(now)).min(DECISION_TIMEOUT)
}

struct Waiting {
    approval: Approval,
    answer: mpsc::Sender<bool>,
    /// The deadline on the monotonic clock the waiting thread times out by, so an answer is
    /// refused exactly when the thread stops listening.
    deadline: std::time::Instant,
}

/// Who withdrew a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Withdrawn {
    /// The dApp sent `rpc.cancel`.
    ByDapp,
    /// The user cancelled it in Klimper; the dApp has to be told with `rpc.cancel`.
    ByUser,
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
    /// Requests withdrawn before their answer was sent, by `(session, request)`. The
    /// session loop takes the entry when it answers the request, so nothing accumulates.
    withdrawn: BTreeMap<(String, String), Withdrawn>,
    /// When the user last said no, per session, within [`REJECTION_WINDOW`].
    rejections: BTreeMap<String, std::collections::VecDeque<std::time::Instant>>,
}

/// After this many rejections within [`REJECTION_WINDOW`], a session's requests are refused
/// without a prompt until the oldest of them is out of the window (TASK-119): a website that
/// keeps asking cannot wear the user down into a tap.
pub const REJECTIONS_BEFORE_PAUSE: usize = 3;
pub const REJECTION_WINDOW: Duration = Duration::from_mins(2);

impl Registry {
    /// Whether `session` is paused now, forgetting rejections out of the window.
    fn paused(&mut self, session: &str, now: std::time::Instant) -> bool {
        let Some(recent) = self.rejections.get_mut(session) else {
            return false;
        };
        while recent
            .front()
            .is_some_and(|at| now.duration_since(*at) >= REJECTION_WINDOW)
        {
            recent.pop_front();
        }
        recent.len() >= REJECTIONS_BEFORE_PAUSE
    }
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

    /// Tell the webview a transaction left the wallet, so it can show where to look it up.
    pub fn sent(&self, payload: Value) {
        (self.emit)(EVENT_SENT, payload);
    }

    /// [`ApprovalBridge::ask_with`] without a callback.
    #[cfg(test)]
    pub fn ask(&self, approval: Approval) -> bool {
        self.ask_with(approval, || {})
    }

    /// Ask the user and block until the answer arrives. Called from a blocking thread
    /// inside `handle()`; `false` means "do not sign". `on_shown` runs once the prompt is
    /// on screen (not when the request was refused before it could be shown).
    pub fn ask_with(&self, approval: Approval, on_shown: impl FnOnce()) -> bool {
        let id = approval.id.clone();
        let key = (approval.session_id.clone(), approval.request_id.clone());
        let (tx, rx) = mpsc::channel();
        let wait = time_left(approval.expires_at);
        // Already past its deadline: never shown, never signed.
        if wait.is_zero() {
            return false;
        }
        let deadline = std::time::Instant::now() + wait;
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
            // Withdrawn while it was being simulated: never shown.
            if registry.withdrawn.contains_key(&key) {
                return false;
            }
            // The user just said no to this website several times: not asked again yet.
            if registry.paused(&approval.session_id, std::time::Instant::now()) {
                return false;
            }
            registry.waiting.insert(
                id.clone(),
                Waiting {
                    approval,
                    answer: tx,
                    deadline,
                },
            );
        }
        (self.emit)(EVENT_APPROVAL, payload);
        on_shown();

        // A timeout and a closed channel are both rejections: the only path to `true` is
        // the user pressing approve, before the request's own deadline. A request the dApp
        // has already given up on (and may have sent again) is never signed late.
        let decision = matches!(
            rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())),
            Ok(true)
        );
        if let Ok(mut registry) = self.registry.lock() {
            registry.waiting.remove(&id);
            // Any prompt that was shown and not approved counts: a no, silence, the user's
            // cancel, and the website withdrawing it too, or a website could keep the user
            // prompted forever by cancelling each request itself.
            if !decision {
                registry
                    .rejections
                    .entry(key.0.clone())
                    .or_default()
                    .push_back(std::time::Instant::now());
            }
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
            registry.waiting.get(id).and_then(|w| {
                // Already withdrawn (and answered "no"): an approval now would be reported
                // as given although nothing will be signed.
                let key = (w.approval.session_id.clone(), w.approval.request_id.clone());
                // Past its deadline: the waiting thread is about to give up, and an approval
                // that slipped in now would sign a request the dApp no longer expects.
                let live = std::time::Instant::now() < w.deadline;
                (live && !registry.withdrawn.contains_key(&key)).then(|| w.answer.clone())
            })
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

    /// Withdraw one request (`rpc.cancel`, or the user cancelling it): a prompt on screen
    /// is answered "no", and a prompt not yet shown never will be. Returns whether a prompt
    /// was waiting.
    pub fn withdraw_request(&self, session_id: &str, request_id: &str, by: Withdrawn) -> bool {
        let senders: Vec<mpsc::Sender<bool>> = self
            .registry
            .lock()
            .map(|mut r| {
                r.withdrawn
                    .entry((session_id.to_owned(), request_id.to_owned()))
                    .or_insert(by);
                r.waiting
                    .values()
                    .filter(|x| {
                        x.approval.session_id == session_id && x.approval.request_id == request_id
                    })
                    .map(|x| x.answer.clone())
                    .collect()
            })
            .unwrap_or_default();
        let waiting = !senders.is_empty();
        for sender in senders {
            let _ = sender.send(false);
        }
        waiting
    }

    /// The user cancelled a waiting approval from the requests page. Unlike a rejection
    /// (4002), the dApp is told the request was withdrawn (`rpc.cancel`, 4102).
    pub fn cancel(&self, approval_id: &str) -> Result<()> {
        let (session_id, request_id) = {
            let registry = self.registry.lock().map_err(Error::internal)?;
            let waiting = registry
                .waiting
                .get(approval_id)
                .ok_or(Error::UnknownApproval)?;
            (
                waiting.approval.session_id.clone(),
                waiting.approval.request_id.clone(),
            )
        };
        self.withdraw_request(&session_id, &request_id, Withdrawn::ByUser);
        Ok(())
    }

    /// Whether, and by whom, a request was withdrawn. Forgets it: called once, when the
    /// session loop answers the request.
    pub fn take_withdrawn(&self, session_id: &str, request_id: &str) -> Option<Withdrawn> {
        self.registry.lock().ok().and_then(|mut r| {
            r.withdrawn
                .remove(&(session_id.to_owned(), request_id.to_owned()))
        })
    }

    /// Whether the user said no to this session too often lately ([`REJECTIONS_BEFORE_PAUSE`]).
    /// Checked before any work for a request, so a paused website learns nothing from it.
    pub fn paused(&self, session_id: &str) -> bool {
        self.registry.lock().map_or(true, |mut r| {
            r.paused(session_id, std::time::Instant::now())
        })
    }

    /// The user changed this website's settings: whatever it is waiting on was checked
    /// against the old ones, so it is withdrawn (the website is told, and may ask again).
    pub fn withdraw_waiting(&self, session_id: &str) {
        let senders: Vec<mpsc::Sender<bool>> = self
            .registry
            .lock()
            .map(|mut r| {
                let keys: Vec<(String, String)> = r
                    .waiting
                    .values()
                    .filter(|x| x.approval.session_id == session_id)
                    .map(|x| (x.approval.session_id.clone(), x.approval.request_id.clone()))
                    .collect();
                for key in keys {
                    r.withdrawn.entry(key).or_insert(Withdrawn::ByUser);
                }
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
            request_id: format!("r-{id}"),
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
    fn a_website_rejected_again_and_again_is_not_asked_for_a_while() {
        let (bridge, seen) = bridge();
        for round in 0..REJECTIONS_BEFORE_PAUSE {
            let worker = {
                let bridge = bridge.clone();
                std::thread::spawn(move || bridge.ask(approval(&format!("r{round}"))))
            };
            for _ in 0..200 {
                if bridge.resolve(&format!("r{round}"), false).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(!worker.join().unwrap());
        }
        let shown = seen.lock().unwrap().len();
        // The next one is refused at once and never reaches the screen.
        assert!(!bridge.ask(approval("again")));
        assert_eq!(seen.lock().unwrap().len(), shown);
    }

    #[test]
    fn a_website_cannot_dodge_the_pause_by_withdrawing_its_own_prompts() {
        let (bridge, _) = bridge();
        for round in 0..REJECTIONS_BEFORE_PAUSE {
            let id = format!("w{round}");
            let worker = {
                let bridge = bridge.clone();
                let id = id.clone();
                std::thread::spawn(move || bridge.ask(approval(&id)))
            };
            while bridge.outstanding().is_empty() {
                std::thread::sleep(Duration::from_millis(5));
            }
            bridge.withdraw_request("s1", &format!("r-{id}"), Withdrawn::ByDapp);
            assert!(!worker.join().unwrap());
        }
        assert!(bridge.paused("s1"));
    }

    #[test]
    fn changed_settings_withdraw_the_prompt_on_screen() {
        let (bridge, _) = bridge();
        let worker = {
            let bridge = bridge.clone();
            std::thread::spawn(move || bridge.ask(approval("p")))
        };
        while bridge.outstanding().is_empty() {
            std::thread::sleep(Duration::from_millis(5));
        }
        bridge.withdraw_waiting("s1");
        assert!(!worker.join().unwrap());
        assert_eq!(bridge.take_withdrawn("s1", "r-p"), Some(Withdrawn::ByUser));
    }

    #[test]
    fn a_request_past_its_deadline_is_never_signed() {
        let (bridge, _) = bridge();
        let mut late = approval("late");
        late.expires_at = 1; // long gone
        assert!(!bridge.ask(late), "refused without waiting");
        assert!(bridge.resolve("late", true).is_err());
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

    fn wait_until_waiting(bridge: &ApprovalBridge) {
        for _ in 0..200 {
            if !bridge.outstanding().is_empty() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the prompt never appeared");
    }

    #[test]
    fn a_withdrawn_request_closes_its_prompt_and_is_remembered_once() {
        let (bridge, _) = bridge();
        let shown = Arc::new(Mutex::new(false));
        let worker = {
            let (bridge, shown) = (bridge.clone(), shown.clone());
            std::thread::spawn(move || {
                bridge.ask_with(approval("e"), || *shown.lock().unwrap() = true)
            })
        };
        wait_until_waiting(&bridge);
        // Another request of the same session is not touched.
        assert!(!bridge.withdraw_request("s1", "r-other", Withdrawn::ByDapp));
        assert!(bridge.withdraw_request("s1", "r-e", Withdrawn::ByDapp));
        // Approving what was just withdrawn is refused, never reported as given.
        assert!(matches!(
            bridge.resolve("e", true),
            Err(Error::UnknownApproval)
        ));
        assert!(!worker.join().unwrap(), "withdrawn means do not sign");
        assert!(*shown.lock().unwrap());
        assert_eq!(bridge.take_withdrawn("s1", "r-e"), Some(Withdrawn::ByDapp));
        assert_eq!(
            bridge.take_withdrawn("s1", "r-e"),
            None,
            "forgotten once taken"
        );
        assert_eq!(
            bridge.take_withdrawn("s1", "r-other"),
            Some(Withdrawn::ByDapp)
        );
    }

    #[test]
    fn a_request_withdrawn_before_its_prompt_is_never_shown() {
        let (bridge, seen) = bridge();
        bridge.withdraw_request("s1", "r-f", Withdrawn::ByDapp);
        let mut shown = false;
        assert!(!bridge.ask_with(approval("f"), || shown = true));
        assert!(!shown);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn the_user_cancelling_withdraws_the_request_for_the_dapp() {
        let (bridge, _) = bridge();
        let worker = {
            let bridge = bridge.clone();
            std::thread::spawn(move || bridge.ask(approval("g")))
        };
        wait_until_waiting(&bridge);
        bridge.cancel("g").unwrap();
        assert!(!worker.join().unwrap());
        assert_eq!(bridge.take_withdrawn("s1", "r-g"), Some(Withdrawn::ByUser));
        assert!(matches!(bridge.cancel("g"), Err(Error::UnknownApproval)));
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
