// SPDX-License-Identifier: AGPL-3.0-or-later
//! Money on the wire: every mojo amount crosses the Tauri bridge as a **decimal string**.
//!
//! A Tauri command's return value is serialised to JSON and parsed by the webview with
//! `JSON.parse`, which produces IEEE-754 doubles. Mojo amounts do not fit: XCH has twelve
//! decimal places, so the exact integers stop at 2^53 mojos — about 9007 XCH — and
//! everything above that is silently rounded to the nearest representable double. Measured
//! on the old encoding:
//!
//! | Amount | Sent as a JSON number | Read back in the webview |
//! |---|---|---|
//! | 12345.678901234567 XCH | `12345678901234567` | `12345678901234568` (+1 mojo) |
//! | 10000 XCH + 1 mojo | `10000000000000001` | `10000000000000000` (−1 mojo) |
//!
//! A wallet that shows an amount one mojo away from what it is about to sign is lying to
//! the user, and a per-day limit that is stored one mojo away from what was typed is a
//! limit nobody set. So nothing numeric carries money across this boundary:
//!
//! * **outbound limits** — [`crate::store::StoredLimit`] serialises `per_request` and
//!   `per_day` through [`opt_decimal`];
//! * **outbound prompts** — wallet-kit's `Prompt` is a read-only dependency whose
//!   `Serialize` writes JSON numbers, so [`decimalise_prompt`] rewrites every money field
//!   of the serialised value before it reaches the approval bridge;
//! * **inbound limits** — the same [`opt_decimal`] accepts a decimal string *and* a JSON
//!   number, so a `set_dapp_policy` from an older webview build, and a session record
//!   already written to the keyring in the numeric form, still load.
//!
//! What is deliberately *not* converted: CLVM costs (capped at the chain's per-block limit,
//! 1.1 × 10^10), coin counts, condition opcodes, spend indexes, and the `time_locks`
//! heights and unix seconds. None of them is money and all of them are far below 2^53.
//!
//! The webview side is `src/klimper/xchonnectPrompt.ts` (`amountOf`) and
//! `src/klimper/XchonnectDappSettings.tsx` (`parseLimit`), both of which read and write
//! decimal strings through `BigNumber`.

use serde_json::{Map, Value};

/// `Option<u128>` as an optional decimal string.
///
/// Serialising always writes a string. Deserialising accepts a string, a JSON number or
/// `null`, because a session record written by an earlier build is still in the keyring and
/// must keep loading — the reader is the compatibility surface, the writer is not.
pub mod opt_decimal {
    use serde::de::{self, Deserializer, Unexpected, Visitor};
    use serde::{Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<u128>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.map(|amount| amount.to_string()).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u128>, D::Error> {
        deserializer.deserialize_option(OptAmount)
    }

    struct OptAmount;

    impl<'de> Visitor<'de> for OptAmount {
        type Value = Option<u128>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an amount in the smallest unit, as a decimal string")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            d.deserialize_any(Amount).map(Some)
        }
    }

    struct Amount;

    impl Visitor<'_> for Amount {
        type Value = u128;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an amount in the smallest unit, as a decimal string")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<u128, E> {
            value
                .parse()
                .map_err(|_| E::invalid_value(Unexpected::Str(value), &self))
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<u128, E> {
            Ok(u128::from(value))
        }

        fn visit_u128<E: de::Error>(self, value: u128) -> Result<u128, E> {
            Ok(value)
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<u128, E> {
            u128::try_from(value).map_err(|_| E::invalid_value(Unexpected::Signed(value), &self))
        }

        fn visit_i128<E: de::Error>(self, value: i128) -> Result<u128, E> {
            u128::try_from(value).map_err(|_| E::custom("a limit cannot be negative"))
        }
    }
}

/// Every money field of a serialised `Prompt`, by the shape wallet-kit writes.
///
/// Addressed explicitly rather than by sweeping the tree for likely key names: a blind
/// sweep would convert the next field that happens to be called `amount` whether or not it
/// is money, and would silently stop converting a renamed one. The shape is asserted by
/// `the_money_fields_of_a_prompt_are_the_ones_this_module_knows`, which fails if wallet-kit
/// grows or renames a field here.
const SUMMARY_AMOUNTS: &[&str] = &["implied_fee", "reserve_fee"];
const ASSET_AMOUNTS: &[&str] = &["sent", "received", "conditional_received", "net"];

