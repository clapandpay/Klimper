// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fetching and verifying the dApp's origin document (clapandpay TASK-075, xchonnect
//! spec 6.1 and 6.3, integration guide §1 steps 3 to 5).
//!
//! Pairing is where phishing happens, so every rule here is a hard failure with no
//! continue-anyway path:
//!
//! * `https://<domain>/.well-known/xchonnect.json`, and plain HTTP only for a
//!   developer-mode `localhost` URI in a debug build;
//! * **no redirects** — a 3xx is a failure, not something to follow, because the
//!   signature is bound to the domain in the URI and not to wherever it points;
//! * **no cookies** — the `reqwest` cookie store is not compiled in, so the request
//!   carries no ambient authority and the document cannot be personalised;
//! * at most 16 KiB (`xchonnect_core::origin::MAX_DOCUMENT_BYTES`), enforced while the
//!   body streams in so an endless response cannot exhaust memory;
//! * a 10 s total timeout;
//! * re-fetched on **every** pairing: nothing here is cached.
//!
//! Only then is the URI verified against the document, which checks the expiry and the
//! Ed25519 origin signature over the URI's signature input.

use std::time::Duration;

use xchonnect_core::origin::{MAX_DOCUMENT_BYTES, OriginDocument};
use xchonnect_core::pairing::VerifiedUri;
use xchonnect_core::uri::{PairingUri, ParseOptions, origin_document_url};

use crate::error::{Error, Result};

/// Total time allowed for connect plus transfer (spec 6.1).
const TIMEOUT: Duration = Duration::from_secs(10);

/// Fetch the origin document for `domain` under the rules above.
pub async fn fetch(domain: &str, options: ParseOptions) -> Result<OriginDocument> {
    let url = origin_document_url(domain, options);
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .connect_timeout(TIMEOUT)
        .user_agent("Klimper")
        .referer(false);
    // A developer-mode URI may only ever reach a loopback `localhost` document; every
    // other pairing is HTTPS-only, enforced by the client and not just by the URL we
    // built.
    if !options.developer_mode {
        builder = builder.https_only(true);
    }
    let client = builder.build().map_err(|e| Error::Origin(e.to_string()))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| Error::Origin(describe(&error)))?;

    let status = response.status();
    if status.is_redirection() {
        // Spec 6.1: the document must be served by the domain itself.
        return Err(Error::Origin(format!(
            "{domain} redirected the request ({status}); the document must be served by the domain itself"
        )));
    }
    if !status.is_success() {
        return Err(Error::Origin(format!("{domain} answered {status}")));
    }
    // A declared length over the cap is refused before a single byte is read.
    if response
        .content_length()
        .is_some_and(|n| n > MAX_DOCUMENT_BYTES as u64)
    {
        return Err(Error::Origin(format!(
            "{domain} sent more than {MAX_DOCUMENT_BYTES} bytes"
        )));
    }

    let body = read_capped(response, domain).await?;
    OriginDocument::parse(&body).map_err(|error| Error::Origin(error.to_string()))
}

