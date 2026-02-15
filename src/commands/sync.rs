use chia_wallet_sdk::prelude::{ChiaRpcClient, CoinsetClient};
use chia_wallet_sdk::puzzles::SINGLETON_LAUNCHER_HASH;
use sqlx::SqlitePool;

use crate::cli::SyncArgs;
use crate::db;
use crate::error::CliError;
use crate::models::{Coin as DbCoin, CoinType, TrackedPuzzleHash};

pub async fn run(pool: &SqlitePool, args: SyncArgs) -> Result<(), CliError> {
    let client = CoinsetClient::mainnet();
    let (puzzle_hashes, unspent_db_coins) = collect_sync_targets(pool).await?;

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
                puzzle_hashes.iter().map(|p| p.puzzle_hash).collect(),
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
                if coin_record.coin.puzzle_hash == SINGLETON_LAUNCHER_HASH.into() {
                    (CoinType::IntermediaryCoin, None)
                } else {
                    // singleton launcher!
                    (CoinType::Nft, Some(coin_record.coin.coin_id()))
                };
            db::add_coin_to_db(pool, coin_type, launcher_id, None, &coin_record).await?;
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
    Ok(())
}

pub async fn collect_sync_targets(
    pool: &SqlitePool,
) -> Result<(Vec<TrackedPuzzleHash>, Vec<DbCoin>), CliError> {
    let tracked_puzzle_hashes = fetch_tracked_puzzle_hashes(pool).await?;
    let unspent_coins = fetch_unspent_coins(pool).await?;
    Ok((tracked_puzzle_hashes, unspent_coins))
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
