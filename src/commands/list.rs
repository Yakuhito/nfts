use sqlx::{Row, SqlitePool};

use crate::cli::ListArgs;
use crate::error::CliError;
use crate::models::{Coin, CoinType};
use crate::utils::{
    bytes32_from_db, encode_did_launcher_id, encode_nft_launcher_id, optional_bytes32_from_db,
};

pub async fn run(pool: &SqlitePool, args: ListArgs) -> Result<(), CliError> {
    if args.nfts_only && args.dids_only {
        return Err(CliError::Message(
            "cannot combine --nfts-only and --dids-only".to_string(),
        ));
    }

    if !args.nfts_only {
        let dids = fetch_dids(pool).await?;
        println!("Tracked DIDs: {}", dids.len());
        for did in dids {
            let launcher_id = did
                .launcher_id
                .ok_or_else(|| CliError::Message("DID row is missing launcher_id".to_string()))?;
            let launcher_string = encode_did_launcher_id(&launcher_id)?;
            println!("- {launcher_string}");
        }
    }

    if !args.dids_only {
        let nfts = fetch_nfts(pool).await?;
        println!("Tracked NFTs (not minted by a tracked DID): {}", nfts.len());
        for nft in nfts {
            let launcher_id = nft
                .launcher_id
                .ok_or_else(|| CliError::Message("NFT row is missing launcher_id".to_string()))?;
            let launcher_string = encode_nft_launcher_id(&launcher_id)?;
            println!("- {launcher_string}");
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

    rows_to_coins(rows)
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

    rows_to_coins(rows)
}

fn rows_to_coins(rows: Vec<sqlx::sqlite::SqliteRow>) -> Result<Vec<Coin>, CliError> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let created_height = row
            .get::<i64, _>("created_height")
            .try_into()
            .map_err(|_| CliError::Message("invalid created_height in db".to_string()))?;
        let spent_height = row
            .get::<Option<i64>, _>("spent_height")
            .and_then(|v| u32::try_from(v).ok());
        let coin_type_raw = row.get::<String, _>("type");
        let coin_type = CoinType::from_db_str(&coin_type_raw).ok_or_else(|| {
            CliError::Message(format!("invalid coin type in db: {coin_type_raw}"))
        })?;

        out.push(Coin {
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
        });
    }

    Ok(out)
}
