// SPDX-License-Identifier: AGPL-3.0-or-later
//! A fake Gnosis Pay user API, for the tests and for running the card screens locally
//! without a sandbox registration:
//!
//! ```sh
//! cargo run -p tauri-plugin-klimper-card --features fake --example fake-gnosis-pay
//! KLIMPER_CARD_API=http://127.0.0.1:8787/user-api KLIMPER_CARD_APP_NAME=klimper-dev \
//!   KLIMPER_CARD_DOMAIN=localhost VITE_ENABLE_CARD=true pnpm tauri dev
//! ```
//!
//! It follows the documented shapes (`OpenAPI` of 2026-10, `docs/card.md`) closely enough to
//! exercise every path Klimper uses: SIWE with a real signature check, rotating refresh
//! tokens that log the session out when reused, the onboarding states in order, the account,
//! balances, cards and a statement. Everything is made up; there is no personal data.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, MutexGuard};

use crate::evm::{Address, personal_digest, recover};
use crate::gnosis_pay::Settings;
use crate::siwe::{SiweMessage, rfc3339};

pub const APP_NAME: &str = "klimper-dev";
pub const DOMAIN: &str = "localhost";
pub const EURE: &str = "0x420ca0f9b9b604ce0fd9c18ef134c705e5fa3430";

/// Where the user is in Gnosis Pay's onboarding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Unregistered,
    Terms,
    Kyc,
    SourceOfFunds,
    CreateAccount,
    Phone,
    Completed,
}

#[derive(Debug)]
pub struct FakeState {
    pub siwe_domain_override: Option<String>,
    pub siwe_issued_at_override: Option<String>,
    nonce: u64,
    access: HashSet<String>,
    refresh: HashSet<String>,
    used_refresh: HashSet<String>,
    pub sign_ins: usize,
    pub refreshes: usize,
    pub reused_refresh_tokens: usize,
    pub logouts: usize,
    pub address: Option<Address>,
    pub stage: Stage,
    /// A code was "sent" (the fake's code is always 123456).
    pub email_code_sent: bool,
    pub phone_code_sent: bool,
    pub cards: Vec<Value>,
    /// Spendable `EURe` in wei (18 decimals).
    pub spendable: u128,
    pub statement: Vec<Value>,
}

impl FakeState {
    fn new() -> Self {
        Self {
            siwe_domain_override: None,
            siwe_issued_at_override: None,
            nonce: 0,
            access: HashSet::new(),
            refresh: HashSet::new(),
            used_refresh: HashSet::new(),
            sign_ins: 0,
            refreshes: 0,
            reused_refresh_tokens: 0,
            logouts: 0,
            address: None,
            stage: Stage::Unregistered,
            email_code_sent: false,
            phone_code_sent: false,
            cards: Vec::new(),
            spendable: 0,
            statement: Vec::new(),
        }
    }

    fn token(&mut self, kind: &str) -> String {
        self.nonce += 1;
        format!("{kind}{:016x}", self.nonce)
    }

    /// Every access token stops working (as after 15 minutes).
    pub fn expire_access(&mut self) {
        self.access.clear();
    }

    /// Every refresh token stops working (as after 7 days).
    pub fn revoke_refresh(&mut self) {
        self.refresh.clear();
    }

    /// A fully onboarded user with an active card, some money and three payments.
    pub fn onboarded(&mut self, address: Address) {
        self.address = Some(address);
        self.stage = Stage::Completed;
        self.spendable = 123_450_000_000_000_000_000; // 123.45 EURe
        self.cards = vec![card("card-1", "active")];
        self.statement = vec![
            payment(
                "t-3",
                "2026-10-23T18:04:00Z",
                "Bakery Example",
                "4.20",
                "settled",
            ),
            payment(
                "t-2",
                "2026-10-22T12:30:00Z",
                "Bookshop Example",
                "18.90",
                "settled",
            ),
            payment(
                "t-1",
                "2026-10-21T08:15:00Z",
                "Coffee Example",
                "3.10",
                "pending",
            ),
        ];
    }
}

