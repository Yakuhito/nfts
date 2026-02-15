use chia_wallet_sdk::prelude::Bytes32;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Did {
    pub launcher_id: Bytes32,
    pub parent_coin_id: Option<Bytes32>,
    pub coin_id: Option<Bytes32>,
    pub spent_height: Option<u32>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct IntermediaryCoin {
    pub parent_coin_id: Bytes32,
    pub coin_id: Bytes32,
    pub spent_height: Option<u32>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Nft {
    pub launcher_id: Bytes32,
    pub did_launcher_id: Option<Bytes32>,
    pub parent_coin_id: Option<Bytes32>,
    pub coin_id: Option<Bytes32>,
    pub inner_puzzle_hash: Option<Bytes32>,
    pub spent_height: Option<u32>,
}