/// Rewrite every mojo amount in a serialised wallet-kit `Prompt` as a decimal string.
///
/// `null` stays `null` (`implied_fee` is `None` when a request relies on coins it does not
/// include), and a value that is already a string is left alone, so calling this twice is
/// harmless.
pub fn decimalise_prompt(prompt: &mut Value) {
    let Some(prompt) = prompt.as_object_mut() else {
        return;
    };
    if let Some(summary) = prompt.get_mut("summary").and_then(Value::as_object_mut) {
        for key in SUMMARY_AMOUNTS {
            stringify(summary, key);
        }
        for asset in array_of(summary, "assets") {
            for key in ASSET_AMOUNTS {
                stringify(asset, key);
            }
        }
        // `spends[].amount` is the value of each spent coin. Nothing renders it today, but
        // it is money and it is on the wire.
        for spend in array_of(summary, "spends") {
            stringify(spend, "amount");
        }
    }
    if let Some(binding) = prompt.get_mut("binding").and_then(Value::as_object_mut) {
        for list in ["bound_payments", "bound_received"] {
            for entry in array_of(binding, list) {
                stringify(entry, "amount");
            }
        }
    }
}

/// The objects of `object[key]`, when it is an array of objects.
fn array_of<'a>(object: &'a mut Map<String, Value>, key: &str) -> Vec<&'a mut Map<String, Value>> {
    object
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .map(|items| items.iter_mut().filter_map(Value::as_object_mut).collect())
        .unwrap_or_default()
}

