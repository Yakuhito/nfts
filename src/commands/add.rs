use chia_wallet_sdk::prelude::Bytes32;
use sqlx::SqlitePool;

use crate::cli::AddArgs;
use crate::error::CliError;
use crate::models::CoinType;
use crate::utils::{TrackedKind, classify_prefixed_launcher_id, parse_launcher_id};

pub async fn run(pool: &SqlitePool, args: AddArgs) -> Result<(), CliError> {
    let entries = collect_entries(&args.value).await?;
    for (raw_id, kind) in entries {
        match kind {
            TrackedKind::Nft => add_coin(pool, &raw_id, CoinType::Nft).await?,
            TrackedKind::Did => add_coin(pool, &raw_id, CoinType::Did).await?,
        }
    }
    Ok(())
}

async fn collect_entries(value: &str) -> Result<Vec<(String, TrackedKind)>, CliError> {
    if value.contains("nft") || value.contains("did:chia:") {
        let mut out = Vec::new();
        for raw in value.split(',').map(str::trim).filter(|v| !v.is_empty()) {
            let Some(kind) = classify_prefixed_launcher_id(raw) else {
                return Err(CliError::Message(format!(
                    "invalid launcher id in comma-separated input (must start with nft or did:chia:): {raw}"
                )));
            };
            out.push((raw.to_string(), kind));
        }
        if out.is_empty() {
            return Err(CliError::Message(
                "no launcher ids found in input".to_string(),
            ));
        }
        return Ok(out);
    }

    let content = tokio::fs::read_to_string(value).await?;
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }
        if let Some(kind) = classify_prefixed_launcher_id(raw) {
            out.push((raw.to_string(), kind));
        } else {
            println!(
                "Skipping line {}: '{}' is not a valid NFT/DID launcher id prefix",
                idx + 1,
                raw
            );
        }
    }
    Ok(out)
}

async fn add_coin(pool: &SqlitePool, raw_id: &str, coin_type: CoinType) -> Result<(), CliError> {
    let launcher_id = parse_launcher_id(raw_id)?;
    let placeholder = Bytes32::default();
    // TODO(yakuhito): Resolve launcher_id through coinset/chia-wallet-sdk into a full Coin row
    // (including puzzle hash, actual coin lineage and heights) before upserting.
    sqlx::query(
        r#"
        INSERT INTO coins (type, launcher_id, did_launcher_id, parent_coin_id, puzzle_hash, coin_id, created_height, spent_height)
        VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, NULL)
        ON CONFLICT(coin_id) DO NOTHING
        "#,
    )
    .bind(coin_type.as_db_str())
    .bind(launcher_id.to_vec())
    .bind(placeholder.to_vec())
    .bind(placeholder.to_vec())
    .bind(placeholder.to_vec())
    .bind(0_i64)
    .execute(pool)
    .await?;

    Ok(())
}
