use sqlx::SqlitePool;

use crate::cli::ListArgs;
use crate::db;
use crate::error::CliError;
use crate::models::{Coin, TrackedPuzzleHash};
use crate::utils::{encode_did_launcher_id, encode_nft_launcher_id, encode_puzzle_hash_address};

pub async fn run(pool: &SqlitePool, args: ListArgs) -> Result<(), CliError> {
    let dids = fetch_dids(pool).await?;
    if !args.exclude_dids {
        println!("Tracked DIDs: {}", dids.len());
        for did in dids {
            let launcher_id = did
                .launcher_id
                .ok_or_else(|| CliError::Message("DID row is missing launcher_id".to_string()))?;
            let launcher_string = encode_did_launcher_id(&launcher_id)?;
            println!("- {launcher_string}");
        }
    }

    if !args.exclude_non_collection_nfts {
        let nfts = fetch_nfts(pool).await?;
        println!(
            "Tracked NFTs (independent from tracked DIDs): {}",
            nfts.len()
        );
        for nft in nfts {
            let launcher_id = nft
                .launcher_id
                .ok_or_else(|| CliError::Message("NFT row is missing launcher_id".to_string()))?;
            let launcher_string = encode_nft_launcher_id(&launcher_id)?;
            println!("- {launcher_string}");
        }
    }

    if !args.exclude_addresses {
        let puzzle_hashes = fetch_tracked_puzzle_hashes(pool).await?;
        println!("Tracked addresses: {}", puzzle_hashes.len());
        for entry in puzzle_hashes {
            let address = encode_puzzle_hash_address(&entry.puzzle_hash)?;
            println!("- {address}");
        }
    }

    Ok(())
}

async fn fetch_dids(pool: &SqlitePool) -> Result<Vec<Coin>, CliError> {
    let rows = sqlx::query(
        r#"
        SELECT type, launcher_id, did_launcher_id, parent_coin_id, puzzle_hash, coin_id, created_height, spent_height
        FROM coins
        WHERE type = 'DID'
          AND launcher_id IS NOT NULL
          AND spent_height IS NULL
        ORDER BY launcher_id
        "#,
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| db::row_to_coin(&row))
        .collect::<Result<Vec<_>, _>>()
}

async fn fetch_nfts(pool: &SqlitePool) -> Result<Vec<Coin>, CliError> {
    let rows = sqlx::query(
        r#"
        SELECT type, launcher_id, did_launcher_id, parent_coin_id, puzzle_hash, coin_id, created_height, spent_height
        FROM coins
        WHERE type = 'NFT'
          AND launcher_id IS NOT NULL
          AND did_launcher_id IS NULL
          AND spent_height IS NULL
        ORDER BY launcher_id
        "#,
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| db::row_to_coin(&row))
        .collect::<Result<Vec<_>, _>>()
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
