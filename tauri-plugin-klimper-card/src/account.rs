// SPDX-License-Identifier: AGPL-3.0-or-later
//! The card account once onboarding is done (TASK-151): the Safe, what can be spent, the
//! card, the payments; freezing and unfreezing the card.
//!
//! Only what the screen shows is passed to the webview. A payment keeps the merchant's
//! name, the amount and the state. Its location, coordinates and card-scheme details are
//! dropped here.

use serde::Serialize;
use serde_json::Value;

use crate::gnosis_pay::{ApiError, ApiResult, GnosisPay};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub safe_address: Option<String>,
    pub owner_address: Option<String>,
    /// `provisioning`, `active`, `blocked`, `closed`.
    pub status: String,
    /// ISO 4217, e.g. `EUR`.
    pub currency: String,
    /// Decimal strings in `currency`, e.g. `123.45`.
    pub spendable: String,
    pub pending: String,
    pub cards: Vec<CardSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardSummary {
    pub id: String,
    /// `provisioning`, `pending_activation`, `active`, `blocked`, `canceled`.
    pub status: String,
    /// `virtual` or `plastic`.
    pub kind: String,
    pub last4: Option<String>,
    pub brand: Option<String>,
    pub expires: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Payment {
    pub id: String,
    /// `card`, `deposit`, `withdrawal` or what Gnosis Pay calls it.
    pub kind: String,
    pub created_at: Option<String>,
    pub status: String,
    pub merchant: Option<String>,
    /// Decimal string, as Gnosis Pay gives it.
    pub amount: Option<String>,
    pub currency: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Statement {
    pub items: Vec<Payment>,
    pub next_cursor: Option<String>,
}

/// A raw integer amount with `decimals` as a decimal string with at most two places
/// (cents), rounded down: `123450000000000000000`, 18 → `123.45`.
pub fn format_units(raw: &str, decimals: u32) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = raw.trim_start_matches('0');
    let width = decimals as usize;
    let padded = format!("{digits:0>width$}", width = width + 1);
    let (whole, fraction) = padded.split_at(padded.len() - width);
    let cents = format!("{:0<2}", &fraction[..fraction.len().min(2)]);
    Some(if width == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{cents}")
    })
}

fn text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let v = value.get(key)?;
        v.as_str()
            .map(str::to_string)
            .or_else(|| v.as_number().map(ToString::to_string))
    })
}

fn summary(card: &Value) -> Option<CardSummary> {
    Some(CardSummary {
        id: text(card, &["id"])?,
        status: text(card, &["status"]).unwrap_or_else(|| "unknown".into()),
        kind: text(card, &["type"]).unwrap_or_else(|| "virtual".into()),
        last4: text(card, &["last4Digits", "lastFourDigits"]),
        brand: text(card, &["brand"]),
        expires: text(card, &["expirationDate"]),
    })
}

/// One statement entry, reduced to what the list shows. The statement's exact fields are
/// only partly documented (TASK-147), so a few names are tried, and nothing else is kept.
pub fn payment(entry: &Value) -> Option<Payment> {
    let merchant = entry
        .pointer("/enrichment/merchant/name")
        .or_else(|| entry.pointer("/merchant/name"))
        .or_else(|| entry.get("merchant").filter(|m| m.is_string()))
        .and_then(Value::as_str)
        .map(|name| name.trim().chars().take(80).collect());
    let mut amount = text(entry, &["billingAmount", "amount", "transactionAmount"]);
    if let (Some(raw), Some(decimals)) = (&amount, entry.get("decimals").and_then(Value::as_u64))
        && !raw.contains('.')
    {
        amount = format_units(raw, decimals as u32);
    }
    let kind = match text(entry, &["type", "kind"]).as_deref() {
        Some(k) if k.contains("card") || k == "Payment" || k == "Refund" || k == "Reversal" => {
            "card".to_string()
        }
        Some(other) => other.chars().take(32).collect(),
        None if merchant.is_some() => "card".to_string(),
        None => "unknown".to_string(),
    };
    Some(Payment {
        id: text(entry, &["id", "threadId"])?,
        kind,
        created_at: text(entry, &["createdAt", "date"]),
        status: text(entry, &["status"]).unwrap_or_else(|| "unknown".into()),
        merchant,
        amount,
        currency: text(
            entry,
            &["billingCurrency", "currency", "transactionCurrency"],
        ),
    })
}

