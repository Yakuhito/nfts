use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use crate::error::CliError;
use crate::models::{Coin as DbCoin, CoinType, TrackedPuzzleHash};
use crate::utils::{bytes32_from_db, optional_bytes32_from_db};
use chia_wallet_sdk::prelude::{Bytes32, CoinRecord};

pub async fn ensure_schema(pool: &SqlitePool) -> Result<(), CliError> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS tracked_puzzle_hashes (
            puzzle_hash BLOB PRIMARY KEY NOT NULL,
            last_sync_height INTEGER NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS coins (
            type TEXT NOT NULL,
            launcher_id BLOB NULL,
            did_launcher_id BLOB NULL,
            parent_coin_id BLOB NOT NULL,
            puzzle_hash BLOB NOT NULL,
            coin_id BLOB PRIMARY KEY NOT NULL,
            created_height INTEGER NOT NULL,
            spent_height INTEGER NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn add_coin_to_db(
    pool: &SqlitePool,
    coin_type: CoinType,
    launcher_id: Option<Bytes32>,
    did_launcher_id: Option<Bytes32>,
    coin_record: &CoinRecord,
) -> Result<(), CliError> {
    sqlx::query(
        r#"
        INSERT INTO coins (type, launcher_id, did_launcher_id, parent_coin_id, puzzle_hash, coin_id, created_height, spent_height)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(coin_id) DO NOTHING
        "#,
    )
    .bind(coin_type.as_db_str())
    .bind(launcher_id.map(|id| id.to_vec()))
    .bind(did_launcher_id.map(|id| id.to_vec()))
    .bind(coin_record.coin.parent_coin_info.to_vec())
    .bind(coin_record.coin.puzzle_hash.to_vec())
    .bind(coin_record.coin.coin_id().to_vec())
    .bind(coin_record.confirmed_block_index)
    .bind(if coin_record.spent_block_index == 0 {
        None
    } else {
        Some(coin_record.spent_block_index)
    })
    .execute(pool)
    .await?;

    Ok(())
}

pub fn row_to_coin(row: &SqliteRow) -> Result<DbCoin, CliError> {
    let created_height = row
        .get::<i64, _>("created_height")
        .try_into()
        .map_err(|_| CliError::Message("invalid created_height in db".to_string()))?;
    let spent_height = row
        .get::<Option<i64>, _>("spent_height")
        .and_then(|v| u32::try_from(v).ok());
    let coin_type_raw = row.get::<String, _>("type");
    let coin_type = CoinType::from_db_str(&coin_type_raw)
        .ok_or_else(|| CliError::Message(format!("invalid coin type in db: {coin_type_raw}")))?;

    Ok(DbCoin {
        coin_type,
        launcher_id: optional_bytes32_from_db(
            "coin launcher_id",
            row.get::<Option<&[u8]>, _>("launcher_id"),
        )?,
        did_launcher_id: optional_bytes32_from_db(
            "coin did_launcher_id",
            row.get::<Option<&[u8]>, _>("did_launcher_id"),
        )?,
        parent_coin_id: bytes32_from_db(
            "coin parent_coin_id",
            row.get::<&[u8], _>("parent_coin_id"),
        )?,
        puzzle_hash: bytes32_from_db("coin puzzle_hash", row.get::<&[u8], _>("puzzle_hash"))?,
        coin_id: bytes32_from_db("coin coin_id", row.get::<&[u8], _>("coin_id"))?,
        created_height,
        spent_height,
    })
}

pub fn row_to_tracked_puzzle_hash(row: &SqliteRow) -> Result<TrackedPuzzleHash, CliError> {
    let last_sync_height = row
        .get::<i64, _>("last_sync_height")
        .try_into()
        .map_err(|_| CliError::Message("invalid last_sync_height in db".to_string()))?;

    Ok(TrackedPuzzleHash {
        puzzle_hash: bytes32_from_db(
            "tracked_puzzle_hashes puzzle_hash",
            row.get::<&[u8], _>("puzzle_hash"),
        )?,
        last_sync_height,
    })
}

pub async fn update_tracked_puzzle_hash_sync_height(
    pool: &SqlitePool,
    puzzle_hash: &Bytes32,
    last_sync_height: u32,
) -> Result<(), CliError> {
    sqlx::query(
        r#"
        UPDATE tracked_puzzle_hashes SET last_sync_height = ?1 WHERE puzzle_hash = ?2
        "#,
    )
    .bind(last_sync_height)
    .bind(puzzle_hash.to_vec())
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn update_coin_spent_height(
    pool: &SqlitePool,
    coin_id: &Bytes32,
    spent_height: u32,
) -> Result<(), CliError> {
    sqlx::query(
        r#"
        UPDATE coins SET spent_height = ?1 WHERE coin_id = ?2
        "#,
    )
    .bind(spent_height)
    .bind(coin_id.to_vec())
    .execute(pool)
    .await?;

    Ok(())
}
