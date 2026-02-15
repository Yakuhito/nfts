use chia_wallet_sdk::{prelude::Bytes32, utils::Address};

use crate::{error::CliError, models::CoinType};

pub fn is_nft_launcher_id(input: &str) -> bool {
    input.starts_with("nft1")
}

pub fn is_did_launcher_id(input: &str) -> bool {
    input.starts_with("did:chia:1")
}

pub fn is_puzzle_hash_launcher_id(input: &str) -> bool {
    input.starts_with("xch1")
}

pub fn classify_prefixed_launcher_id(input: &str) -> Option<TrackedKind> {
    if is_nft_launcher_id(input) {
        return Some(TrackedKind::Nft);
    }
    if is_did_launcher_id(input) {
        return Some(TrackedKind::Did);
    }
    if is_puzzle_hash_launcher_id(input) {
        return Some(TrackedKind::PuzzleHash);
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackedKind {
    Nft,
    Did,
    PuzzleHash,
}

impl TrackedKind {
    pub fn to_coin_type(self) -> CoinType {
        match self {
            Self::Nft => CoinType::Nft,
            Self::Did => CoinType::Did,
            Self::PuzzleHash => CoinType::IntermediaryCoin,
        }
    }
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
