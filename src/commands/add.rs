use chia_wallet_sdk::prelude::Bytes32;
use sqlx::SqlitePool;

use crate::cli::{AddArgs, AddInputArgs, AddKind};
use crate::error::CliError;
use crate::utils::parse_launcher_id;

pub async fn run(pool: &SqlitePool, args: AddArgs) -> Result<(), CliError> {
    match args.kind {
        AddKind::Nft(input) => add_nfts(pool, &input).await?,
        AddKind::Did(input) => add_dids(pool, &input).await?,
    }
    Ok(())
}

async fn add_nfts(pool: &SqlitePool, input: &AddInputArgs) -> Result<(), CliError> {
    let ids = collect_input_ids(input).await?;
    for raw_id in ids {
        let launcher_id = parse_launcher_id(&raw_id)?;
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
    }
    Ok(())
}

async fn add_dids(pool: &SqlitePool, input: &AddInputArgs) -> Result<(), CliError> {
    let ids = collect_input_ids(input).await?;
    for raw_id in ids {
        let launcher_id = parse_launcher_id(&raw_id)?;
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
    }
    Ok(())
}

async fn collect_input_ids(args: &AddInputArgs) -> Result<Vec<String>, CliError> {
    let mut ids = Vec::new();

    if let Some(csv_ids) = &args.ids {
        ids.extend(
            csv_ids
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(ToString::to_string),
        );
    }

    if let Some(file_path) = &args.file {
        let content = tokio::fs::read_to_string(file_path).await?;
        ids.extend(
            content
                .lines()
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(ToString::to_string),
        );
    }

    if ids.is_empty() {
        return Err(CliError::Message(
            "provide launcher IDs with --ids and/or --file".to_string(),
        ));
    }

    Ok(ids)
}
