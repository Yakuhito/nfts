mod cli;
mod commands;
mod db;
mod error;
mod models;
mod premine;
mod utils;

use clap::Parser;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::cli::{Cli, Command};
use crate::error::CliError;

#[tokio::main]
async fn main() -> Result<(), CliError> {
    let cli = Cli::parse();

    let db_options = SqliteConnectOptions::new()
        .filename(&cli.db)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(db_options)
        .await?;

    db::ensure_schema(&pool).await?;

    match cli.command {
        Command::List(args) => commands::list::run(&pool, args).await?,
        Command::Add(args) => commands::add::run(&pool, args).await?,
        Command::Sync(args) => commands::sync::run(&pool, args).await?,
        Command::Query(args) => commands::query::run(&pool, args).await?,
        Command::Premine(args) => commands::premine::run(&pool, args).await?,
    }

    Ok(())
}
