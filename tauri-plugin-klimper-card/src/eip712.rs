// SPDX-License-Identifier: AGPL-3.0-or-later
//! EIP-712 typed-data hashing, and the one kind of typed data the card key signs.
//!
//! Gnosis Pay asks for an EIP-712 signature before it hands over the card details
//! (`GET /pci/cards/{id}/challenge`). Typed data can mean much more than "show my card": a
//! Safe transaction, a token permit. So [`check_card_challenge`] accepts only a `Message`
//! with an authorization text and a nonce, on this build's chain, from a domain named
//! Gnosis Pay, with no contract and no salt. Nothing of that shape can move money.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;

use serde_json::Value;
use sha3::{Digest, Keccak256};

use crate::evm::Address;

#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum TypedDataError {
    #[error("the typed data is not well formed")]
    Malformed,
    #[error("the typed data asks for more than showing the card")]
    NotACardChallenge,
    #[error("the typed data is for another chain")]
    WrongChain,
}

type Fields = Vec<(String, String)>;

/// `{ types, primaryType, domain, message }`, as `eth_signTypedData_v4` takes it.
#[derive(Debug, Clone)]
pub struct TypedData {
    types: HashMap<String, Fields>,
    primary_type: String,
    domain: Value,
    message: Value,
}

fn keccak(data: &[u8]) -> [u8; 32] {
    Keccak256::digest(data).into()
}

impl TypedData {
    pub fn from_json(value: &Value) -> Result<Self, TypedDataError> {
        let mut types = HashMap::new();
        for (name, fields) in value["types"]
            .as_object()
            .ok_or(TypedDataError::Malformed)?
        {
            let fields = fields
                .as_array()
                .ok_or(TypedDataError::Malformed)?
                .iter()
                .map(|f| {
                    Some((
                        f["name"].as_str()?.to_string(),
                        f["type"].as_str()?.to_string(),
                    ))
                })
                .collect::<Option<Fields>>()
                .ok_or(TypedDataError::Malformed)?;
            types.insert(name.clone(), fields);
        }
        let domain = value["domain"].clone();
        if !domain.is_object() {
            return Err(TypedDataError::Malformed);
        }
        if !types.contains_key("EIP712Domain") {
            // Derived from the fields present, in the order EIP-712 gives.
            let canonical = [
                ("name", "string"),
                ("version", "string"),
                ("chainId", "uint256"),
                ("verifyingContract", "address"),
                ("salt", "bytes32"),
            ];
            let fields = canonical
                .iter()
                .filter(|(name, _)| domain.get(name).is_some_and(|v| !v.is_null()))
                .map(|(n, t)| ((*n).to_string(), (*t).to_string()))
                .collect();
            types.insert("EIP712Domain".into(), fields);
        }
        Ok(Self {
            types,
            primary_type: value["primaryType"]
                .as_str()
                .ok_or(TypedDataError::Malformed)?
                .to_string(),
            domain,
            message: value["message"].clone(),
        })
    }

    fn dependencies(&self, name: &str, found: &mut BTreeSet<String>) -> Result<(), TypedDataError> {
        let fields = self.types.get(name).ok_or(TypedDataError::Malformed)?;
        for (_, kind) in fields {
            if kind.ends_with(']') {
                return Err(TypedDataError::Malformed); // arrays: not needed, not accepted
            }
            if self.types.contains_key(kind) && found.insert(kind.clone()) {
                self.dependencies(kind, found)?;
            }
        }
        Ok(())
    }

    fn encode_type(&self, name: &str) -> Result<String, TypedDataError> {
        let mut deps = BTreeSet::new();
        self.dependencies(name, &mut deps)?;
        deps.remove(name);
        let mut out = String::new();
        for kind in std::iter::once(name.to_string()).chain(deps) {
            let fields = &self.types[&kind];
            let inner: Vec<String> = fields.iter().map(|(n, t)| format!("{t} {n}")).collect();
            let _ = write!(out, "{kind}({})", inner.join(","));
        }
        Ok(out)
    }

    fn hash_struct(&self, name: &str, value: &Value) -> Result<[u8; 32], TypedDataError> {
        let mut data = keccak(self.encode_type(name)?.as_bytes()).to_vec();
        for (field, kind) in self.types.get(name).ok_or(TypedDataError::Malformed)? {
            data.extend_from_slice(&self.encode_value(kind, &value[field.as_str()])?);
        }
        Ok(keccak(&data))
    }

    fn encode_value(&self, kind: &str, value: &Value) -> Result<[u8; 32], TypedDataError> {
        if self.types.contains_key(kind) {
            return self.hash_struct(kind, value);
        }
        let mut word = [0u8; 32];
        match kind {
            "string" => {
                return Ok(keccak(
                    value.as_str().ok_or(TypedDataError::Malformed)?.as_bytes(),
                ));
            }
            "bytes" => return Ok(keccak(&hex_bytes(value)?)),
            "address" => {
                let text = value.as_str().ok_or(TypedDataError::Malformed)?;
                let address =
                    Address::parse(&text.to_ascii_lowercase()).ok_or(TypedDataError::Malformed)?;
                word[12..].copy_from_slice(&address.0);
            }
            "bool" => word[31] = u8::from(value.as_bool().ok_or(TypedDataError::Malformed)?),
            _ if kind.starts_with("uint") => word = uint256(value)?,
            _ if kind.starts_with("bytes") => {
                let bytes = hex_bytes(value)?;
                if bytes.len() > 32 {
                    return Err(TypedDataError::Malformed);
                }
                word[..bytes.len()].copy_from_slice(&bytes);
            }
            _ => return Err(TypedDataError::Malformed),
        }
        Ok(word)
    }

