use sqlx::SqlitePool;

use crate::error::CliError;

pub async fn ensure_schema(pool: &SqlitePool) -> Result<(), CliError> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS dids (
            launcher_id BLOB PRIMARY KEY NOT NULL,
            parent_coin_id BLOB NOT NULL,
            coin_id BLOB NOT NULL,
            spent_height INTEGER NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS intermediary_coins (
            parent_coin_id BLOB NOT NULL,
            coin_id BLOB PRIMARY KEY NOT NULL,
            spent_height INTEGER NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS nfts (
            launcher_id BLOB PRIMARY KEY NOT NULL,
            did_launcher_id BLOB NULL,
            parent_coin_id BLOB NOT NULL,
            coin_id BLOB NOT NULL,
            inner_puzzle_hash BLOB NOT NULL,
            spent_height INTEGER NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}
