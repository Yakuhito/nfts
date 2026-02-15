use chia_wallet_sdk::prelude::Bytes32;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct TrackedPuzzleHash {
    pub puzzle_hash: Bytes32,
    pub last_sync_height: u32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Coin {
    pub coin_type: CoinType,
    pub launcher_id: Option<Bytes32>,
    pub did_launcher_id: Option<Bytes32>,
    pub parent_coin_id: Bytes32,
    pub puzzle_hash: Bytes32,
    pub coin_id: Bytes32,
    pub created_height: u32,
    pub spent_height: Option<u32>,
    pub metadata: Option<Metadata>,
}

pub type Metadata = BTreeMap<String, MetadataValue>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetadataValue {
    String(String),
    StringArray(Vec<String>),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct OffchainMetadata {
    pub metadata_hash: Bytes32,
    pub urls: Vec<String>,
    pub value: Option<String>,
    pub next_retry: Option<u64>,
    pub retries_so_far: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoinType {
    Nft = 1,
    Did = 2,
    IntermediaryCoin = 3,
}

impl CoinType {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Nft => "NFT",
            Self::Did => "DID",
            Self::IntermediaryCoin => "ITR",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "NFT" => Some(Self::Nft),
            "DID" => Some(Self::Did),
            "ITR" => Some(Self::IntermediaryCoin),
            _ => None,
        }
    }
}
