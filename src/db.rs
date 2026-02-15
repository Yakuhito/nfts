use sqlx::SqlitePool;

use crate::error::CliError;

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
