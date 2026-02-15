use chia_wallet_sdk::{prelude::Bytes32, utils::Address};

use crate::error::CliError;

pub fn parse_launcher_id(input: &str) -> Result<Bytes32, CliError> {
    if input.starts_with("did:chia:") {
        // Accept full DID bech32m string (`did:chia:...`) first.
        if let Ok(value) = parse_bech32m_payload(input) {
            return Ok(value);
        }
        // Also accept `did:chia:` URI prefix followed by plain launcher id value.
        if let Some(stripped) = input.strip_prefix("did:chia:") {
            return parse_launcher_id(stripped);
        }
    }

    if input.starts_with("nft") {
        return parse_bech32m_payload(input);
    }

    if input.len() == 64 && input.chars().all(|c| c.is_ascii_hexdigit()) {
        let bytes = decode_hex_32(input).ok_or_else(|| {
            CliError::Message(format!(
                "invalid hex launcher id (expected 32 bytes): {input}"
            ))
        })?;
        return Ok(Bytes32::new(bytes));
    }

    parse_bech32m_payload(input)
}

pub fn is_nft_launcher_id(input: &str) -> bool {
    input.starts_with("nft")
}

pub fn is_did_launcher_id(input: &str) -> bool {
    input.starts_with("did:chia:")
}

pub fn classify_prefixed_launcher_id(input: &str) -> Option<TrackedKind> {
    if is_nft_launcher_id(input) {
        return Some(TrackedKind::Nft);
    }
    if is_did_launcher_id(input) {
        return Some(TrackedKind::Did);
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackedKind {
    Nft,
    Did,
}

pub fn parse_bech32m_payload(value: &str) -> Result<Bytes32, CliError> {
    Ok(Address::decode(value)?.puzzle_hash)
}

pub fn encode_nft_launcher_id(launcher_id: &Bytes32) -> Result<String, CliError> {
    Ok(Address::new(*launcher_id, "nft".to_string()).encode()?)
}

pub fn encode_did_launcher_id(launcher_id: &Bytes32) -> Result<String, CliError> {
    Ok(Address::new(*launcher_id, "did:chia:".to_string()).encode()?)
}

pub fn bytes32_from_db(field_name: &str, value: &[u8]) -> Result<Bytes32, CliError> {
    if value.len() != 32 {
        return Err(CliError::Message(format!(
            "{field_name} has invalid length {}, expected 32",
            value.len()
        )));
    }

    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(value);
    Ok(Bytes32::new(bytes))
}

pub fn optional_bytes32_from_db(
    field_name: &str,
    value: Option<&[u8]>,
) -> Result<Option<Bytes32>, CliError> {
    value
        .map(|bytes| bytes32_from_db(field_name, bytes))
        .transpose()
}

fn decode_hex_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }

    let mut out = [0u8; 32];
    for (idx, slot) in out.iter_mut().enumerate() {
        let start = idx * 2;
        let end = start + 2;
        let part = &hex[start..end];
        *slot = u8::from_str_radix(part, 16).ok()?;
    }
    Some(out)
}