fn card(id: &str, status: &str) -> Value {
    json!({
        "id": id, "accountId": "acc-1", "cardholderId": "ch-1", "status": status, "type": "virtual",
        "nativeCurrency": "EUR", "last4Digits": "4242", "bin": "400000", "brand": "VISA",
        "expirationDate": "10/29",
    })
}

fn payment(id: &str, at: &str, merchant: &str, amount: &str, status: &str) -> Value {
    json!({
        "id": id, "type": "card_transaction", "createdAt": at, "status": status,
        "billingAmount": amount, "billingCurrency": "EUR",
        "transactionAmount": amount, "transactionCurrency": "EUR",
        "merchant": { "name": merchant, "city": "Example City", "country": "AT" },
        "mcc": "5814",
    })
}

type Shared = Arc<Mutex<FakeState>>;

#[derive(Debug)]
pub struct FakeGnosisPay {
    pub address: SocketAddr,
    state: Shared,
}

impl FakeGnosisPay {
    /// Serves on a free local port.
    pub async fn start() -> Self {
        Self::serve("127.0.0.1:0").await
    }

    pub async fn serve(bind: &str) -> Self {
        let state: Shared = Arc::new(Mutex::new(FakeState::new()));
        let listener = tokio::net::TcpListener::bind(bind)
            .await
            .expect("a local port");
        let address = listener.local_addr().expect("a bound address");
        let app = router(state.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { address, state }
    }

    pub fn api(&self) -> String {
        format!("http://{}/user-api", self.address)
    }

    pub fn settings(&self) -> Settings {
        Settings::from_values(Some(&self.api()), Some(APP_NAME), Some(DOMAIN), None, None)
            .expect("fake settings")
    }

    pub async fn state(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().await
    }
}

fn router(state: Shared) -> Router {
    Router::new()
        .route("/user-api/auth/siwe/:address", get(siwe_challenge))
        .route("/user-api/auth/siwe", post(siwe_verify))
        .route("/user-api/auth/refresh", post(refresh))
        .route("/user-api/auth/logout", post(logout))
        .route("/user-api/email-verification", post(email_verification))
        .route("/user-api/user", post(register))
        .route("/user-api/user/onboarding", get(onboarding))
        .route("/user-api/terms", get(terms))
        .route("/user-api/user/terms", post(accept_terms))
        .route(
            "/user-api/source-of-funds",
            get(source_of_funds).post(answer_source_of_funds),
        )
        .route("/user-api/user/account", get(account).post(create_account))
        .route("/user-api/phone/verification", post(phone_verification))
        .route("/user-api/phone", post(phone))
        .route("/user-api/user/balances/v2", get(balances))
        .route("/user-api/user/statement", get(statement))
        .route("/user-api/cards", get(cards))
        .route("/user-api/cards/virtual", post(order_card))
        .route("/user-api/cards/:id/block", post(block))
        .route("/user-api/cards/:id/unblock", post(unblock))
        // The fake's own controls (not Gnosis Pay's): finish KYC, pay with the card.
        .route("/fake/kyc-approved", post(fake_kyc_approved))
        .route("/fake/pay", post(fake_pay))
        .route("/fake/onboarded", post(fake_onboarded))
        .with_state(state)
}

fn error(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({ "error": code }))).into_response()
}

async fn authed(state: &Shared, headers: &HeaderMap) -> Result<(), Response> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if state.lock().await.access.contains(token) {
        Ok(())
    } else {
        Err(error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED"))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChallengeQuery {
    domain: String,
    uri: String,
    app_name: String,
}

async fn siwe_challenge(
    State(state): State<Shared>,
    Path(address): Path<String>,
    Query(query): Query<ChallengeQuery>,
) -> Response {
    if query.app_name != APP_NAME {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"success": false, "error": "Unknown app name"})),
        )
            .into_response();
    }
    if query.domain != DOMAIN {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"success": false, "error": "Domain not allowed for this app"})),
        )
            .into_response();
    }
    let Some(address) = Address::parse(&address) else {
        return error(StatusCode::BAD_REQUEST, "INVALID_ADDRESS");
    };
    let mut state = state.lock().await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let nonce = state.token("n");
    let message = SiweMessage {
        domain: state.siwe_domain_override.clone().unwrap_or(query.domain),
        address,
        statement: Some("Sign in with Ethereum to the app.".into()),
        uri: query.uri,
        chain_id: 100,
        nonce,
        issued_at: state
            .siwe_issued_at_override
            .clone()
            .unwrap_or_else(|| rfc3339(now)),
        expiration_time: None,
        not_before: None,
        request_id: None,
    };
    Json(json!({ "success": true, "message": message.to_text() })).into_response()
}

