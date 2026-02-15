use sqlx::{Row, SqlitePool};

use crate::cli::ListArgs;
use crate::error::CliError;
use crate::models::{Did, Nft};
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
            let launcher_string = encode_did_launcher_id(&did.launcher_id)?;
            println!("- {launcher_string}");
        }
    }

    if !args.dids_only {
        let nfts = fetch_nfts(pool).await?;
        println!("Tracked NFTs: {}", nfts.len());
        for nft in nfts {
            let launcher_string = encode_nft_launcher_id(&nft.launcher_id)?;
            println!("- {launcher_string}");
        }
    }

    Ok(())
}

async fn fetch_dids(pool: &SqlitePool) -> Result<Vec<Did>, CliError> {
    let rows = sqlx::query(
        r#"
        SELECT launcher_id, parent_coin_id, coin_id, spent_height
        FROM dids
        WHERE spent_height IS NULL
          AND parent_coin_id IS NOT NULL
          AND coin_id IS NOT NULL
        ORDER BY launcher_id
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let spent_height = row
            .get::<Option<i64>, _>("spent_height")
            .and_then(|v| u32::try_from(v).ok());
        out.push(Did {
            launcher_id: bytes32_from_db("did launcher_id", row.get::<&[u8], _>("launcher_id"))?,
            parent_coin_id: bytes32_from_db(
                "did parent_coin_id",
                row.get::<&[u8], _>("parent_coin_id"),
            )?,
            coin_id: bytes32_from_db("did coin_id", row.get::<&[u8], _>("coin_id"))?,
            spent_height,
        });
    }

    Ok(out)
}

async fn fetch_nfts(pool: &SqlitePool) -> Result<Vec<Nft>, CliError> {
    let rows = sqlx::query(
        r#"
        SELECT launcher_id, did_launcher_id, parent_coin_id, coin_id, inner_puzzle_hash, spent_height
        FROM nfts
        WHERE spent_height IS NULL
          AND did_launcher_id IS NULL
          AND parent_coin_id IS NOT NULL
          AND coin_id IS NOT NULL
          AND inner_puzzle_hash IS NOT NULL
        ORDER BY launcher_id
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let spent_height = row
            .get::<Option<i64>, _>("spent_height")
            .and_then(|v| u32::try_from(v).ok());
        out.push(Nft {
            launcher_id: bytes32_from_db("nft launcher_id", row.get::<&[u8], _>("launcher_id"))?,
            did_launcher_id: optional_bytes32_from_db(
                "nft did_launcher_id",
                row.get::<Option<&[u8]>, _>("did_launcher_id"),
            )?,
            parent_coin_id: bytes32_from_db(
                "nft parent_coin_id",
                row.get::<&[u8], _>("parent_coin_id"),
            )?,
            coin_id: bytes32_from_db("nft coin_id", row.get::<&[u8], _>("coin_id"))?,
            inner_puzzle_hash: bytes32_from_db(
                "nft inner_puzzle_hash",
                row.get::<&[u8], _>("inner_puzzle_hash"),
            )?,
            spent_height,
        });
    }

    Ok(out)
}
