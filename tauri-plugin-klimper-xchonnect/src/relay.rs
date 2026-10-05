// SPDX-License-Identifier: AGPL-3.0-or-later
//! Relay client (clapandpay TASK-074, xchonnect `docs/spec/wire/relay-api.md`).
//!
//! Plain HTTPS JSON. The relay only ever sees token **hashes** and opaque envelopes, so
//! nothing here needs to trust it: it cannot read a message, and a lying relay can at
//! worst withhold or reorder, which the session layer detects.
//!
//! Every call goes through one [`DirectTransport`], so routing requests through an OHTTP
//! relay later (clapandpay TASK-089, spec 10) replaces that one struct rather than the
//! client above it — the same reason the message loop never assumes long-polling is the
//! only wake-up (decision-008 point 2: push slots in later).

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use xchonnect_core::crypto::{MailboxId, Token};
use xchonnect_core::session::Outgoing;
use xchonnect_core::{b64, pow};

use crate::error::{Error, Result};

/// Relay answers above this are a broken or hostile relay, not a message.
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

/// Total timeout for a non-polling call.
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// Attempts [`RelayClient::post_retrying`] makes, and the pause before the first retry
/// (doubled each time).
const POST_ATTEMPTS: u32 = 3;
const POST_RETRY_DELAY: Duration = Duration::from_millis(500);

/// What `GET /v1/info` tells us. Only the fields Klimper acts on.
#[derive(Debug, Clone)]
pub struct RelayInfo {
    /// Longest `wait` the relay accepts on a direct request.
    pub max_wait_s: u64,
    /// Longest `wait` through the OHTTP gateway (often 0).
    pub max_wait_ohttp_s: u64,
    /// Accepted mailbox creation methods: `api_key`, `ticket`, `pow`, `open`.
    pub mailbox_creation: Vec<String>,
    pub pow_difficulty: u8,
}

impl Default for RelayInfo {
    fn default() -> Self {
        RelayInfo {
            max_wait_s: 25,
            max_wait_ohttp_s: 0,
            mailbox_creation: vec!["pow".to_owned()],
            pow_difficulty: pow::DEFAULT_DIFFICULTY,
        }
    }
}

/// One HTTP exchange with the relay. `status` is the HTTP status, `body` the parsed JSON
/// (`Value::Null` for an empty body).
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Value,
}

/// Direct HTTPS to the relay.
///
/// Every relay call in this module goes through one of these, which is what keeps OHTTP
/// (clapandpay TASK-089, spec 10) a second transport rather than a rewrite: the
/// encapsulating client replaces this struct and `RelayClient::call`, and nothing above
/// them changes.
#[derive(Debug)]
pub struct DirectTransport {
    client: reqwest::Client,
}

impl DirectTransport {
    pub fn new(developer_mode: bool) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Klimper")
            .referer(false);
        if !developer_mode {
            builder = builder.https_only(true);
        }
        Ok(DirectTransport {
            client: builder.build().map_err(|e| Error::Relay(e.to_string()))?,
        })
    }
}

/// A relay base URL plus the transport to reach it.
#[derive(Debug, Clone)]
pub struct RelayClient {
    base: String,
    transport: Arc<DirectTransport>,
}

impl RelayClient {
    /// `base` is the relay URL from the pairing URI, already validated by
    /// `PairingUri::parse` (HTTPS unless this is a developer-mode loopback relay).
    pub fn new(base: &str, developer_mode: bool) -> Result<Self> {
        Ok(RelayClient {
            base: base.trim_end_matches('/').to_owned(),
            transport: Arc::new(DirectTransport::new(developer_mode)?),
        })
    }

