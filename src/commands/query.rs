use chia_wallet_sdk::utils::Address;
use sqlx::SqlitePool;

use crate::cli::QueryArgs;
use crate::db;
use crate::error::CliError;
use crate::models::Coin;
use crate::utils::is_nft_launcher_id;

pub async fn run(pool: &SqlitePool, args: QueryArgs) -> Result<(), CliError> {
    if !is_nft_launcher_id(&args.nft_id) {
        return Err(CliError::Message("nft id must start with nft1".to_string()));
    }
    let launcher_id = Address::decode(&args.nft_id)?.puzzle_hash;

    let coins = fetch_nft_coins(pool, launcher_id.to_vec()).await?;
    if coins.is_empty() {
        println!("No coins found for NFT id {}", args.nft_id);
        return Ok(());
    }

    let mut spent = Vec::new();
    let mut unspent = Vec::new();
    for coin in coins {
        if coin.spent_height.is_some() {
            spent.push(coin);
        } else {
            unspent.push(coin);
        }
    }

    println!("NFT: {}", args.nft_id);
    println!("Spent coins: {}", spent.len());
    for coin in spent {
        print_coin(&coin)?;
    }
    println!("Unspent coins: {}", unspent.len());
    for coin in unspent {
        print_coin(&coin)?;
    }

    Ok(())
}

async fn fetch_nft_coins(pool: &SqlitePool, launcher_id: Vec<u8>) -> Result<Vec<Coin>, CliError> {
    let rows = sqlx::query(
        r#"
        SELECT type, launcher_id, did_launcher_id, parent_coin_id, puzzle_hash, coin_id, created_height, spent_height, metadata, inner_puzzle_hash
        FROM coins
        WHERE type = 'NFT'
          AND launcher_id = ?1
        ORDER BY created_height, coin_id
        "#,
    )
    .bind(launcher_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| db::row_to_coin(&row))
        .collect::<Result<Vec<_>, _>>()
}

fn print_coin(coin: &Coin) -> Result<(), CliError> {
    let spent_block = coin
        .spent_height
        .map(|h| h.to_string())
        .unwrap_or_else(|| "unspent".to_string());
    println!(
        "- coin_id: 0x{}, spent_block: {}",
        hex::encode(coin.coin_id),
        spent_block
    );
    Ok(())
}
