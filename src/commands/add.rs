use chia_wallet_sdk::prelude::Bytes32;
use sqlx::SqlitePool;

use crate::cli::AddArgs;
use crate::error::CliError;
use crate::utils::{TrackedKind, classify_prefixed_launcher_id, parse_launcher_id};

pub async fn run(pool: &SqlitePool, args: AddArgs) -> Result<(), CliError> {
    let entries = collect_entries(&args.value).await?;
    for (raw_id, kind) in entries {
        match kind {
            TrackedKind::Nft => add_nft(pool, &raw_id).await?,
            TrackedKind::Did => add_did(pool, &raw_id).await?,
        }
    }
    Ok(())
}

async fn collect_entries(value: &str) -> Result<Vec<(String, TrackedKind)>, CliError> {
    if value.contains("nft1") || value.contains("did:chia:") {
        let mut out = Vec::new();
        for raw in value.split(',').map(str::trim).filter(|v| !v.is_empty()) {
            let Some(kind) = classify_prefixed_launcher_id(raw) else {
                return Err(CliError::Message(format!(
                    "invalid launcher id in comma-separated input (must start with nft1 or did:chia:): {raw}"
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

async fn add_nft(pool: &SqlitePool, raw_id: &str) -> Result<(), CliError> {
    let launcher_id = parse_launcher_id(raw_id)?;
    let placeholder = Bytes32::default();

    // TODO(yakuhito): Resolve launcher_id through coinset/chia-wallet-sdk into
    // the full NFT object (coin lineage, inner_puzzle_hash, did linkage),
    // then insert/update the full record in the database.
    sqlx::query(
        r#"
        INSERT INTO nfts (launcher_id, did_launcher_id, parent_coin_id, coin_id, inner_puzzle_hash, spent_height)
        VALUES (?1, NULL, ?2, ?3, ?4, NULL)
        ON CONFLICT(launcher_id) DO NOTHING
        "#,
    )
    .bind(launcher_id.to_vec())
    .bind(placeholder.to_vec())
    .bind(placeholder.to_vec())
    .bind(placeholder.to_vec())
    .execute(pool)
    .await?;

    Ok(())
}

async fn add_did(pool: &SqlitePool, raw_id: &str) -> Result<(), CliError> {
    let launcher_id = parse_launcher_id(raw_id)?;
    let placeholder = Bytes32::default();

    // TODO(yakuhito): Resolve launcher_id through coinset/chia-wallet-sdk into
    // the full DID object (coin lineage and current spend state),
    // then insert/update the full record in the database.
    sqlx::query(
        r#"
        INSERT INTO dids (launcher_id, parent_coin_id, coin_id, spent_height)
        VALUES (?1, ?2, ?3, NULL)
        ON CONFLICT(launcher_id) DO NOTHING
        "#,
    )
    .bind(launcher_id.to_vec())
    .bind(placeholder.to_vec())
    .bind(placeholder.to_vec())
    .execute(pool)
    .await?;

    Ok(())
}
