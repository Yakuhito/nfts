use chia_wallet_sdk::prelude::Bytes32;

#[derive(Debug, Clone)]
pub struct TrackedPuzzleHash {
    pub puzzle_hash: Bytes32,
    pub last_sync_height: u32,
}

#[derive(Debug, Clone)]
pub struct Coin {
    pub coin_type: CoinType,
    pub launcher_id: Option<Bytes32>,
    pub did_launcher_id: Option<Bytes32>,
    pub parent_coin_id: Bytes32,
    pub puzzle_hash: Bytes32,
    pub coin_id: Bytes32,
    pub created_height: u32,
    pub spent_height: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoinType {
    Nft = 1,
    Did = 2,
    Intermediary = 3,
}

impl CoinType {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Nft => "NFT",
            Self::Did => "DID",
            Self::Intermediary => "Intermediary Coin",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "NFT" => Some(Self::Nft),
            "DID" => Some(Self::Did),
            "Intermediary Coin" => Some(Self::Intermediary),
            _ => None,
        }
    }
}