#[derive(Deserialize)]
struct Verify {
    address: String,
    message: String,
    signature: String,
}

async fn siwe_verify(State(state): State<Shared>, Json(body): Json<Verify>) -> Response {
    let signature: Option<[u8; 65]> = body
        .signature
        .strip_prefix("0x")
        .and_then(|s| hex::decode(s).ok())
        .and_then(|b| b.try_into().ok());
    let address = Address::parse(&body.address);
    let signer = signature.and_then(|s| recover(&personal_digest(body.message.as_bytes()), &s));
    if address.is_none() || signer != address {
        return error(StatusCode::UNAUTHORIZED, "INVALID_SIGNATURE");
    }
    let mut state = state.lock().await;
    state.sign_ins += 1;
    if state.address.is_none() {
        state.address = address;
    }
    let access = state.token("a");
    let refresh = state.token("r");
    state.access.insert(access.clone());
    state.refresh.insert(refresh.clone());
    Json(json!({ "success": true, "accessToken": access, "refreshToken": refresh })).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefreshBody {
    refresh_token: String,
}

async fn refresh(State(state): State<Shared>, Json(body): Json<RefreshBody>) -> Response {
    let mut state = state.lock().await;
    if state.used_refresh.contains(&body.refresh_token) {
        // Reuse: the whole family is logged out, as Gnosis Pay does.
        state.reused_refresh_tokens += 1;
        state.access.clear();
        state.refresh.clear();
        return error(StatusCode::UNAUTHORIZED, "REFRESH_TOKEN_REUSED");
    }
    if !state.refresh.remove(&body.refresh_token) {
        return error(StatusCode::UNAUTHORIZED, "INVALID_REFRESH_TOKEN");
    }
    state.used_refresh.insert(body.refresh_token);
    state.refreshes += 1;
    let access = state.token("a");
    let refresh = state.token("r");
    state.access.insert(access.clone());
    state.refresh.insert(refresh.clone());
    Json(json!({ "success": true, "accessToken": access, "refreshToken": refresh })).into_response()
}

async fn logout(State(state): State<Shared>, Json(_body): Json<RefreshBody>) -> Response {
    let mut state = state.lock().await;
    state.logouts += 1;
    state.access.clear();
    state.refresh.clear();
    Json(json!({ "success": true })).into_response()
}

async fn email_verification(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    state.lock().await.email_code_sent = true;
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
struct Register {
    email: String,
    otp: Option<String>,
}

async fn register(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Register>,
) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    let mut state = state.lock().await;
    if state.stage != Stage::Unregistered {
        return error(StatusCode::CONFLICT, "USER_ALREADY_EXISTS");
    }
    if !body.email.contains('@') {
        return error(StatusCode::BAD_REQUEST, "INVALID_EMAIL");
    }
    match body.otp.as_deref() {
        None => return error(StatusCode::BAD_REQUEST, "EMAIL_VERIFICATION_REQUIRED"),
        Some("123456") if state.email_code_sent => {}
        Some(_) => return error(StatusCode::BAD_REQUEST, "INVALID_OTP"),
    }
    state.stage = Stage::Terms;
    let access = state.token("a");
    state.access.insert(access.clone());
    (
        StatusCode::CREATED,
        Json(json!({ "userId": "user-1", "accessToken": access })),
    )
        .into_response()
}

async fn onboarding(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    let state = state.lock().await;
    let body = match state.stage {
        Stage::Unregistered => return error(StatusCode::NOT_FOUND, "USER_NOT_FOUND"),
        Stage::Terms => json!({ "status": "action_accept_tos", "terms": terms_list() }),
        Stage::Kyc => json!({
            "status": "action_complete_kyc",
            "webSdkUrl": "https://in.sumsub.com/websdk/p/sbx_example",
            "mobileSdkToken": "_act-sbx-example",
        }),
        Stage::SourceOfFunds => {
            json!({ "status": "action_complete_sof", "questions": sof_questions(), "version": 1 })
        }
        Stage::CreateAccount => json!({ "status": "action_create_account" }),
        Stage::Phone => json!({ "status": "action_verify_phone" }),
        Stage::Completed => json!({ "status": "completed" }),
    };
    Json(body).into_response()
}

fn terms_list() -> Value {
    json!([
        { "type": "general-tos", "currentVersion": "TOS_GENERAL_VERSION_1", "name": "Terms of Service",
          "url": "https://legal.gnosispay.com/", "accepted": false },
        { "type": "card-monavate-tos", "currentVersion": "TOS_CARD_VERSION_1", "name": "Cardholder Terms",
          "url": "https://legal.gnosispay.com/", "accepted": false },
    ])
}

fn sof_questions() -> Value {
    json!([
        { "id": "q1", "question": "What is your main source of income?",
          "answers": ["Employment", "Self-employment", "Savings", "Other"] },
        { "id": "q2", "question": "How much do you expect to spend each month?",
          "answers": ["Up to 1,000 EUR", "1,000 to 5,000 EUR", "More than 5,000 EUR"] },
    ])
}

async fn terms(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    Json(json!({ "terms": terms_list() })).into_response()
}

async fn accept_terms(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    let accepted = body["terms"].as_array().map_or(0, Vec::len);
    let mut state = state.lock().await;
    if state.stage != Stage::Terms || accepted != 2 {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "TERMS_INCOMPLETE");
    }
    state.stage = Stage::Kyc;
    Json(json!({ "ok": true })).into_response()
}