    pub fn domain_separator(&self) -> Result<[u8; 32], TypedDataError> {
        self.hash_struct("EIP712Domain", &self.domain)
    }

    /// keccak256(0x19 0x01 ‖ domainSeparator ‖ hashStruct(message)): what is signed.
    pub fn digest(&self) -> Result<[u8; 32], TypedDataError> {
        let mut data = vec![0x19, 0x01];
        data.extend_from_slice(&self.domain_separator()?);
        data.extend_from_slice(&self.hash_struct(&self.primary_type, &self.message)?);
        Ok(keccak(&data))
    }
}

fn hex_bytes(value: &Value) -> Result<Vec<u8>, TypedDataError> {
    let text = value.as_str().ok_or(TypedDataError::Malformed)?;
    hex::decode(text.strip_prefix("0x").unwrap_or(text)).map_err(|_| TypedDataError::Malformed)
}

/// A JSON number, a decimal string or a 0x hex string as a big-endian 256-bit word.
pub fn uint256(value: &Value) -> Result<[u8; 32], TypedDataError> {
    let text = match value {
        Value::Number(n) => n.as_u64().ok_or(TypedDataError::Malformed)?.to_string(),
        Value::String(s) => s.clone(),
        _ => return Err(TypedDataError::Malformed),
    };
    let mut word = [0u8; 32];
    if let Some(hex) = text.strip_prefix("0x") {
        let bytes = hex::decode(if hex.len() % 2 == 1 {
            format!("0{hex}")
        } else {
            hex.to_string()
        })
        .map_err(|_| TypedDataError::Malformed)?;
        if bytes.len() > 32 {
            return Err(TypedDataError::Malformed);
        }
        word[32 - bytes.len()..].copy_from_slice(&bytes);
        return Ok(word);
    }
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(TypedDataError::Malformed);
    }
    for digit in text.bytes() {
        // word = word * 10 + digit, overflow refused.
        let mut carry = u32::from(digit - b'0');
        for byte in word.iter_mut().rev() {
            let value = u32::from(*byte) * 10 + carry;
            *byte = (value & 0xff) as u8;
            carry = value >> 8;
        }
        if carry != 0 {
            return Err(TypedDataError::Malformed);
        }
    }
    Ok(word)
}

/// A card-details challenge that may be signed, with what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardChallenge {
    pub digest: [u8; 32],
    /// The nonce, as Gnosis Pay wants it back in `x-eip712-nonce`.
    pub nonce: String,
    /// e.g. "Display VISA Card Details".
    pub authorization: String,
}

