use chia_wallet_sdk::driver::SpendContext;
use chia_wallet_sdk::prelude::{Bytes32, ChiaRpcClient, Coin, CoinRecord, CoinsetClient};
use chia_wallet_sdk::puzzles::SINGLETON_LAUNCHER_HASH;
use chia_wallet_sdk::types::{Condition, Conditions};
use sqlx::SqlitePool;

use crate::cli::SyncArgs;
use crate::db;
use crate::error::CliError;
use crate::models::{Coin as DbCoin, CoinType, TrackedPuzzleHash};

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

    loop {
        println!("New sync loop: fetching unspent coins...");
        let unspent_coins = fetch_unspent_coins(pool).await?;

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

            spent_coin_records.extend(
                coin_records
                    .into_iter()
                    .zip(coin_data)
                    .filter(|c| c.0.spent),
            );
        }

        if spent_coin_records.is_empty() {
            println!("No spent coin records found. Sync complete!");
            break;
        }

        println!(
            "Fetching solutions for and processing {} spent coin records...",
            spent_coin_records.len()
        );
        for (coin_record, coin_data) in spent_coin_records {
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
                    // TODO: parse DIDs
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