async fn source_of_funds(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    Json(json!({ "questions": sof_questions(), "version": 1 })).into_response()
}

async fn answer_source_of_funds(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    let answered = body["answers"].as_array().map_or(0, Vec::len);
    let mut state = state.lock().await;
    if state.stage != Stage::SourceOfFunds || answered != 2 {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "ANSWERS_INCOMPLETE");
    }
    state.stage = Stage::CreateAccount;
    StatusCode::CREATED.into_response()
}

async fn create_account(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    let mut state = state.lock().await;
    if state.stage != Stage::CreateAccount {
        return error(StatusCode::CONFLICT, "ACCOUNT_ALREADY_EXISTS");
    }
    state.stage = Stage::Phone;
    (StatusCode::CREATED, Json(json!({ "accountId": "acc-1" }))).into_response()
}

async fn account(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    let state = state.lock().await;
    if matches!(
        state.stage,
        Stage::Unregistered
            | Stage::Terms
            | Stage::Kyc
            | Stage::SourceOfFunds
            | Stage::CreateAccount
    ) {
        return error(StatusCode::NOT_FOUND, "ACCOUNT_NOT_FOUND");
    }
    let owner = state.address.map(|a| a.checksummed()).unwrap_or_default();
    Json(json!({
        "safeOwnerAddress": owner,
        "safeAddress": "0x5afe5afE5afE5AfE5aFE5aFe5AFe5afe5afE5aFE",
        "chain": { "name": "gnosis", "id": "100" },
        "nativeCurrency": "EUR",
        "additionalCurrencies": [],
        "status": "active",
    }))
    .into_response()
}

#[derive(Deserialize)]
struct PhoneNumber {
    phone: String,
}

async fn phone_verification(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<PhoneNumber>,
) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    if !body.phone.starts_with('+') {
        return error(StatusCode::BAD_REQUEST, "INVALID_PHONE");
    }
    state.lock().await.phone_code_sent = true;
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
struct PhoneCode {
    code: String,
}

async fn phone(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<PhoneCode>,
) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    let mut state = state.lock().await;
    if !state.phone_code_sent || body.code != "123456" {
        return error(StatusCode::BAD_REQUEST, "INVALID_OTP");
    }
    state.stage = Stage::Completed;
    Json(json!({ "ok": true })).into_response()
}

