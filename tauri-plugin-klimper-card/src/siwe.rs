// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sign-In with Ethereum (EIP-4361): checking the message Gnosis Pay hands over.
//!
//! Gnosis Pay builds the sign-in text on its server (`GET /auth/siwe/{address}`) and the
//! wallet signs it. Signing whatever text a server sends would make the card key a signing
//! oracle: a compromised or impersonated server could get a sign-in for another site, or a
//! message that means something else entirely. So Klimper parses the text and signs it only
//! when it is exactly a sign-in for the domain and URI compiled into this build, for the
//! card's own address, on the expected chain, issued just now, with no resources (no
//! delegated capabilities) and nothing it does not understand.

use std::fmt::Write as _;

use crate::evm::Address;

/// Accepted distance between the message's issue time and this device's clock.
pub const CLOCK_SKEW_SECS: u64 = 5 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiweMessage {
    /// RFC 3986 authority that asks for the sign-in.
    pub domain: String,
    pub address: Address,
    pub statement: Option<String>,
    pub uri: String,
    pub chain_id: u64,
    pub nonce: String,
    /// RFC 3339.
    pub issued_at: String,
    pub expiration_time: Option<String>,
    pub not_before: Option<String>,
    pub request_id: Option<String>,
}

/// What this build will sign in to.
#[derive(Debug, Clone)]
pub struct Expected<'a> {
    pub domain: &'a str,
    pub uri: &'a str,
    pub address: Address,
    pub chain_id: u64,
    /// This device's clock, seconds since the Unix epoch.
    pub now: u64,
}

#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum SiweError {
    #[error("the sign-in message is not a Sign-In with Ethereum message")]
    Malformed,
    #[error("the sign-in message is for another website")]
    WrongDomain,
    #[error("the sign-in message names another address")]
    WrongAddress,
    #[error("the sign-in message is for another address on the website")]
    WrongUri,
    #[error("the sign-in message is for another chain")]
    WrongChain,
    #[error("the sign-in message has an unknown version")]
    WrongVersion,
    #[error("the sign-in nonce is not a plain nonce")]
    BadNonce,
    #[error("the sign-in message is not from now; check the phone's clock")]
    WrongTime,
    #[error("the sign-in message asks for more than a sign-in")]
    Resources,
}

/// EIP-4361: at least 8 alphanumeric characters.
pub fn valid_nonce(nonce: &str) -> bool {
    nonce.len() >= 8 && nonce.len() <= 128 && nonce.chars().all(|c| c.is_ascii_alphanumeric())
}

const HEADER: &str = " wants you to sign in with your Ethereum account:";

impl SiweMessage {
    /// Parses the EIP-4361 text. Anything outside the grammar is refused.
    pub fn parse(text: &str) -> Result<Self, SiweError> {
        let lines: Vec<&str> = text.split('\n').collect();
        let mut at = 0;
        let mut next = || {
            let line = lines.get(at).copied();
            at += 1;
            line
        };
        let header = next().ok_or(SiweError::Malformed)?;
        let domain = header.strip_suffix(HEADER).ok_or(SiweError::Malformed)?;
        // A scheme in front of the domain is allowed by the grammar; Klimper's builds use none.
        if domain.is_empty() || domain.contains(char::is_whitespace) {
            return Err(SiweError::Malformed);
        }
        let address =
            Address::parse(next().ok_or(SiweError::Malformed)?).ok_or(SiweError::Malformed)?;
        if next() != Some("") {
            return Err(SiweError::Malformed);
        }
        let mut statement = None;
        let after = next().ok_or(SiweError::Malformed)?;
        if after.is_empty() {
            // No statement: the second blank line.
        } else if after.starts_with("URI: ") {
            // No statement and only one blank line: tolerated (some servers do this).
            at -= 1;
        } else {
            statement = Some(after.to_string());
            if next() != Some("") {
                return Err(SiweError::Malformed);
            }
        }

        let mut field = |name: &str, required: bool| -> Result<Option<String>, SiweError> {
            match lines.get(at) {
                Some(line) if line.starts_with(&format!("{name}: ")) => {
                    at += 1;
                    Ok(Some(line[name.len() + 2..].to_string()))
                }
                _ if required => Err(SiweError::Malformed),
                _ => Ok(None),
            }
        };
        let uri = field("URI", true)?.ok_or(SiweError::Malformed)?;
        let version = field("Version", true)?.ok_or(SiweError::Malformed)?;
        if version != "1" {
            return Err(SiweError::WrongVersion);
        }
        let chain_id = field("Chain ID", true)?
            .and_then(|id| id.parse::<u64>().ok())
            .ok_or(SiweError::Malformed)?;
        let nonce = field("Nonce", true)?.ok_or(SiweError::Malformed)?;
        let issued_at = field("Issued At", true)?.ok_or(SiweError::Malformed)?;
        let expiration_time = field("Expiration Time", false)?;
        let not_before = field("Not Before", false)?;
        let request_id = field("Request ID", false)?;
        if lines
            .get(at)
            .is_some_and(|line| line.starts_with("Resources:"))
        {
            return Err(SiweError::Resources);
        }
        // Nothing may follow, except one trailing line break.
        let rest: Vec<&&str> = lines[at.min(lines.len())..].iter().collect();
        if !(rest.is_empty() || rest == [&""]) {
            return Err(SiweError::Malformed);
        }
        if !valid_nonce(&nonce) {
            return Err(SiweError::BadNonce);
        }
        Ok(Self {
            domain: domain.to_string(),
            address,
            statement,
            uri,
            chain_id,
            nonce,
            issued_at,
            expiration_time,
            not_before,
            request_id,
        })
    }

