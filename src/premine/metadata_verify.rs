//! Hash-verify off-chain metadata bytes against an on-chain metadata hash.

use sha2::{Digest, Sha256};

/// If `bytes` are UTF-8 JSON whose SHA-256 equals `expected_hash`, return the text.
pub fn accept_metadata_bytes(expected_hash: &[u8; 32], bytes: &[u8]) -> Option<String> {
    let digest = Sha256::digest(bytes);
    if digest.as_slice() != expected_hash {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    if serde_json::from_str::<serde_json::Value>(text).is_err() {
        return None;
    }
    Some(text.to_string())
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
}