/// Accepts the typed data only as a Gnosis Pay card challenge on `chain_id`.
pub fn check_card_challenge(value: &Value, chain_id: u64) -> Result<CardChallenge, TypedDataError> {
    let data = TypedData::from_json(value)?;
    if data.primary_type != "Message" {
        return Err(TypedDataError::NotACardChallenge);
    }
    let message_fields: BTreeSet<(&str, &str)> = data
        .types
        .get("Message")
        .ok_or(TypedDataError::Malformed)?
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    if message_fields != BTreeSet::from([("authorization", "string"), ("nonce", "uint256")]) {
        return Err(TypedDataError::NotACardChallenge);
    }
    // Only the domain and the message: no other struct may hide in the types.
    if data
        .types
        .keys()
        .any(|k| k != "Message" && k != "EIP712Domain")
    {
        return Err(TypedDataError::NotACardChallenge);
    }
    let domain_fields: BTreeSet<&str> = data.types["EIP712Domain"]
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    if domain_fields.contains("verifyingContract") || domain_fields.contains("salt") {
        return Err(TypedDataError::NotACardChallenge);
    }
    let name = data.domain["name"].as_str().unwrap_or_default();
    if !name.to_ascii_lowercase().contains("gnosis pay") {
        return Err(TypedDataError::NotACardChallenge);
    }
    let chain = uint256(&data.domain["chainId"])?;
    if chain != uint256(&Value::from(chain_id))? {
        return Err(TypedDataError::WrongChain);
    }
    let authorization = data.message["authorization"]
        .as_str()
        .ok_or(TypedDataError::Malformed)?;
    if authorization.len() > 120 || authorization.chars().any(char::is_control) {
        return Err(TypedDataError::NotACardChallenge);
    }
    let nonce = match &data.message["nonce"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err(TypedDataError::Malformed),
    };
    Ok(CardChallenge {
        digest: data.digest()?,
        nonce,
        authorization: authorization.to_string(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::evm::{EvmKey, recover};
    use serde_json::json;

    /// The example from EIP-712 itself.
    fn mail() -> Value {
        json!({
            "types": {
                "EIP712Domain": [
                    {"name": "name", "type": "string"}, {"name": "version", "type": "string"},
                    {"name": "chainId", "type": "uint256"}, {"name": "verifyingContract", "type": "address"}
                ],
                "Person": [{"name": "name", "type": "string"}, {"name": "wallet", "type": "address"}],
                "Mail": [
                    {"name": "from", "type": "Person"}, {"name": "to", "type": "Person"},
                    {"name": "contents", "type": "string"}
                ]
            },
            "primaryType": "Mail",
            "domain": {
                "name": "Ether Mail", "version": "1", "chainId": 1,
                "verifyingContract": "0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"
            },
            "message": {
                "from": {"name": "Cow", "wallet": "0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"},
                "to": {"name": "Bob", "wallet": "0xbBbBBBBbbBBBbbbBbbBbbbbBBbBbbbbBbBbbBBbB"},
                "contents": "Hello, Bob!"
            }
        })
    }

    #[test]
    fn the_eip712_mail_example() {
        let data = TypedData::from_json(&mail()).unwrap();
        assert_eq!(
            data.encode_type("Mail").unwrap(),
            "Mail(Person from,Person to,string contents)Person(string name,address wallet)"
        );
        assert_eq!(
            hex::encode(data.domain_separator().unwrap()),
            "f2cee375fa42b42143804025fc449deafd50cc031ca257e0b194a650a912090f"
        );
        let digest = data.digest().unwrap();
        assert_eq!(
            hex::encode(digest),
            "be609aee343fb3c4b28e1df9e632fca64fcfaede20f02e86244efddf30957bd2"
        );

        // The example's signer is keccak256("cow"); its signature is in the EIP.
        let key = EvmKey::from_secret(&keccak(b"cow")).unwrap();
        assert_eq!(
            key.address().checksummed(),
            "0xCD2a3d9F938E13CD947Ec05AbC7FE734Df8DD826"
        );
        let signature = key.sign_digest(&digest).unwrap();
        assert_eq!(
            hex::encode(&signature[..64]),
            "4355c47d63924e8a72e509b65029052eb6c299d53a04e167c5775fd466751c9d\
             07299936d304c153f6443dfa05f40ff007d72911b6f72307f996231605b91562"
        );
        assert_eq!(signature[64], 28);
        assert_eq!(recover(&digest, &signature), Some(key.address()));
    }

    #[test]
    fn numbers() {
        assert_eq!(uint256(&json!(1)).unwrap()[31], 1);
        assert_eq!(uint256(&json!("256")).unwrap()[30..], [1, 0]);
        assert_eq!(uint256(&json!("0x0100")).unwrap()[30..], [1, 0]);
        let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        assert_eq!(uint256(&json!(max)).unwrap(), [0xff; 32]);
        let over = "115792089237316195423570985008687907853269984665640564039457584007913129639936";
        assert!(uint256(&json!(over)).is_err());
        assert!(uint256(&json!("-1")).is_err());
    }

    pub(crate) fn challenge(chain: u64) -> Value {
        json!({
            "domain": {"name": "Gnosis Pay", "version": "1", "chainId": chain},
            "primaryType": "Message",
            "types": {"Message": [{"name": "authorization", "type": "string"}, {"name": "nonce", "type": "uint256"}]},
            "message": {"authorization": "Display VISA Card Details", "nonce": "123456789012345678901234567890"}
        })
    }

    #[test]
    fn a_card_challenge_is_accepted() {
        let checked = check_card_challenge(&challenge(100), 100).unwrap();
        assert_eq!(checked.authorization, "Display VISA Card Details");
        assert_eq!(checked.nonce, "123456789012345678901234567890");
    }

    #[test]
    fn anything_that_could_move_money_is_refused() {
        assert_eq!(
            check_card_challenge(&mail(), 100),
            Err(TypedDataError::NotACardChallenge)
        );
        assert_eq!(
            check_card_challenge(&challenge(1), 100),
            Err(TypedDataError::WrongChain)
        );

        let mut with_contract = challenge(100);
        with_contract["domain"]["verifyingContract"] =
            json!("0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC");
        assert_eq!(
            check_card_challenge(&with_contract, 100),
            Err(TypedDataError::NotACardChallenge)
        );

        let mut extra_field = challenge(100);
        extra_field["types"]["Message"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "to", "type": "address"}));
        assert_eq!(
            check_card_challenge(&extra_field, 100),
            Err(TypedDataError::NotACardChallenge)
        );

        let mut other_name = challenge(100);
        other_name["domain"]["name"] = json!("Permit2");
        assert_eq!(
            check_card_challenge(&other_name, 100),
            Err(TypedDataError::NotACardChallenge)
        );

        let mut safe_tx = challenge(100);
        safe_tx["primaryType"] = json!("SafeTx");
        assert_eq!(
            check_card_challenge(&safe_tx, 100),
            Err(TypedDataError::NotACardChallenge)
        );
    }
}