    /// Parses `text` and accepts it only as a sign-in to `expected`.
    pub fn check(text: &str, expected: &Expected<'_>) -> Result<Self, SiweError> {
        let message = Self::parse(text)?;
        if message.domain != expected.domain {
            return Err(SiweError::WrongDomain);
        }
        if message.address != expected.address {
            return Err(SiweError::WrongAddress);
        }
        if message.uri != expected.uri {
            return Err(SiweError::WrongUri);
        }
        if message.chain_id != expected.chain_id {
            return Err(SiweError::WrongChain);
        }
        let issued = parse_rfc3339(&message.issued_at).ok_or(SiweError::Malformed)?;
        if issued.abs_diff(expected.now) > CLOCK_SKEW_SECS {
            return Err(SiweError::WrongTime);
        }
        let time = |text: &Option<String>| {
            text.as_deref()
                .map(|t| parse_rfc3339(t).ok_or(SiweError::Malformed))
                .transpose()
        };
        if time(&message.expiration_time)?.is_some_and(|expiry| expiry <= expected.now) {
            return Err(SiweError::WrongTime);
        }
        if time(&message.not_before)?.is_some_and(|start| start > expected.now + CLOCK_SKEW_SECS) {
            return Err(SiweError::WrongTime);
        }
        Ok(message)
    }

    /// The EIP-4361 text (used by the tests and the fake server).
    pub fn to_text(&self) -> String {
        let mut text = format!(
            "{}{HEADER}\n{}\n\n",
            self.domain,
            self.address.checksummed()
        );
        if let Some(statement) = &self.statement {
            text.push_str(statement);
            text.push('\n');
        }
        text.push('\n');
        let _ = write!(
            text,
            "URI: {}\nVersion: 1\nChain ID: {}\nNonce: {}\nIssued At: {}",
            self.uri, self.chain_id, self.nonce, self.issued_at
        );
        for (name, value) in [
            ("Expiration Time", &self.expiration_time),
            ("Not Before", &self.not_before),
            ("Request ID", &self.request_id),
        ] {
            if let Some(value) = value {
                let _ = write!(text, "\n{name}: {value}");
            }
        }
        text
    }
}

/// Days since 1970-01-01 of a civil date (H. Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// RFC 3339 (`2026-10-24T05:00:00Z`, `…:00.123Z`, `…:00+02:00`) to Unix seconds.
pub fn parse_rfc3339(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || !matches!(bytes[10], b'T' | b't')
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = text.get(range)?;
        part.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ if rest.len() == 6
            && (rest.starts_with('+') || rest.starts_with('-'))
            && &rest[3..4] == ":" =>
        {
            let hours: i64 = rest[1..3].parse().ok()?;
            let minutes: i64 = rest[4..6].parse().ok()?;
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            sign * (hours * 3600 + minutes * 60)
        }
        _ => return None,
    };
    let seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    u64::try_from(seconds).ok()
}

