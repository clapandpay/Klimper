// SPDX-License-Identifier: AGPL-3.0-or-later
//! Gnosis Pay onboarding, step by step (TASK-150).
//!
//! `GET /user/onboarding` says what the user must do next; Klimper turns it into one step
//! with only what that step needs. Identity data stays between the user and Gnosis Pay or
//! Sumsub:
//!
//! * Klimper never calls `GET /user` (it returns name, date of birth and address).
//! * The e-mail address and phone number go to Gnosis Pay as typed and are neither stored
//!   nor logged here.
//! * Identity verification happens on Sumsub's own page, opened in the system browser, not
//!   inside the wallet. Klimper hands the webview that link only if it is an https link to
//!   Sumsub.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::gnosis_pay::{ApiError, ApiResult, GnosisPay};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(alias = "currentVersion")]
    pub version: String,
    pub name: String,
    pub url: Option<String>,
    #[serde(default)]
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub question: String,
    pub answers: Vec<String>,
}

/// The one thing to do next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "step", rename_all = "camelCase")]
pub enum Step {
    /// No Gnosis Pay user for this address yet: e-mail (and its code).
    Register,
    AcceptTerms {
        terms: Vec<Term>,
    },
    /// Gnosis Pay is preparing the identity check; poll.
    WaitingKyc,
    /// Identity check on Sumsub's page.
    CompleteKyc {
        url: Option<String>,
        resubmission: bool,
    },
    /// A person at Gnosis Pay is looking at it; nothing to do.
    KycReview,
    SourceOfFunds {
        questions: Vec<Question>,
    },
    CreateAccount,
    /// The Safe is being set up; poll.
    WaitingAccount,
    VerifyPhone,
    Completed,
    Rejected,
    /// A status this build does not know: show it and poll, never guess.
    Unknown {
        status: String,
    },
}

/// Only an https link to Sumsub is handed to the webview to open.
pub fn sumsub_link(url: Option<&str>) -> Option<String> {
    let url = url?;
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.split(':').next()?;
    (host == "sumsub.com" || host.ends_with(".sumsub.com")).then(|| url.to_string())
}

pub fn step_of(body: &Value) -> Step {
    let status = body["status"].as_str().unwrap_or_default();
    let list = |key: &str| body.get(key).cloned().unwrap_or(Value::Array(Vec::new()));
    match status {
        "action_accept_tos" => Step::AcceptTerms {
            terms: serde_json::from_value(list("terms")).unwrap_or_default(),
        },
        "waiting_kyc_setup" => Step::WaitingKyc,
        "action_complete_kyc" | "action_kyc_resubmission_requested" => Step::CompleteKyc {
            url: sumsub_link(body["webSdkUrl"].as_str()),
            resubmission: status == "action_kyc_resubmission_requested",
        },
        "kyc_manual_review" => Step::KycReview,
        "action_complete_sof" => Step::SourceOfFunds {
            questions: serde_json::from_value(list("questions")).unwrap_or_default(),
        },
        "action_create_account" => Step::CreateAccount,
        "waiting_account_setup" => Step::WaitingAccount,
        "action_verify_phone" => Step::VerifyPhone,
        "completed" => Step::Completed,
        "rejected" => Step::Rejected,
        other => Step::Unknown {
            status: other.chars().take(64).collect(),
        },
    }
}

/// Whether an error means "this address has no Gnosis Pay user yet". The documented API
/// does not say what `GET /user/onboarding` answers before `POST /user`; a 404 or a
/// user-not-found code is taken as that (to be confirmed in the sandbox, TASK-147).
fn unregistered(error: &ApiError) -> bool {
    match error {
        ApiError::Status { status: 404, .. } => true,
        ApiError::Status {
            code: Some(code), ..
        } => code.contains("USER_NOT_FOUND") || code == "USER_NOT_REGISTERED",
        _ => false,
    }
}

impl GnosisPay {
    pub async fn onboarding(&self) -> ApiResult<Step> {
        match self.get::<Value>("/user/onboarding").await {
            Ok(body) => Ok(step_of(&body)),
            Err(error) if unregistered(&error) => Ok(Step::Register),
            Err(error) => Err(error),
        }
    }

    /// Sends the e-mail code. `false` when this program needs no code.
    pub async fn send_email_code(&self, email: &str) -> ApiResult<bool> {
        match self
            .post::<Value>("/email-verification", &json!({ "email": email }))
            .await
        {
            Ok(_) => Ok(true),
            Err(error) if error.code() == Some("EMAIL_VERIFICATION_NOT_REQUIRED") => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub async fn accept_terms(&self, terms: &[Term]) -> ApiResult<()> {
        let terms: Vec<Value> = terms
            .iter()
            .map(|t| json!({ "type": t.kind, "version": t.version }))
            .collect();
        self.post::<Value>("/user/terms", &json!({ "terms": terms }))
            .await
            .map(drop)
    }

    pub async fn answer_source_of_funds(&self, answers: &[(String, String)]) -> ApiResult<()> {
        let answers: Vec<Value> = answers
            .iter()
            .map(|(q, a)| json!({ "question": q, "answer": a }))
            .collect();
        self.post::<Value>("/source-of-funds", &json!({ "answers": answers }))
            .await
            .map(drop)
    }

    /// Deploys the Safe; its owner is the signed-in card address, for good.
    pub async fn create_account(&self) -> ApiResult<()> {
        self.post::<Value>("/user/account", &json!({}))
            .await
            .map(drop)
    }

    pub async fn send_phone_code(&self, phone: &str) -> ApiResult<()> {
        self.post::<Value>("/phone/verification", &json!({ "phone": phone }))
            .await
            .map(drop)
    }

    pub async fn verify_phone(&self, code: &str) -> ApiResult<()> {
        self.post::<Value>("/phone", &json!({ "code": code }))
            .await
            .map(drop)
    }

    pub async fn order_virtual_card(&self, name: &str) -> ApiResult<()> {
        self.post::<Value>("/cards/virtual", &json!({ "cardName": name }))
            .await
            .map(drop)
    }
}

/// E.164: a plus and 8 to 15 digits.
pub fn valid_phone(phone: &str) -> bool {
    phone.strip_prefix('+').is_some_and(|digits| {
        (8..=15).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
    })
}

/// Six digits.
pub fn valid_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit())
}

