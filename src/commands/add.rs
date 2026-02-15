use chia_wallet_sdk::prelude::{Bytes32, ChiaRpcClient, CoinsetClient};
use chia_wallet_sdk::utils::Address;
use sqlx::SqlitePool;

use crate::cli::AddArgs;
use crate::error::CliError;
use crate::utils::{TrackedKind, classify_prefixed_launcher_id};

pub async fn run(pool: &SqlitePool, args: AddArgs) -> Result<(), CliError> {
    let client = CoinsetClient::mainnet();

    let entries = collect_entries(&args.value).await?;
    for (raw_id, kind) in entries {
        add_coin(pool, &client, raw_id, kind).await?
    }
    Ok(())
}

async fn collect_entries(value: &str) -> Result<Vec<(Bytes32, TrackedKind)>, CliError> {
    if value.contains("nft1") || value.contains("did:chia:1") || value.contains("xch1") {
        let mut out = Vec::new();
        for raw in value.split(',').map(str::trim).filter(|v| !v.is_empty()) {
            let Some(kind) = classify_prefixed_launcher_id(raw) else {
                return Err(CliError::Message(format!(
                    "invalid launcher id in comma-separated input (must start with nft or did:chia: or xch): {raw}"
                )));
            };
            out.push((Address::decode(raw)?.puzzle_hash, kind));
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
            out.push((Address::decode(raw)?.puzzle_hash, kind));
        } else {
            println!(
                "Skipping line {}: '{}' is not a valid NFT/DID launcher id/address prefix",
                idx + 1,
                raw
            );
        }
    }

    Ok(out)
}

async fn add_coin(
    pool: &SqlitePool,
    client: &CoinsetClient,
    launcher_id_or_ph: Bytes32,
    coin_type: TrackedKind,
) -> Result<(), CliError> {
    match coin_type {
        TrackedKind::Nft | TrackedKind::Did => {
            let Some(coin_record) = client
                .get_coin_record_by_name(launcher_id_or_ph)
                .await
                .map_err(CliError::Request)?
                .coin_record
            else {
                return Err(CliError::Message(format!(
                    "coin record not found for launcher id {launcher_id_or_ph}"
                )));
            };

            sqlx::query(
                    r#"
                    INSERT INTO coins (type, launcher_id, did_launcher_id, parent_coin_id, puzzle_hash, coin_id, created_height, spent_height)
                    VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, NULL)
                    ON CONFLICT(coin_id) DO NOTHING
                    "#,
                )
                .bind(coin_type.to_coin_type().as_db_str())
                .bind(launcher_id_or_ph.to_vec())
                .bind(coin_record.coin.parent_coin_info.to_vec())
                .bind(coin_record.coin.puzzle_hash.to_vec())
                .bind(coin_record.coin.coin_id().to_vec())
                .bind(coin_record.confirmed_block_index)
                .execute(pool)
                .await?;
        }
        TrackedKind::PuzzleHash => {
            sqlx::query(
                r#"
                INSERT INTO tracked_puzzle_hashes (puzzle_hash, last_sync_height)
                VALUES (?1, ?2)
                ON CONFLICT(puzzle_hash) DO NOTHING
                "#,
            )
            .bind(launcher_id_or_ph.to_vec())
            .bind(0)
            .execute(pool)
            .await?;
        }
    }

    Ok(())
}