/// Unix seconds as RFC 3339 UTC.
pub fn rfc3339(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400).cast_signed();
    let secs = unix_seconds % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const NOW: u64 = 1_792_818_000; // 2026-10-24T05:00:00Z

    fn address() -> Address {
        Address::parse("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap()
    }

    fn message() -> SiweMessage {
        SiweMessage {
            domain: "card.example".into(),
            address: address(),
            statement: Some("Sign in to the card.".into()),
            uri: "https://card.example".into(),
            chain_id: 100,
            nonce: "abc123DEF456".into(),
            issued_at: "2026-10-24T05:00:00.000Z".into(),
            expiration_time: None,
            not_before: None,
            request_id: None,
        }
    }

    fn expected() -> Expected<'static> {
        Expected {
            domain: "card.example",
            uri: "https://card.example",
            address: address(),
            chain_id: 100,
            now: NOW,
        }
    }

    #[test]
    fn the_eip4361_layout() {
        assert_eq!(
            message().to_text(),
            "card.example wants you to sign in with your Ethereum account:\n\
             0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266\n\
             \n\
             Sign in to the card.\n\
             \n\
             URI: https://card.example\n\
             Version: 1\n\
             Chain ID: 100\n\
             Nonce: abc123DEF456\n\
             Issued At: 2026-10-24T05:00:00.000Z"
        );
    }

    #[test]
    fn a_sign_in_for_this_build_is_accepted() {
        assert_eq!(
            SiweMessage::check(&message().to_text(), &expected()),
            Ok(message())
        );
        let mut m = message();
        m.statement = None;
        m.expiration_time = Some("2026-10-24T05:10:00Z".into());
        m.request_id = Some("r-1".into());
        assert_eq!(SiweMessage::check(&m.to_text(), &expected()), Ok(m.clone()));
        // A trailing line break is fine.
        assert_eq!(
            SiweMessage::check(&(m.to_text() + "\n"), &expected()),
            Ok(m)
        );
    }

    #[test]
    fn anything_else_is_refused() {
        type Change = Box<dyn Fn(&mut SiweMessage)>;
        let cases: Vec<(Change, SiweError)> = vec![
            (
                Box::new(|m| m.domain = "evil.example".into()),
                SiweError::WrongDomain,
            ),
            (
                Box::new(|m| m.uri = "https://card.example/other".into()),
                SiweError::WrongUri,
            ),
            (Box::new(|m| m.chain_id = 1), SiweError::WrongChain),
            (
                Box::new(|m| {
                    m.address =
                        Address::parse("0x9858EfFD232B4033E47d90003D41EC34EcaEda94").unwrap();
                }),
                SiweError::WrongAddress,
            ),
            (Box::new(|m| m.nonce = "short".into()), SiweError::BadNonce),
            (
                Box::new(|m| m.nonce = "abc 1234567".into()),
                SiweError::BadNonce,
            ),
            (
                Box::new(|m| m.issued_at = "2026-10-24T04:00:00Z".into()),
                SiweError::WrongTime,
            ),
            (
                Box::new(|m| m.issued_at = "2026-10-24T06:00:00Z".into()),
                SiweError::WrongTime,
            ),
            (
                Box::new(|m| m.expiration_time = Some("2026-10-24T04:59:00Z".into())),
                SiweError::WrongTime,
            ),
            (
                Box::new(|m| m.not_before = Some("2026-10-25T00:00:00Z".into())),
                SiweError::WrongTime,
            ),
            (
                Box::new(|m| m.issued_at = "yesterday".into()),
                SiweError::Malformed,
            ),
        ];
        for (change, error) in cases {
            let mut m = message();
            change(&mut m);
            assert_eq!(
                SiweMessage::check(&m.to_text(), &expected()),
                Err(error),
                "{m:?}"
            );
        }
    }

    #[test]
    fn text_outside_the_grammar_is_refused() {
        let good = message().to_text();
        for (bad, error) in [
            (
                good.replace("Version: 1", "Version: 2"),
                SiweError::WrongVersion,
            ),
            (
                good.clone() + "\nResources:\n- https://card.example/x",
                SiweError::Resources,
            ),
            (
                good.clone() + "\nTransfer: everything",
                SiweError::Malformed,
            ),
            (
                good.replacen("account:", "account", 1),
                SiweError::Malformed,
            ),
            (
                "Please sign this transaction".to_string(),
                SiweError::Malformed,
            ),
            (String::new(), SiweError::Malformed),
        ] {
            assert_eq!(SiweMessage::check(&bad, &expected()), Err(error), "{bad:?}");
        }
    }

    #[test]
    fn dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(NOW), "2026-10-24T05:00:00Z");
        assert_eq!(parse_rfc3339("2026-10-24T05:00:00Z"), Some(NOW));
        assert_eq!(parse_rfc3339("2026-10-24T05:00:00.123Z"), Some(NOW));
        assert_eq!(parse_rfc3339("2026-10-24T07:00:00+02:00"), Some(NOW));
        assert_eq!(parse_rfc3339("2000-02-29T00:00:00Z"), Some(951_782_400));
        for bad in [
            "2026-13-01T00:00:00Z",
            "2026-10-24 05:00:00Z",
            "2026-10-24T05:00:00",
            "2026-10-24T05:00:00.Z",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }
}
