use chia_wallet_sdk::prelude::{Bytes, Bytes32};
use clvm_traits::{ClvmDecoder, ClvmEncoder, FromClvm, FromClvmError, Raw, ToClvm, ToClvmError};
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;
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
    pub metadata: Option<JsonValue>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ParsedMetadata(pub BTreeMap<String, MetadataValue>);

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetadataValue {
    Number(u64),
    Bytes32(Bytes32),
    String(String),
    StringArray(Vec<String>),
    Pairs(Vec<(Bytes, Bytes)>),
}

impl MetadataValue {
    fn from_clvm_raw<N: Clone, D: ClvmDecoder<Node = N>>(
        decoder: &D,
        node: N,
    ) -> Result<Self, FromClvmError> {
        if let Ok(value) = Bytes32::from_clvm(decoder, node.clone()) {
            Ok(Self::Bytes32(value))
        } else if let Ok(value) = u64::from_clvm(decoder, node.clone()) {
            Ok(Self::Number(value))
        } else if let Ok(value) = String::from_clvm(decoder, node.clone()) {
            Ok(Self::String(value))
        } else if let Ok(values) = Vec::<String>::from_clvm(decoder, node.clone()) {
            Ok(Self::StringArray(values))
        } else if let Ok(pairs) = Vec::<(Bytes, Bytes)>::from_clvm(decoder, node.clone()) {
            Ok(Self::Pairs(pairs))
        } else {
            Err(FromClvmError::Custom(
                "failed to deserialize metadata value".to_string(),
            ))
        }
    }
}

impl<N: Clone, D: ClvmDecoder<Node = N>> FromClvm<D> for MetadataValue {
    fn from_clvm(decoder: &D, node: N) -> Result<Self, FromClvmError> {
        Self::from_clvm_raw(decoder, node)
    }
}

impl<N, E: ClvmEncoder<Node = N>> ToClvm<E> for MetadataValue {
    fn to_clvm(&self, encoder: &mut E) -> Result<N, ToClvmError> {
        match self {
            Self::String(value) => value.to_clvm(encoder),
            Self::StringArray(values) => values.to_clvm(encoder),
            Self::Number(value) => value.to_clvm(encoder),
            Self::Bytes32(value) => value.to_clvm(encoder),
            Self::Pairs(pairs) => pairs.to_clvm(encoder),
        }
    }
}

impl<N: Clone, D: ClvmDecoder<Node = N>> FromClvm<D> for ParsedMetadata {
    fn from_clvm(decoder: &D, node: N) -> Result<Self, FromClvmError> {
        let items: Vec<(String, Raw<N>)> = FromClvm::from_clvm(decoder, node)?;
        let mut metadata = BTreeMap::new();

        for (key, Raw(ptr)) in items {
            let value = MetadataValue::from_clvm_raw(decoder, ptr)?;
            metadata.insert(key, value);
        }

        Ok(Self(metadata))
    }
}

impl<N, E: ClvmEncoder<Node = N>> ToClvm<E> for ParsedMetadata {
    fn to_clvm(&self, encoder: &mut E) -> Result<N, ToClvmError> {
        let mut items: Vec<(String, Raw<N>)> = Vec::with_capacity(self.0.len());
        for (key, value) in &self.0 {
            items.push((key.clone(), Raw(value.to_clvm(encoder)?)));
        }
        items.to_clvm(encoder)
    }
}

#[allow(dead_code)]
impl Coin {
    pub fn metadata_as<T: DeserializeOwned>(&self) -> Result<Option<T>, serde_json::Error> {
        self.metadata
            .as_ref()
            .map(|metadata| serde_json::from_value(metadata.clone()))
            .transpose()
    }

    pub fn parsed_metadata(&self) -> Result<Option<ParsedMetadata>, serde_json::Error> {
        self.metadata_as::<ParsedMetadata>()
    }
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