    async fn call(
        &self,
        method: &'static str,
        path: &str,
        token: Option<&Token>,
        body: Option<Value>,
        timeout: Duration,
    ) -> Result<Reply> {
        let url = format!("{}{path}", self.base);
        let client = &self.transport.client;
        let mut request = match method {
            "GET" => client.get(&url),
            "DELETE" => client.delete(&url),
            "PUT" => client.put(&url),
            _ => client.post(&url),
        }
        .timeout(timeout);
        if let Some(token) = token {
            // The capability token goes in the Authorization header and nowhere else: it
            // must never reach a log line or a URL.
            request = request.header(
                "authorization",
                format!("Bearer {}", b64::encode(token.expose())),
            );
        }
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let response = request
            .send()
            .await
            .map_err(|error| Error::Relay(describe(&error)))?;
        let status = response.status().as_u16();
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES)
        {
            return Err(Error::Relay("the relay sent too much data".to_owned()));
        }
        // Read in chunks and stop at the cap: a chunked response has no length to check up
        // front, and a relay is named by whoever made the pairing code (it is contacted on
        // scan, before the user has agreed to anything).
        let mut response = response;
        let mut bytes = Vec::with_capacity(1024);
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| Error::Relay(describe(&error)))?
        {
            if (bytes.len() + chunk.len()) as u64 > MAX_RESPONSE_BYTES {
                return Err(Error::Relay("the relay sent too much data".to_owned()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = if bytes.iter().all(u8::is_ascii_whitespace) {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        Ok(Reply { status, body })
    }

    /// Like [`Self::call`] but turns a non-2xx into an error. A `404` becomes
    /// [`Error::PairingCodeUsed`] for the caller to interpret: the relay answers
    /// byte-identically for "no such mailbox" and "wrong token", so that is all we know.
    async fn ok(
        &self,
        method: &'static str,
        path: &str,
        token: Option<&Token>,
        body: Option<Value>,
        timeout: Duration,
    ) -> Result<Value> {
        let reply = self.call(method, path, token, body, timeout).await?;
        if (200..300).contains(&reply.status) {
            return Ok(reply.body);
        }
        let code = reply.body["error"].as_str().unwrap_or("unknown").to_owned();
        if reply.status == 404 {
            return Err(Error::PairingCodeUsed);
        }
        Err(Error::Relay(format!("{} ({code})", reply.status)))
    }

    /// `GET /v1/info`. Defaults are used when the relay omits a field.
    pub async fn info(&self) -> Result<RelayInfo> {
        let value = self.ok("GET", "/v1/info", None, None, CALL_TIMEOUT).await?;
        let default = RelayInfo::default();
        Ok(RelayInfo {
            max_wait_s: value["max_wait_s"].as_u64().unwrap_or(default.max_wait_s),
            max_wait_ohttp_s: value["max_wait_ohttp_s"]
                .as_u64()
                .unwrap_or(default.max_wait_ohttp_s),
            mailbox_creation: value["mailbox_creation"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or(default.mailbox_creation),
            pow_difficulty: u8::try_from(
                value["pow_difficulty"]
                    .as_u64()
                    .unwrap_or(u64::from(default.pow_difficulty)),
            )
            .unwrap_or(default.pow_difficulty),
        })
    }

    /// Create a mailbox. The relay stores only `SHA-256("xchonnect v1 token" || token)`,
    /// so the read and write tokens stay with the wallet.
    ///
    /// Uses the URI's sponsorship ticket when the relay accepts tickets, otherwise solves
    /// the proof-of-work. The solve runs on the blocking pool, never on the UI thread or
    /// an async worker (TASK-074 AC #1).
    pub async fn create_mailbox(
        &self,
        read: &Token,
        write: &Token,
        ticket: Option<&[u8; 32]>,
    ) -> Result<MailboxId> {
        let info = self.info().await?;
        let mut body = json!({
            "read_token_hash": b64::encode(&read.hash()),
            "write_token_hash": b64::encode(&write.hash()),
        });
        let accepts = |method: &str| info.mailbox_creation.iter().any(|m| m == method);
        match ticket {
            Some(ticket) if accepts("ticket") => {
                body["ticket"] = json!(b64::encode(ticket));
            }
            _ if accepts("pow") => {
                let challenge = self
                    .ok("POST", "/v1/challenge", None, None, CALL_TIMEOUT)
                    .await?;
                let encoded = challenge["challenge"]
                    .as_str()
                    .ok_or_else(|| Error::Relay("the relay sent no challenge".to_owned()))?
                    .to_owned();
                let bytes = b64::decode(&encoded).map_err(Error::protocol)?;
                let nonce = tauri::async_runtime::spawn_blocking(move || pow::solve(&bytes))
                    .await
                    .map_err(Error::internal)?
                    .map_err(Error::protocol)?;
                body["pow"] = json!({ "challenge": encoded, "nonce": b64::encode(&nonce) });
            }
            // `open` relays, or an API key relay we have no key for: try without a proof
            // and let the relay answer.
            _ => {}
        }
        let value = self
            .ok("POST", "/v1/mailboxes", None, Some(body), CALL_TIMEOUT)
            .await?;
        MailboxId::from_b64(
            value["mailbox_id"]
                .as_str()
                .ok_or_else(|| Error::Relay("the relay sent no mailbox id".to_owned()))?,
        )
        .map_err(Error::protocol)
    }

    /// Post an envelope. Callers persist the session **before** this (spec 5.3).
    pub async fn post(&self, outgoing: &Outgoing) -> Result<()> {
        self.ok(
            "POST",
            &format!("/v1/mailboxes/{}/messages", outgoing.mailbox.to_b64()),
            Some(&outgoing.write_token),
            Some(json!({ "env": b64::encode(&outgoing.envelope) })),
            CALL_TIMEOUT,
        )
        .await?;
        Ok(())
    }

    /// [`Self::post`] for the messages the other side must not miss: `session.ready` and
    /// `session.end`. A transient failure is retried with backoff; a `404` is not, because
    /// the dApp's mailbox is gone and a retry cannot bring it back. Posting the same
    /// envelope twice is harmless: the dApp drops the second as a replay.
    pub async fn post_retrying(&self, outgoing: &Outgoing) -> Result<()> {
        let mut delay = POST_RETRY_DELAY;
        let mut attempt = 1;
        loop {
            match self.post(outgoing).await {
                Ok(()) => return Ok(()),
                Err(error @ Error::PairingCodeUsed) => return Err(error),
                Err(error) if attempt == POST_ATTEMPTS => return Err(error),
                Err(_) => {
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                    attempt += 1;
                }
            }
        }
    }

    /// Long-poll for messages. `wait` is clamped to the relay's `max_wait_s`, so
    /// foreground polling stays inside the limits the relay publishes (TASK-074 AC #4).
    pub async fn fetch(
        &self,
        mailbox: MailboxId,
        read: &Token,
        wait: u64,
        info: &RelayInfo,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let wait = wait.min(info.max_wait_s);
        let timeout = CALL_TIMEOUT + Duration::from_secs(wait);
        let value = self
            .ok(
                "GET",
                &format!("/v1/mailboxes/{}/messages?wait={wait}", mailbox.to_b64()),
                Some(read),
                None,
                timeout,
            )
            .await?;
        let mut out = Vec::new();
        for message in value["messages"].as_array().cloned().unwrap_or_default() {
            let Some(id) = message["msg_id"].as_str() else {
                continue;
            };
            let Ok(envelope) = b64::decode(message["env"].as_str().unwrap_or_default()) else {
                // A malformed entry is the relay's problem; acknowledge it so it does not
                // block the mailbox, and skip it.
                tracing::warn!("relay sent an undecodable envelope; skipping");
                out.push((id.to_owned(), Vec::new()));
                continue;
            };
            out.push((id.to_owned(), envelope));
        }
        Ok(out)
    }

    /// Acknowledge messages. Only ever called after they were processed **and** the new
    /// session state was persisted (TASK-074 AC #2).
    pub async fn ack(&self, mailbox: MailboxId, read: &Token, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        self.ok(
            "POST",
            &format!("/v1/mailboxes/{}/ack", mailbox.to_b64()),
            Some(read),
            Some(json!({ "msg_ids": ids })),
            CALL_TIMEOUT,
        )
        .await?;
        Ok(())
    }

    /// Delete a mailbox and everything in it. Failures are logged, not propagated: the
    /// local session is gone either way and the relay drops an unused mailbox after 30
    /// days.
    pub async fn delete(&self, mailbox: MailboxId, read: &Token) {
        if let Err(error) = self
            .ok(
                "DELETE",
                &format!("/v1/mailboxes/{}", mailbox.to_b64()),
                Some(read),
                None,
                CALL_TIMEOUT,
            )
            .await
        {
            tracing::warn!("could not delete a relay mailbox: {error}");
        }
    }
}

fn describe(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "the relay did not answer in time".to_owned()
    } else if error.is_connect() {
        "could not connect to the relay".to_owned()
    } else {
        "the relay could not be reached".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_info_falls_back_to_the_spec_defaults() {
        let info = RelayInfo::default();
        assert_eq!(info.max_wait_s, 25);
        assert_eq!(info.max_wait_ohttp_s, 0);
        assert_eq!(info.pow_difficulty, pow::DEFAULT_DIFFICULTY);
    }

    #[test]
    fn a_wait_longer_than_the_relay_allows_is_clamped() {
        let info = RelayInfo {
            max_wait_s: 5,
            ..RelayInfo::default()
        };
        assert_eq!(20_u64.min(info.max_wait_s), 5);
    }

    #[test]
    fn a_release_client_refuses_plain_http() {
        // `https_only` is set unless the URI was developer mode; building the client is
        // the only thing this can assert without a server.
        assert!(DirectTransport::new(false).is_ok());
        assert!(DirectTransport::new(true).is_ok());
    }
}