/// Replace one numeric field with its exact decimal string. `serde_json::Number` keeps the
/// full integer, so this is the last point at which the value is still exact.
fn stringify(object: &mut Map<String, Value>, key: &str) {
    let Some(value) = object.get_mut(key) else {
        return;
    };
    let text = match value {
        Value::Number(number) => number.to_string(),
        _ => return,
    };
    *value = Value::String(text);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;
    use xchonnect_wallet_kit::{
        AssetAmount, AssetDelta, AssetId, BindingReport, BoundPayment, Prompt, SigningPlan,
        SpendInfo, SpendKind, Summary, TimeLocks,
    };

    /// Above 2^53 and not a round power of ten: 10^16 happens to be exactly representable
    /// as a double and would show no drift at all.
    const BIG: u128 = 12_345_678_901_234_567;
    /// One mojo over 10^16, which a double rounds down.
    const BIG_PLUS_ONE: u64 = 10_000_000_000_000_001;

    fn summary() -> Summary {
        Summary {
            assets: vec![AssetDelta {
                asset: AssetId::Xch,
                sent: BIG,
                received: 1,
                conditional_received: BIG + 2,
                net: 1 - i128::try_from(BIG).unwrap(),
            }],
            implied_fee: Some(BIG_PLUS_ONE),
            reserve_fee: BIG_PLUS_ONE,
            owned_reserve_fee: BIG_PLUS_ONE,
            time_locks: TimeLocks::default(),
            unknown_puzzles: Vec::new(),
            cost: 1_000_000,
            spends: vec![SpendInfo {
                coin_id: "ab".to_owned(),
                puzzle_hash: "cd".to_owned(),
                amount: BIG_PLUS_ONE,
                kind: SpendKind::Standard,
                owned: true,
                agg_sigs: Vec::new(),
                cost: 1_000_000,
            }],
        }
    }

    fn binding() -> BindingReport {
        BindingReport {
            all_bound: true,
            unbound_user_spends: Vec::new(),
            bound_payments: vec![BoundPayment {
                user_spend: 0,
                settlement_spend: 1,
                asset: AssetId::Xch,
                amount: BIG,
            }],
            bound_received: vec![AssetAmount {
                asset: AssetId::Xch,
                amount: BIG,
            }],
        }
    }

    fn prompt_json() -> Value {
        let summary = summary();
        let plan = SigningPlan {
            ours: Vec::new(),
            foreign: Vec::new(),
            has_unsafe: false,
        };
        let binding = binding();
        let prompt = Prompt::SignCoinSpends {
            dapp: "pengui.xyz",
            summary: &summary,
            plan: &plan,
            partial: true,
            binding: Some(&binding),
        };
        serde_json::to_value(&prompt).unwrap()
    }

    /// The regression this module exists for: an amount above 2^53 survives the trip the
    /// webview makes, which it cannot do as a JSON number.
    #[test]
    fn an_amount_above_two_to_the_fifty_third_survives_the_bridge() {
        let mut value = prompt_json();
        decimalise_prompt(&mut value);
        let text = serde_json::to_string(&value).unwrap();

        // What the old encoding did, for the same value, through a double — which is what
        // `JSON.parse` produces.
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let through_a_double = (BIG as f64) as u128;
        assert_ne!(through_a_double, BIG, "the drift this test is about");

        let asset = &value["summary"]["assets"][0];
        assert_eq!(asset["sent"], json!(BIG.to_string()));
        assert_eq!(asset["conditional_received"], json!((BIG + 2).to_string()));
        assert_eq!(asset["net"], json!("-12345678901234566"));
        assert_eq!(value["summary"]["implied_fee"], json!("10000000000000001"));
        assert_eq!(value["summary"]["reserve_fee"], json!("10000000000000001"));
        assert_eq!(
            value["summary"]["spends"][0]["amount"],
            json!("10000000000000001")
        );
        assert_eq!(value["binding"]["bound_payments"][0]["amount"], json!(BIG.to_string()));
        assert_eq!(value["binding"]["bound_received"][0]["amount"], json!(BIG.to_string()));

        // And the exact digits are in the bytes that leave the process.
        assert!(text.contains("\"12345678901234567\""), "{text}");
        assert!(
            !text.contains("12345678901234568"),
            "no rounded value anywhere: {text}"
        );
    }

    /// `implied_fee: None` is "outputs exceed inputs", which the screen shows differently
    /// from a zero fee. It must stay `null` rather than become `"0"` or `"null"`.
    #[test]
    fn an_absent_fee_stays_absent() {
        let mut summary = summary();
        summary.implied_fee = None;
        let plan = SigningPlan {
            ours: Vec::new(),
            foreign: Vec::new(),
            has_unsafe: false,
        };
        let mut value = serde_json::to_value(Prompt::SignCoinSpends {
            dapp: "pengui.xyz",
            summary: &summary,
            plan: &plan,
            partial: false,
            binding: None,
        })
        .unwrap();
        decimalise_prompt(&mut value);
        assert_eq!(value["summary"]["implied_fee"], Value::Null);
        assert_eq!(value["binding"], Value::Null);
    }

    /// A `sign_message` prompt has no money in it and must come through untouched.
    #[test]
    fn a_message_prompt_is_unchanged() {
        let before = serde_json::to_value(Prompt::SignMessage {
            dapp: "pengui.xyz",
            message_hex: "00ff".to_owned(),
            message_text: None,
            public_key: "0xab".to_owned(),
        })
        .unwrap();
        let mut after = before.clone();
        decimalise_prompt(&mut after);
        assert_eq!(before, after);
    }

    /// Running the conversion twice changes nothing.
    #[test]
    fn the_conversion_is_idempotent() {
        let mut once = prompt_json();
        decimalise_prompt(&mut once);
        let mut twice = once.clone();
        decimalise_prompt(&mut twice);
        assert_eq!(once, twice);
    }

    /// The drift guard.
    ///
    /// [`decimalise_prompt`] converts fields it names, so a money field wallet-kit adds or
    /// renames would pass through as a number and start rounding again without anything
    /// failing. This pins the shape instead: the keys of every object that carries an
    /// amount are compared with what this module was written against, so the next
    /// wallet-kit change that touches them fails here and is decided by a person.
    #[test]
    fn the_money_fields_of_a_prompt_are_the_ones_this_module_knows() {
        let value = prompt_json();
        let keys = |value: &Value| -> Vec<String> {
            let mut names: Vec<String> = value
                .as_object()
                .expect("an object")
                .keys()
                .cloned()
                .collect();
            names.sort();
            names
        };

        assert_eq!(
            keys(&value),
            ["binding", "dapp", "partial", "plan", "summary", "type"]
        );
        assert_eq!(
            keys(&value["summary"]),
            [
                "assets",
                "cost",
                "implied_fee",
                "reserve_fee",
                "spends",
                "time_locks",
                "unknown_puzzles"
            ]
        );
        assert_eq!(
            keys(&value["summary"]["assets"][0]),
            ["asset", "conditional_received", "net", "received", "sent"]
        );
        assert_eq!(
            keys(&value["summary"]["spends"][0]),
            [
                "agg_sigs",
                "amount",
                "coin_id",
                "cost",
                "kind",
                "owned",
                "puzzle_hash"
            ]
        );
        assert_eq!(
            keys(&value["binding"]),
            [
                "all_bound",
                "bound_payments",
                "bound_received",
                "unbound_user_spends"
            ]
        );
        assert_eq!(
            keys(&value["binding"]["bound_payments"][0]),
            ["amount", "asset", "settlement_spend", "user_spend"]
        );
        assert_eq!(
            keys(&value["binding"]["bound_received"][0]),
            ["amount", "asset"]
        );

        // Nothing that is left as a number may be able to exceed 2^53. `cost` is capped at
        // the chain's per-block limit and the rest are indexes, opcodes, heights and unix
        // seconds.
        let mut converted = value;
        decimalise_prompt(&mut converted);
        let mut large = Vec::new();
        collect_large_numbers(&converted, String::new(), &mut large);
        assert!(large.is_empty(), "numbers above 2^53 remain: {large:?}");
    }

    fn collect_large_numbers(value: &Value, path: String, out: &mut Vec<String>) {
        const SAFE: u64 = 1 << 53;
        match value {
            Value::Number(number) => {
                let magnitude = number
                    .as_u64()
                    .or_else(|| number.as_i64().map(u64::try_from).and_then(Result::ok));
                if magnitude.is_some_and(|n| n > SAFE) {
                    out.push(format!("{path} = {number}"));
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    collect_large_numbers(item, format!("{path}[{index}]"), out);
                }
            }
            Value::Object(fields) => {
                for (key, field) in fields {
                    collect_large_numbers(field, format!("{path}.{key}"), out);
                }
            }
            _ => {}
        }
    }
}
