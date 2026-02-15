use std::collections::HashSet;

use chia_wallet_sdk::driver::{Nft, Puzzle, SpendContext};
use chia_wallet_sdk::prelude::{Bytes32, ChiaRpcClient, Coin, CoinRecord, CoinsetClient};
use chia_wallet_sdk::puzzles::SINGLETON_LAUNCHER_HASH;
use chia_wallet_sdk::types::{Condition, Conditions, announcement_id};
use sqlx::SqlitePool;

use crate::cli::SyncArgs;
use crate::db;
use crate::error::CliError;
use crate::models::{Coin as DbCoin, CoinType, TrackedPuzzleHash};
use crate::utils::encode_nft_launcher_id;

pub async fn run(pool: &SqlitePool, args: SyncArgs) -> Result<(), CliError> {
    let client = CoinsetClient::mainnet();
    let puzzle_hashes = fetch_tracked_puzzle_hashes(pool).await?;
    let puzzle_hashes_to_follow: Vec<Bytes32> =
        puzzle_hashes.iter().map(|p| p.puzzle_hash).collect();

    let Some(status) = client.get_blockchain_state().await?.blockchain_state else {
        return Err(CliError::Message(
            "failed to get blockchain state".to_string(),
        ));
    };
    let peak_height = status.peak.height;
    println!("Peak height: {peak_height}");

    if !args.skip_puzzle_hash_sync && !puzzle_hashes.is_empty() {
        println!("Checking for new coins for tracked addresses...");

        let start_height = puzzle_hashes
            .last()
            .map(|p| {
                if p.last_sync_height == 0 {
                    u32::MAX
                } else {
                    p.last_sync_height
                }
            })
            .unwrap_or(u32::MAX);
        let (start_height, end_height) = if start_height == u32::MAX {
            (None, None)
        } else {
            // Start one block earlier just in case the last block is not inclusive
            (Some(start_height - 1), Some(peak_height))
        };

        let Some(coin_records) = client
            .get_coin_records_by_puzzle_hashes(
                puzzle_hashes_to_follow.clone(),
                start_height,
                end_height,
                Some(true),
            )
            .await?
            .coin_records
        else {
            return Err(CliError::Message(
                "failed to get coin records for tracked addresses".to_string(),
            ));
        };

        if !coin_records.is_empty() {
            println!(
                "Found {} coin records; adding to db (this may take a while)...",
                coin_records.len()
            );
        } else {
            println!("No new coins created at tracked addresses found.");
        }
        for coin_record in coin_records {
            let (coin_type, launcher_id) =
                if coin_record.coin.puzzle_hash != SINGLETON_LAUNCHER_HASH.into() {
                    (CoinType::IntermediaryCoin, None)
                } else {
                    // singleton launcher!
                    (CoinType::Nft, Some(coin_record.coin.coin_id()))
                };
            db::add_coin_to_db(
                pool,
                coin_type,
                launcher_id,
                None,
                &CoinRecord {
                    coin: coin_record.coin,
                    confirmed_block_index: coin_record.confirmed_block_index,
                    spent_block_index: 0,
                    spent: false,
                    coinbase: coin_record.coinbase,
                    timestamp: coin_record.timestamp,
                },
            )
            .await?;
        }

        for puzzle_hash in puzzle_hashes {
            db::update_tracked_puzzle_hash_sync_height(
                pool,
                &puzzle_hash.puzzle_hash,
                peak_height - 32,
            )
            .await?;
        }
    }

    let mut processed_coin_ids: HashSet<Bytes32> = HashSet::new();
    loop {
        println!("New sync loop: fetching unspent coins...");
        let unspent_coins = fetch_unspent_coins(pool)
            .await?
            .into_iter()
            .filter(|c| !processed_coin_ids.contains(&c.coin_id))
            .collect::<Vec<_>>();

        let mut spent_coin_records: Vec<(CoinRecord, &DbCoin)> = Vec::new();
        println!(
            "Fetching a total of {} unspent coin records...",
            unspent_coins.len()
        );

        for (batch_no, coin_data) in unspent_coins.chunks(args.batch_size).enumerate() {
            let coin_ids = coin_data.iter().map(|c| c.coin_id).collect::<Vec<_>>();
            println!(
                "Fetching {} coin records for batch #{batch_no}...",
                coin_ids.len()
            );
            let Some(coin_records) = client
                .get_coin_records_by_names(coin_ids.to_vec(), None, None, Some(true))
                .await?
                .coin_records
            else {
                return Err(CliError::Message(
                    "failed to get coin records for batch #{batch_no}".to_string(),
                ));
            };

            coin_records.iter().for_each(|c| {
                processed_coin_ids.insert(c.coin.coin_id());
            });
            let zipped = coin_records.into_iter().zip(coin_data);
            spent_coin_records.extend(zipped.filter(|c| c.0.spent));
        }

        if spent_coin_records.is_empty() {
            println!("No spent coin records found. Sync complete!");
            break;
        }

        println!(
            "Fetching solutions for and processing {} spent coin records...",
            spent_coin_records.len()
        );

        if spent_coin_records.len() == 1 {
            println!(
                "Coin being processed: {}",
                hex::encode(spent_coin_records[0].0.coin.coin_id())
            );
        }
        for (i, (coin_record, coin_data)) in spent_coin_records.iter().enumerate() {
            if i % 100 == 0 && i > 0 {
                println!("Processed {} spent coin records...", i);
            }

            let Some(coin_spend) = client
                .get_puzzle_and_solution(
                    coin_record.coin.coin_id(),
                    Some(coin_record.spent_block_index),
                )
                .await?
                .coin_solution
            else {
                return Err(CliError::Message(
                    "failed to get puzzle and solution for coin record".to_string(),
                ));
            };

            let ctx = &mut SpendContext::new();
            let puzzle = ctx.alloc(&coin_spend.puzzle_reveal)?;
            let solution = ctx.alloc(&coin_spend.solution)?;

            match coin_data.coin_type {
                CoinType::Nft => {
                    // TODO: parse NFTs
                }
                CoinType::Did => {
                    let res = ctx.run(puzzle, solution)?;
                    let output_conds = ctx.extract::<Conditions>(res)?;

                    for cond in output_conds.iter() {
                        if let Condition::CreateCoin(cc) = cond {
                            let new_coin = Coin::new(coin_data.coin_id, cc.puzzle_hash, cc.amount);
                            if cc.amount % 2 == 1 {
                                // Even amount -> this is the new singleton / DID

                                db::add_coin_to_db(
                                    pool,
                                    CoinType::Did,
                                    coin_data.launcher_id,
                                    coin_data.did_launcher_id,
                                    &CoinRecord {
                                        coin: new_coin,
                                        confirmed_block_index: coin_record.spent_block_index,
                                        spent_block_index: 0,
                                        spent: false,
                                        coinbase: false,
                                        timestamp: coin_record.timestamp,
                                    },
                                )
                                .await?;

                                continue;
                            }

                            // this is an even amount coin -> launcher or intermediary coin
                            let (coin_type, launcher_id) =
                                if new_coin.puzzle_hash != SINGLETON_LAUNCHER_HASH.into() {
                                    (CoinType::IntermediaryCoin, None)
                                } else {
                                    // singleton launcher!
                                    (CoinType::Nft, Some(new_coin.coin_id()))
                                };
                            db::add_coin_to_db(
                                pool,
                                coin_type,
                                launcher_id,
                                coin_data.launcher_id, // DID launcher id is the DID's launcher id property :)
                                &CoinRecord {
                                    coin: new_coin,
                                    confirmed_block_index: coin_record.spent_block_index,
                                    spent_block_index: 0,
                                    spent: false,
                                    coinbase: false,
                                    timestamp: coin_record.timestamp,
                                },
                            )
                            .await?;
                        } else if let Condition::CreatePuzzleAnnouncement(cpa) = cond {
                            // may be minting an NFT the old way
                            let Some(block_record) = client
                                .get_block_record_by_height(coin_record.spent_block_index)
                                .await?
                                .block_record
                            else {
                                return Err(CliError::Message(format!(
                                    "failed to get block record for spent block with height {}",
                                    coin_record.spent_block_index
                                )));
                            };
                            let Some(block_spends) = client
                                .get_block_spends(block_record.header_hash)
                                .await?
                                .block_spends
                            else {
                                return Err(CliError::Message(format!(
                                    "failed to get block spends for block 0x{}",
                                    hex::encode(block_record.header_hash)
                                )));
                            };

                            let ann_id =
                                announcement_id(coin_record.coin.puzzle_hash, cpa.message.to_vec());
                            for spend in block_spends {
                                if spend.coin.coin_id() == coin_record.coin.coin_id() {
                                    continue;
                                }

                                let this_puzzle = ctx.alloc(&spend.puzzle_reveal)?;
                                let this_solution = ctx.alloc(&spend.solution)?;
                                let this_output = ctx.run(this_puzzle, this_solution)?;
                                let this_conds = ctx.extract::<Conditions>(this_output)?;
                                let is_ann_receiver = this_conds.iter().any(|c| {
                                    if let Condition::AssertPuzzleAnnouncement(apa) = c {
                                        apa.announcement_id == ann_id
                                    } else {
                                        false
                                    }
                                });
                                if !is_ann_receiver {
                                    continue;
                                }

                                // this may be the old way of minting an NFT
                                // by minting it in the same block from a wallet ph,
                                //  then spending it and assigning its owner as the minter DID
                                let this_puzzle = Puzzle::parse(ctx, this_puzzle);
                                let Some(new_nft) =
                                    Nft::parse_child(ctx, spend.coin, this_puzzle, this_solution)?
                                else {
                                    continue; // not an NFT
                                };
                                let Some((current_nft, _, _)) =
                                    Nft::parse(ctx, spend.coin, this_puzzle, this_solution)?
                                else {
                                    return Err(CliError::Message(format!(
                                        "failed to parse current NFT for coin 0x{}",
                                        hex::encode(coin_data.coin_id)
                                    )));
                                };

                                if new_nft.info.current_owner != coin_data.launcher_id {
                                    continue; // minter DID not assigned as owner
                                }
                                if current_nft.coin.parent_coin_info != new_nft.info.launcher_id {
                                    // Needs to be in the first spend, otherwise the NFT is not 'minted' by the DID
                                    continue;
                                }

                                db::add_coin_to_db(
                                    pool,
                                    CoinType::Nft,
                                    Some(new_nft.info.launcher_id),
                                    coin_data.launcher_id, // DID's launcher id
                                    &CoinRecord {
                                        coin: current_nft.coin,
                                        confirmed_block_index: coin_record.spent_block_index,
                                        spent_block_index: 0,
                                        spent: false,
                                        coinbase: false,
                                        timestamp: block_record
                                            .timestamp
                                            .unwrap_or(coin_record.timestamp),
                                    },
                                )
                                .await?;
                                println!(
                                    "Added NFT minted in the old way with id {}",
                                    encode_nft_launcher_id(&new_nft.info.launcher_id)?
                                );
                            }
                        }
                    }
                    db::update_coin_spent_height(
                        pool,
                        &coin_data.coin_id,
                        coin_record.spent_block_index,
                    )
                    .await?;
                }
                CoinType::IntermediaryCoin => {
                    let res = ctx.run(puzzle, solution)?;
                    let output_conds = ctx.extract::<Conditions>(res)?;
                    for cond in output_conds.iter() {
                        if let Condition::CreateCoin(cc) = cond {
                            if puzzle_hashes_to_follow.contains(&coin_data.puzzle_hash)
                                && cc.puzzle_hash != SINGLETON_LAUNCHER_HASH.into()
                            {
                                // When following an address, we only follow launchers created
                                //   directly by that address.
                                continue;
                            }

                            let new_coin = Coin::new(coin_data.coin_id, cc.puzzle_hash, cc.amount);
                            let (coin_type, launcher_id) =
                                if new_coin.puzzle_hash != SINGLETON_LAUNCHER_HASH.into() {
                                    (CoinType::IntermediaryCoin, None)
                                } else {
                                    // singleton launcher!
                                    (CoinType::Nft, Some(new_coin.coin_id()))
                                };
                            db::add_coin_to_db(
                                pool,
                                coin_type,
                                launcher_id,
                                None,
                                &CoinRecord {
                                    coin: new_coin,
                                    confirmed_block_index: coin_record.spent_block_index,
                                    spent_block_index: 0,
                                    spent: false,
                                    coinbase: false,
                                    timestamp: coin_record.timestamp,
                                },
                            )
                            .await?;
                        }
                    }
                    db::update_coin_spent_height(
                        pool,
                        &coin_data.coin_id,
                        coin_record.spent_block_index,
                    )
                    .await?;
                }
            }
        }
    }

    Ok(())
}

async fn fetch_tracked_puzzle_hashes(
    pool: &SqlitePool,
) -> Result<Vec<TrackedPuzzleHash>, CliError> {
    let rows = sqlx::query(
        r#"
        SELECT puzzle_hash, last_sync_height
        FROM tracked_puzzle_hashes
        ORDER BY puzzle_hash
        "#,
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| db::row_to_tracked_puzzle_hash(&row))
        .collect::<Result<Vec<_>, _>>()
}

async fn fetch_unspent_coins(pool: &SqlitePool) -> Result<Vec<DbCoin>, CliError> {
    let rows = sqlx::query(
        r#"
        SELECT type, launcher_id, did_launcher_id, parent_coin_id, puzzle_hash, coin_id, created_height, spent_height
        FROM coins
        WHERE spent_height IS NULL
        ORDER BY created_height, coin_id
        "#,
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| db::row_to_coin(&row))
        .collect::<Result<Vec<_>, _>>()
}
