pub mod confirm;
pub mod generate;
pub mod mintgarden_hydrate;

use crate::cli::{PremineArgs, PremineCommand};
use crate::error::CliError;
use sqlx::SqlitePool;

pub async fn run(pool: &SqlitePool, args: PremineArgs) -> Result<(), CliError> {
    match args.command {
        PremineCommand::Generate(generate_args) => generate::run(pool, generate_args).await,
        PremineCommand::Confirm(confirm_args) => confirm::run(confirm_args).await,
        PremineCommand::MintgardenHydrate(hydrate_args) => {
            mintgarden_hydrate::run(pool, hydrate_args).await
        }
    }
}