async fn balances(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    let state = state.lock().await;
    let spendable = state.spendable.to_string();
    Json(json!({
        "data": [{
            "currency": "EUR", "decimals": 18, "spendable": spendable, "nonSpendable": "0",
            "processingDeposits": "0", "processingWithdrawals": "0",
            "tokens": [{ "token": EURE, "decimals": 18, "spendable": spendable, "nonSpendable": "0",
                         "processingDeposits": "0", "maxInstantWithdraw": spendable }],
        }]
    }))
    .into_response()
}

async fn statement(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    let state = state.lock().await;
    Json(json!({ "data": state.statement, "nextCursor": null, "hasNext": false })).into_response()
}

async fn cards(State(state): State<Shared>, headers: HeaderMap) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    Json(json!({ "cards": state.lock().await.cards })).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrderCard {
    card_name: String,
}

async fn order_card(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<OrderCard>,
) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    if body.card_name.is_empty() || body.card_name.len() > 50 {
        return error(StatusCode::BAD_REQUEST, "INVALID_CARD_NAME");
    }
    let mut state = state.lock().await;
    if state.stage != Stage::Completed {
        return error(StatusCode::UNPROCESSABLE_ENTITY, "ACCOUNT_NOT_READY");
    }
    if !state.cards.is_empty() {
        return error(StatusCode::CONFLICT, "CARD_LIMIT_REACHED");
    }
    // Gnosis Pay provisions in about ten seconds; the fake is instant.
    state.cards.push(card("card-1", "active"));
    (StatusCode::ACCEPTED, Json(json!({ "cardId": "card-1" }))).into_response()
}

async fn set_card_status(
    state: Shared,
    headers: HeaderMap,
    id: String,
    from: &str,
    to: &str,
) -> Response {
    if let Err(response) = authed(&state, &headers).await {
        return response;
    }
    let mut state = state.lock().await;
    let Some(card) = state.cards.iter_mut().find(|c| c["id"] == id.as_str()) else {
        return error(StatusCode::NOT_FOUND, "CARD_NOT_FOUND");
    };
    if card["status"] != from {
        return error(StatusCode::CONFLICT, "INVALID_CARD_STATUS");
    }
    card["status"] = Value::String(to.into());
    Json(json!({ "ok": true })).into_response()
}

async fn block(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    set_card_status(state, headers, id, "active", "blocked").await
}

async fn unblock(
    State(state): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    set_card_status(state, headers, id, "blocked", "active").await
}

async fn fake_kyc_approved(State(state): State<Shared>) -> Response {
    let mut state = state.lock().await;
    if state.stage == Stage::Kyc {
        state.stage = Stage::SourceOfFunds;
    }
    Json(json!({ "ok": true })).into_response()
}

async fn fake_onboarded(State(state): State<Shared>) -> Response {
    let mut state = state.lock().await;
    let address = state.address.unwrap_or(Address([0; 20]));
    state.onboarded(address);
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
struct Pay {
    merchant: String,
    /// Euros, e.g. "4.20".
    amount: String,
}

async fn fake_pay(State(state): State<Shared>, Json(body): Json<Pay>) -> Response {
    let mut state = state.lock().await;
    let cents: u128 = body
        .amount
        .split_once('.')
        .map_or_else(
            || body.amount.parse::<u128>().ok().map(|e| e * 100),
            |(e, c)| Some(e.parse::<u128>().ok()? * 100 + c.parse::<u128>().ok()?),
        )
        .unwrap_or(0);
    let wei = cents * 10_000_000_000_000_000;
    let blocked = state.cards.first().is_none_or(|c| c["status"] != "active");
    let status = if blocked || wei > state.spendable {
        "declined"
    } else {
        "pending"
    };
    if status == "pending" {
        state.spendable -= wei;
    }
    let id = format!("t-{}", state.statement.len() + 1);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    state.statement.insert(
        0,
        payment(&id, &rfc3339(now), &body.merchant, &body.amount, status),
    );
    Json(json!({ "status": status })).into_response()
}
