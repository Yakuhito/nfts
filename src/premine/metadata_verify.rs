//! Hash-verify off-chain metadata bytes against an on-chain metadata hash.

use sha2::{Digest, Sha256};
use serde_json::{Map, Value};

/// If `bytes` are UTF-8 JSON whose SHA-256 equals `expected_hash`, return the text.
pub fn accept_metadata_bytes(expected_hash: &[u8; 32], bytes: &[u8]) -> Option<String> {
    let digest = Sha256::digest(bytes);
    if digest.as_slice() != expected_hash {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    if serde_json::from_str::<Value>(text).is_err() {
        return None;
    }
    Some(text.to_string())
}

/// CNS / Pawket (`Pawket/0.1`) serializes CHIP-0007 metadata as indent-2 JSON with CRLF
/// and a fixed key order. MintGarden's `metadata_json` is the same object with shuffled
/// keys; re-emitting in Pawket's wire format recovers bytes that match on-chain `mh`.
pub fn accept_pawket_reconstructed(
    expected_hash: &[u8; 32],
    metadata_json: &Value,
) -> Option<String> {
    let bytes = serialize_pawket_cns(metadata_json)?;
    accept_metadata_bytes(expected_hash, &bytes)
}

fn serialize_pawket_cns(metadata_json: &Value) -> Option<Vec<u8>> {
    let reordered = reorder_pawket(metadata_json)?;
    let pretty = serde_json::to_string_pretty(&reordered).ok()?;
    Some(pretty.replace('\n', "\r\n").into_bytes())
}

const TOP_KEYS: &[&str] = &[
    "format",
    "name",
    "description",
    "minting_tool",
    "sensitive_content",
    "series_number",
    "series_total",
    "attributes",
    "collection",
];
const ATTR_KEYS: &[&str] = &["trait_type", "value"];
const COLLECTION_KEYS: &[&str] = &["name", "id", "attributes"];
const COLLECTION_ATTR_KEYS: &[&str] = &["type", "value"];

fn reorder_pawket(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    let mut out = Map::new();
    for key in TOP_KEYS {
        let Some(child) = obj.get(*key) else {
            continue;
        };
        let ordered = match *key {
            "attributes" => reorder_object_array(child, ATTR_KEYS)?,
            "collection" => reorder_collection(child)?,
            _ => child.clone(),
        };
        out.insert((*key).to_string(), ordered);
    }
    for (key, child) in obj {
        if !out.contains_key(key) {
            out.insert(key.clone(), child.clone());
        }
    }
    Some(Value::Object(out))
}

fn reorder_collection(value: &Value) -> Option<Value> {
    let obj = value.as_object()?;
    let mut out = Map::new();
    for key in COLLECTION_KEYS {
        let Some(child) = obj.get(*key) else {
            continue;
        };
        let ordered = if *key == "attributes" {
            reorder_object_array(child, COLLECTION_ATTR_KEYS)?
        } else {
            child.clone()
        };
        out.insert((*key).to_string(), ordered);
    }
    for (key, child) in obj {
        if !out.contains_key(key) {
            out.insert(key.clone(), child.clone());
        }
    }
    Some(Value::Object(out))
}

fn reorder_object_array(value: &Value, key_order: &[&str]) -> Option<Value> {
    let arr = value.as_array()?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        out.push(reorder_object_keys(item, key_order)?);
    }
    Some(Value::Array(out))
}

fn reorder_object_keys(value: &Value, key_order: &[&str]) -> Option<Value> {
    let obj = value.as_object()?;
    let mut out = Map::new();
    for key in key_order {
        if let Some(child) = obj.get(*key) {
            out.insert((*key).to_string(), child.clone());
        }
    }
    for (key, child) in obj {
        if !out.contains_key(key) {
            out.insert(key.clone(), child.clone());
        }
    }
    Some(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_matching_json_bytes() {
        let body = br#"{"format":"CHIP-0007","name":"alice.xch"}"#;
        let digest = Sha256::digest(body);
        let mut expected = [0u8; 32];
        expected.copy_from_slice(&digest);
        let got = accept_metadata_bytes(&expected, body).unwrap();
        assert_eq!(got, std::str::from_utf8(body).unwrap());
    }

    #[test]
    fn rejects_hash_mismatch() {
        let body = br#"{"format":"CHIP-0007","name":"alice.xch"}"#;
        let expected = [0u8; 32];
        assert!(accept_metadata_bytes(&expected, body).is_none());
    }

    #[test]
    fn rejects_non_json_even_if_hash_matches() {
        let body = b"not-json";
        let digest = Sha256::digest(body);
        let mut expected = [0u8; 32];
        expected.copy_from_slice(&digest);
        assert!(accept_metadata_bytes(&expected, body).is_none());
    }

    #[test]
    fn reconstructs_pawket_cns_from_mintgarden_metadata_json() {
        let metadata_json: Value = serde_json::from_str(include_str!(
            "testdata/domains_xch_metadata_json.json"
        ))
        .unwrap();
        let expected = hex_hash("b80c827330b7aae124f274be35ec1eeca1cb44b57c4b88eadeb4a6d593cc0d33");
        let text = accept_pawket_reconstructed(&expected, &metadata_json).unwrap();
        assert!(text.contains("\r\n"));
        assert!(text.starts_with("{\r\n  \"format\": \"CHIP-0007\""));
        assert_eq!(
            hex::encode(Sha256::digest(text.as_bytes())),
            "b80c827330b7aae124f274be35ec1eeca1cb44b57c4b88eadeb4a6d593cc0d33"
        );
    }

    #[test]
    fn pawket_reconstruct_rejects_wrong_hash() {
        let metadata_json: Value = serde_json::from_str(include_str!(
            "testdata/domains_xch_metadata_json.json"
        ))
        .unwrap();
        assert!(accept_pawket_reconstructed(&[0u8; 32], &metadata_json).is_none());
    }

    fn hex_hash(hex_str: &str) -> [u8; 32] {
        let bytes = hex::decode(hex_str).unwrap();
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        out
    }
}
