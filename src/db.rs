use sqlx::{Row, SqlitePool, sqlite::SqliteRow, types::Json};
use std::collections::BTreeSet;

use crate::error::CliError;
use crate::models::{
    Coin as DbCoin, CoinType, MetadataValue, OffchainMetadata, ParsedMetadata, TrackedPuzzleHash,
};
use crate::utils::{bytes32_from_db, optional_bytes32_from_db};
use chia_wallet_sdk::prelude::{Bytes32, Coin};
use serde_json::Value as JsonValue;

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
            spent_height INTEGER NULL,
            metadata JSON NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS offchain_metadata (
            metadata_hash BLOB PRIMARY KEY NOT NULL,
            urls JSON NOT NULL,
            value TEXT NULL,
            next_retry TIMESTAMP NULL,
            retries_so_far INTEGER NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Older databases may lack this column; ADD COLUMN is idempotent enough via probe.
    let has_inner = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*) FROM pragma_table_info('coins') WHERE name = 'inner_puzzle_hash'
        "#,
    )
    .fetch_one(pool)
    .await?;
    if has_inner == 0 {
        sqlx::query(
            r#"
            ALTER TABLE coins ADD COLUMN inner_puzzle_hash BLOB NULL
            "#,
        )
        .execute(pool)
        .await?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn add_coin_to_db(
    pool: &SqlitePool,
    coin_type: CoinType,
    launcher_id: Option<Bytes32>,
    did_launcher_id: Option<Bytes32>,
    coin: &Coin,
    created_height: u32,
    metadata: Option<ParsedMetadata>,
    inner_puzzle_hash: Option<Bytes32>,
) -> Result<(), CliError> {
    if let Some(metadata) = metadata.as_ref() {
        upsert_offchain_metadata_from_parsed_metadata(pool, metadata).await?;
    }

    let metadata_json = metadata
        .map(|metadata| {
            serde_json::to_value(metadata).map_err(|err| {
                CliError::Message(format!("failed to serialize coin metadata as JSON: {err}"))
            })
        })
        .transpose()?
        .map(Json);

    sqlx::query(
        r#"
        INSERT INTO coins (
            type, launcher_id, did_launcher_id, parent_coin_id, puzzle_hash, coin_id,
            created_height, spent_height, metadata, inner_puzzle_hash
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9)
        ON CONFLICT(coin_id) DO NOTHING
        "#,
    )
    .bind(coin_type.as_db_str())
    .bind(launcher_id.map(|id| id.to_vec()))
    .bind(did_launcher_id.map(|id| id.to_vec()))
    .bind(coin.parent_coin_info.to_vec())
    .bind(coin.puzzle_hash.to_vec())
    .bind(coin.coin_id().to_vec())
    .bind(created_height)
    .bind(metadata_json)
    .bind(inner_puzzle_hash.map(|id| id.to_vec()))
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn upsert_offchain_metadata_from_parsed_metadata(
    pool: &SqlitePool,
    metadata: &ParsedMetadata,
) -> Result<(), CliError> {
    let Some((metadata_hash, urls)) = extract_offchain_metadata(metadata)? else {
        return Ok(());
    };

    sqlx::query(
        r#"
        INSERT INTO offchain_metadata (metadata_hash, urls, value, next_retry, retries_so_far)
        VALUES (?1, ?2, NULL, NULL, 0)
        ON CONFLICT(metadata_hash) DO UPDATE SET
            urls = (
                SELECT COALESCE(json_group_array(value), '[]')
                FROM (
                    SELECT value FROM json_each(offchain_metadata.urls)
                    UNION
                    SELECT value FROM json_each(excluded.urls)
                )
            )
        "#,
    )
    .bind(metadata_hash.to_vec())
    .bind(Json(urls))
    .execute(pool)
    .await?;

    Ok(())
}

fn extract_offchain_metadata(
    metadata: &ParsedMetadata,
) -> Result<Option<(Bytes32, Vec<String>)>, CliError> {
    let Some(metadata_hash_value) = metadata.0.get("mh") else {
        return Ok(None);
    };
    let Some(urls_value) = metadata.0.get("mu") else {
        return Ok(None);
    };

    let metadata_hash = parse_metadata_hash(metadata_hash_value)?;
    let urls = parse_metadata_urls(urls_value);

    Ok(Some((metadata_hash, urls)))
}

fn parse_metadata_hash(value: &MetadataValue) -> Result<Bytes32, CliError> {
    match value {
        MetadataValue::Bytes32(hash) => Ok(*hash),
        MetadataValue::String(raw) => {
            let normalized = raw.trim_start_matches("0x");
            let bytes = hex::decode(normalized).map_err(|err| {
                CliError::Message(format!("invalid metadata hash hex in mh: {err}"))
            })?;
            if bytes.len() != 32 {
                return Err(CliError::Message(format!(
                    "invalid metadata hash length in mh: {}, expected 32",
                    bytes.len()
                )));
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(Bytes32::new(arr))
        }
        _ => Err(CliError::Message(
            "invalid metadata hash in mh: expected Bytes32 or hex string".to_string(),
        )),
    }
}

fn parse_metadata_urls(value: &MetadataValue) -> Vec<String> {
    let urls = match value {
        MetadataValue::StringArray(urls) => urls.clone(),
        MetadataValue::String(url) => vec![url.clone()],
        _ => Vec::new(),
    };

    let mut unique = BTreeSet::new();
    for url in urls.into_iter().filter(|url| !url.is_empty()) {
        unique.insert(url);
    }
    unique.into_iter().collect()
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
    let metadata = row
        .get::<Option<Json<JsonValue>>, _>("metadata")
        .map(|json| json.0);
    let inner_puzzle_hash = optional_bytes32_from_db(
        "coin inner_puzzle_hash",
        row.get::<Option<&[u8]>, _>("inner_puzzle_hash"),
    )?;

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
        metadata,
        inner_puzzle_hash,
    })
}

pub async fn update_offchain_metadata_value(
    pool: &SqlitePool,
    metadata_hash: &Bytes32,
    value: &str,
) -> Result<(), CliError> {
    sqlx::query(
        r#"
        UPDATE offchain_metadata
        SET value = ?1, next_retry = NULL
        WHERE metadata_hash = ?2
        "#,
    )
    .bind(value)
    .bind(metadata_hash.to_vec())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_coin_nft_fields(
    pool: &SqlitePool,
    coin_id: &Bytes32,
    metadata: &ParsedMetadata,
    inner_puzzle_hash: &Bytes32,
) -> Result<(), CliError> {
    upsert_offchain_metadata_from_parsed_metadata(pool, metadata).await?;
    let metadata_json = Json(serde_json::to_value(metadata).map_err(|err| {
        CliError::Message(format!("failed to serialize coin metadata as JSON: {err}"))
    })?);

    sqlx::query(
        r#"
        UPDATE coins
        SET metadata = ?1, inner_puzzle_hash = ?2
        WHERE coin_id = ?3
        "#,
    )
    .bind(metadata_json)
    .bind(inner_puzzle_hash.to_vec())
    .bind(coin_id.to_vec())
    .execute(pool)
    .await?;
    Ok(())
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

#[allow(dead_code)]
pub fn row_to_offchain_metadata(row: &SqliteRow) -> Result<OffchainMetadata, CliError> {
    let retries_so_far = row
        .get::<i64, _>("retries_so_far")
        .try_into()
        .map_err(|_| CliError::Message("invalid retries_so_far in db".to_string()))?;
    let urls = row.get::<Json<Vec<String>>, _>("urls").0;
    let next_retry = row
        .get::<Option<i64>, _>("next_retry")
        .map(|v| {
            u64::try_from(v).map_err(|_| CliError::Message("invalid next_retry in db".to_string()))
        })
        .transpose()?;

    Ok(OffchainMetadata {
        metadata_hash: bytes32_from_db(
            "offchain_metadata metadata_hash",
            row.get::<&[u8], _>("metadata_hash"),
        )?,
        urls,
        value: row.get::<Option<String>, _>("value"),
        next_retry,
        retries_so_far,
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