/// Read at most [`MAX_DOCUMENT_BYTES`]; a longer body fails rather than being truncated,
/// because a truncated document would simply fail to parse and hide the real problem.
async fn read_capped(mut response: reqwest::Response, domain: &str) -> Result<Vec<u8>> {
    let mut body = Vec::with_capacity(1024);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| Error::Origin(describe(&error)))?
    {
        if body.len() + chunk.len() > MAX_DOCUMENT_BYTES {
            return Err(Error::Origin(format!(
                "{domain} sent more than {MAX_DOCUMENT_BYTES} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Network errors without the URL or any header, which can carry a token in other
/// contexts; the domain is added by the caller's message.
fn describe(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "the website did not answer within 10 seconds".to_owned()
    } else if error.is_connect() {
        "could not connect to the website".to_owned()
    } else if error.is_decode() {
        "the website's answer could not be read".to_owned()
    } else {
        "the website could not be reached".to_owned()
    }
}

/// Parse, fetch and verify in one step. Returns the verified URI; any failure aborts
/// pairing (spec 6.3 step 2, integration guide MUST checklist item 1).
///
/// `allow_developer_mode` is false in release builds, so a developer-mode URI — which
/// would accept a plain-HTTP loopback relay and an unencrypted origin document — can
/// never be paired with a shipped app (clapandpay TASK-074 AC #5).
pub async fn verify_pairing_uri(
    uri_text: &str,
    now: u64,
    allow_developer_mode: bool,
) -> Result<(VerifiedUri, bool)> {
    // Parse strictly first. Trying developer mode only after a strict parse failed keeps
    // a release build from ever accepting one.
    let strict = ParseOptions {
        developer_mode: false,
    };
    let (uri, developer_mode) = match PairingUri::parse(uri_text, strict) {
        Ok(uri) => (uri, false),
        Err(strict_error) => {
            if !allow_developer_mode {
                // Say plainly when the only thing wrong was the developer-mode scheme,
                // so a developer does not chase a parse bug in a release build.
                let dev = ParseOptions {
                    developer_mode: true,
                };
                if PairingUri::parse(uri_text, dev).is_ok() {
                    return Err(Error::DeveloperModeRefused);
                }
                return Err(Error::InvalidUri(strict_error.to_string()));
            }
            let dev = ParseOptions {
                developer_mode: true,
            };
            let uri = PairingUri::parse(uri_text, dev)
                .map_err(|error| Error::InvalidUri(error.to_string()))?;
            (uri, true)
        }
    };

    let options = ParseOptions { developer_mode };
    // Expiry before the network call: an expired code is not worth a request.
    uri.check_time(now)
        .map_err(|error| Error::InvalidUri(error.to_string()))?;
    let document = fetch(&uri.domain, options).await?;
    let verified = VerifiedUri::new(uri, &document, now)
        .map_err(|error| Error::OriginVerification(error.to_string()))?;
    Ok((verified, developer_mode))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::testing;

    /// A release build refuses a developer-mode URI outright, and says so.
    #[tokio::test]
    async fn release_builds_refuse_developer_mode_uris() {
        let mut f = testing::fixture();
        let dapp = xchonnect_core::pairing::DappPairing::new(
            &mut f.rng,
            testing::NOW,
            &f.signer,
            xchonnect_core::pairing::DappPairingParams {
                relay: "http://127.0.0.1:8787",
                domain: "localhost:5173",
                pairing_mailbox: xchonnect_core::crypto::MailboxId([1; 16]),
                pairing_write: xchonnect_core::crypto::Token::from_bytes([2; 32]),
                lifetime_s: 300,
                ticket: None,
                options: ParseOptions {
                    developer_mode: true,
                },
            },
        )
        .unwrap();
        let uri = dapp.uri().to_uri();
        let error = verify_pairing_uri(&uri, testing::NOW + 1, false)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::DeveloperModeRefused), "{error}");
    }

    /// An expired code never reaches the network.
    #[tokio::test]
    async fn an_expired_uri_aborts_before_any_request() {
        let mut f = testing::fixture();
        let (_, uri) = testing::pairing_uri(&mut f);
        let error = verify_pairing_uri(&uri, testing::NOW + 100_000, true)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::InvalidUri(_)), "{error}");
    }

    /// A document that does not carry the key the URI names, or whose signature does not
    /// verify, is rejected by `VerifiedUri` — the step after this module's fetch.
    #[test]
    fn a_wrong_origin_key_fails_verification() {
        let mut f = testing::fixture();
        let (_, uri) = testing::pairing_uri(&mut f);
        let parsed = PairingUri::parse(&uri, ParseOptions::default()).unwrap();
        let other = xchonnect_core::uri::LocalSigner::new(
            xchonnect_core::crypto::Ed25519Seed::from_bytes([9; 32]),
            "k1",
        )
        .unwrap();
        let json = format!(
            r#"{{"v":1,"name":"Not Pengui","origin_keys":[{{"kid":"k1","pk":"{}","not_after":"2030-01-01"}}]}}"#,
            xchonnect_core::b64::encode(&other.public_key())
        );
        let doc = OriginDocument::parse(json.as_bytes()).unwrap();
        assert!(VerifiedUri::new(parsed, &doc, testing::NOW + 1).is_err());
    }

    #[test]
    fn an_oversize_document_is_refused_by_the_parser_cap() {
        let big = vec![b'x'; MAX_DOCUMENT_BYTES + 1];
        assert!(OriginDocument::parse(&big).is_err());
    }
}