/// A card id is safe to put in a URL path.
pub fn valid_card_id(card_id: &str) -> bool {
    !card_id.is_empty()
        && card_id.len() <= 64
        && card_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl GnosisPay {
    pub async fn account(&self) -> ApiResult<Account> {
        let (account, balances, cards) = tokio::join!(
            self.get::<Value>("/user/account"),
            self.get::<Value>("/user/balances/v2"),
            self.get::<Value>("/cards"),
        );
        let account = account?;
        let balances = balances?;
        let cards = cards?;
        let currency = text(&account, &["nativeCurrency"]).unwrap_or_else(|| "EUR".into());
        let balance = balances["data"]
            .as_array()
            .and_then(|list| {
                list.iter()
                    .find(|b| b["currency"].as_str() == Some(currency.as_str()))
                    .or(list.first())
            })
            .cloned()
            .unwrap_or(Value::Null);
        let decimals = balance["decimals"].as_u64().unwrap_or(18) as u32;
        let amount = |key: &str| {
            text(&balance, &[key])
                .and_then(|raw| format_units(&raw, decimals))
                .unwrap_or_else(|| "0.00".into())
        };
        let pending_raw = ["processingDeposits", "nonSpendable"]
            .iter()
            .filter_map(|k| text(&balance, &[k]).and_then(|r| r.parse::<u128>().ok()))
            .sum::<u128>();
        Ok(Account {
            safe_address: text(&account, &["safeAddress"]),
            owner_address: text(&account, &["safeOwnerAddress"]),
            status: text(&account, &["status"]).unwrap_or_else(|| "unknown".into()),
            spendable: amount("spendable"),
            pending: format_units(&pending_raw.to_string(), decimals)
                .unwrap_or_else(|| "0.00".into()),
            currency,
            cards: cards["cards"]
                .as_array()
                .or(cards.as_array())
                .map(|list| list.iter().filter_map(summary).collect())
                .unwrap_or_default(),
        })
    }

    pub async fn statement(&self, cursor: Option<&str>) -> ApiResult<Statement> {
        let mut path = "/user/statement?limit=25".to_string();
        if let Some(cursor) = cursor {
            if !cursor
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_=.".contains(&b))
            {
                return Err(ApiError::Unreadable);
            }
            path.push_str("&cursor=");
            path.push_str(cursor);
        }
        let body = self.get::<Value>(&path).await?;
        Ok(Statement {
            items: body["data"]
                .as_array()
                .or(body["items"].as_array())
                .map(|list| list.iter().filter_map(payment).collect())
                .unwrap_or_default(),
            next_cursor: if body["hasNext"].as_bool() == Some(false) {
                None
            } else {
                text(&body, &["nextCursor"])
            },
        })
    }

    /// Freezes (blocks) or unfreezes the card. Cancelling for good is not offered.
    pub async fn set_frozen(&self, card_id: &str, frozen: bool) -> ApiResult<()> {
        if !valid_card_id(card_id) {
            return Err(ApiError::Unreadable);
        }
        let action = if frozen { "block" } else { "unblock" };
        self.post::<Value>(
            &format!("/cards/{card_id}/{action}"),
            &serde_json::json!({}),
        )
        .await
        .map(drop)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::fake::FakeGnosisPay;
    use crate::gnosis_pay::tests::{key, signed_in};
    use serde_json::json;

    #[test]
    fn amounts() {
        assert_eq!(
            format_units("123450000000000000000", 18).as_deref(),
            Some("123.45")
        );
        assert_eq!(format_units("1", 18).as_deref(), Some("0.00"));
        assert_eq!(format_units("0", 18).as_deref(), Some("0.00"));
        assert_eq!(format_units("1999", 2).as_deref(), Some("19.99"));
        assert_eq!(format_units("5", 1).as_deref(), Some("0.50"));
        assert_eq!(format_units("42", 0).as_deref(), Some("42"));
        assert_eq!(format_units("-5", 2), None);
        assert_eq!(format_units("1e5", 2), None);
    }

    #[test]
    fn a_payment_keeps_only_what_the_list_shows() {
        let entry = json!({
            "id": "t-1", "type": "card_transaction", "createdAt": "2026-10-23T18:04:00Z",
            "status": "settled", "billingAmount": "4.20", "billingCurrency": "EUR",
            "merchant": { "name": "Bakery", "city": "Somewhere" }, "mcc": "5814",
            "enrichment": { "merchant": { "name": "The Bakery" },
                            "shop": { "address": "1 Example Street", "coordinates": [1, 2] } },
        });
        let p = payment(&entry).unwrap();
        assert_eq!(p.merchant.as_deref(), Some("The Bakery"));
        assert_eq!(p.amount.as_deref(), Some("4.20"));
        assert_eq!(p.kind, "card");
        let shown = serde_json::to_string(&p).unwrap();
        assert!(
            !shown.contains("Example Street")
                && !shown.contains("Somewhere")
                && !shown.contains("5814")
        );
    }

    #[tokio::test]
    async fn the_account_of_an_onboarded_user() {
        let fake = FakeGnosisPay::start().await;
        fake.state().await.onboarded(key().address());
        let api = signed_in(&fake).await;

        let account = api.account().await.unwrap();
        assert_eq!(account.spendable, "123.45");
        assert_eq!(account.currency, "EUR");
        assert_eq!(account.owner_address, Some(key().address().checksummed()));
        assert_eq!(account.cards.len(), 1);
        assert_eq!(account.cards[0].last4.as_deref(), Some("4242"));
        assert_eq!(account.cards[0].status, "active");

        let statement = api.statement(None).await.unwrap();
        assert_eq!(statement.items.len(), 3);
        assert_eq!(
            statement.items[0].merchant.as_deref(),
            Some("Bakery Example")
        );
        assert_eq!(statement.next_cursor, None);
        assert!(api.statement(Some("a&b=c")).await.is_err());
    }

    #[tokio::test]
    async fn freezing_stops_payments_and_unfreezing_restarts_them() {
        let fake = FakeGnosisPay::start().await;
        fake.state().await.onboarded(key().address());
        let api = signed_in(&fake).await;
        let pay = || async {
            reqwest::Client::new()
                .post(format!("http://{}/fake/pay", fake.address))
                .json(&json!({ "merchant": "Shop", "amount": "1.00" }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()["status"]
                .clone()
        };

        api.set_frozen("card-1", true).await.unwrap();
        assert_eq!(api.account().await.unwrap().cards[0].status, "blocked");
        assert_eq!(pay().await, "declined");
        api.set_frozen("card-1", false).await.unwrap();
        assert_eq!(pay().await, "pending");
        assert_eq!(api.account().await.unwrap().spendable, "122.45");
        assert!(api.set_frozen("../user", true).await.is_err());
    }
}