/// The shape of an address, nothing more (Gnosis Pay checks it).
pub fn valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !email.contains(char::is_whitespace)
        && email.len() <= 254
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::fake::FakeGnosisPay;
    use crate::gnosis_pay::tests::signed_in;

    #[test]
    fn only_sumsub_links_are_handed_out() {
        let good = "https://in.sumsub.com/websdk/p/sbx_abc";
        assert_eq!(sumsub_link(Some(good)).as_deref(), Some(good));
        assert_eq!(
            sumsub_link(Some("https://sumsub.com/x")).as_deref(),
            Some("https://sumsub.com/x")
        );
        for bad in [
            "http://in.sumsub.com/websdk",
            "https://sumsub.com.evil.example/websdk",
            "https://evil.example/?https://in.sumsub.com",
            "https://evilsumsub.com/",
            "javascript:alert(1)",
        ] {
            assert_eq!(sumsub_link(Some(bad)), None, "{bad}");
        }
    }

    #[test]
    fn every_documented_status_has_a_step() {
        for (status, step) in [
            ("waiting_kyc_setup", Step::WaitingKyc),
            ("kyc_manual_review", Step::KycReview),
            ("action_create_account", Step::CreateAccount),
            ("waiting_account_setup", Step::WaitingAccount),
            ("action_verify_phone", Step::VerifyPhone),
            ("completed", Step::Completed),
            ("rejected", Step::Rejected),
        ] {
            assert_eq!(step_of(&json!({ "status": status })), step);
        }
        assert_eq!(
            step_of(
                &json!({ "status": "action_kyc_resubmission_requested", "webSdkUrl": "https://in.sumsub.com/x" })
            ),
            Step::CompleteKyc {
                url: Some("https://in.sumsub.com/x".into()),
                resubmission: true
            }
        );
        assert_eq!(
            step_of(&json!({ "status": "something_new" })),
            Step::Unknown {
                status: "something_new".into()
            }
        );
        // A rejection's reason and comments are not passed on: they are about the person.
        let rejected = serde_json::to_value(step_of(
            &json!({ "status": "rejected", "reason": "x", "comments": "y" }),
        ))
        .unwrap();
        assert_eq!(rejected, json!({ "step": "rejected" }));
    }

    #[test]
    fn input_shapes() {
        assert!(valid_phone("+15550100123"));
        assert!(!valid_phone("0043 1234"));
        assert!(!valid_phone("+12"));
        assert!(valid_code("123456"));
        assert!(!valid_code("12345a"));
        assert!(valid_email("someone@example.org"));
        assert!(!valid_email("someone@example"));
        assert!(!valid_email("some one@example.org"));
    }

    /// The whole onboarding against the fake, in the order Gnosis Pay drives it.
    #[tokio::test]
    async fn from_e_mail_to_a_card() {
        let fake = FakeGnosisPay::start().await;
        let api = signed_in(&fake).await;

        assert_eq!(api.onboarding().await.unwrap(), Step::Register);
        assert!(api.send_email_code("someone@example.org").await.unwrap());
        assert_eq!(
            api.register("someone@example.org", Some("000000"))
                .await
                .unwrap_err()
                .code(),
            Some("INVALID_OTP")
        );
        api.register("someone@example.org", Some("123456"))
            .await
            .unwrap();

        let Step::AcceptTerms { terms } = api.onboarding().await.unwrap() else {
            panic!("terms first")
        };
        assert_eq!(terms.len(), 2);
        api.accept_terms(&terms).await.unwrap();

        let Step::CompleteKyc { url, resubmission } = api.onboarding().await.unwrap() else {
            panic!("then KYC")
        };
        assert!(url.unwrap().starts_with("https://in.sumsub.com/"));
        assert!(!resubmission);
        reqwest::Client::new()
            .post(format!("http://{}/fake/kyc-approved", fake.address))
            .send()
            .await
            .unwrap();

        let Step::SourceOfFunds { questions } = api.onboarding().await.unwrap() else {
            panic!("then source of funds")
        };
        let answers: Vec<(String, String)> = questions
            .iter()
            .map(|q| (q.question.clone(), q.answers[0].clone()))
            .collect();
        api.answer_source_of_funds(&answers).await.unwrap();

        assert_eq!(api.onboarding().await.unwrap(), Step::CreateAccount);
        api.create_account().await.unwrap();

        assert_eq!(api.onboarding().await.unwrap(), Step::VerifyPhone);
        api.send_phone_code("+15550100123").await.unwrap();
        api.verify_phone("123456").await.unwrap();

        assert_eq!(api.onboarding().await.unwrap(), Step::Completed);
        api.order_virtual_card("Klimper").await.unwrap();
        assert_eq!(fake.state().await.cards.len(), 1);
    }
}
