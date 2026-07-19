use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "nfts")]
#[command(about = "Track NFTs and DIDs", version)]
pub struct Cli {
    /// SQLite database path
    #[arg(long, default_value = "nfts.db")]
    pub db: PathBuf,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List currently tracked items
    List(ListArgs),
    /// Add launcher ids to tracking
    Add(AddArgs),
    /// Sync tracked items
    Sync(SyncArgs),
    /// Show spent and unspent coins for an NFT id
    Query(QueryArgs),
    /// Base Premine generate / confirm
    Premine(PremineArgs),
}

#[derive(Args, Debug)]
pub struct PremineArgs {
    #[command(subcommand)]
    pub command: PremineCommand,
}

#[derive(Subcommand, Debug)]
pub enum PremineCommand {
    /// Generate the deterministic Base Premine from the local snapshot
    Generate(PremineGenerateArgs),
    /// Independently confirm a Base Premine CSV against MintGarden
    Confirm(PremineConfirmArgs),
    /// Fill missing CNS off-chain metadata via MintGarden (Pawket CRLF reconstruct, hash-asserted)
    MintgardenCnsHydrate(PremineMintgardenCnsHydrateArgs),
}

#[derive(Args, Debug)]
pub struct PremineGenerateArgs {
    /// Base Premine CSV output path
    #[arg(long)]
    pub output: PathBuf,
    /// Warnings CSV output path
    #[arg(long)]
    pub warnings: PathBuf,
}

#[derive(Args, Debug)]
pub struct PremineConfirmArgs {
    /// Base Premine CSV to validate (never modified)
    pub input: PathBuf,
}

#[derive(Args, Debug)]
pub struct PremineMintgardenCnsHydrateArgs {
    /// Optional path to a file of nft1... ids (one per line). Default: every CNS launcher still missing cached metadata.
    #[arg(long)]
    pub nfts_file: Option<PathBuf>,
    /// Max concurrent MintGarden metadata fetches
    #[arg(long, default_value_t = 8)]
    pub concurrency: usize,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Exclude NFTs that are not part of a collection (i.e. not minted by a tracked DID)
    #[arg(long)]
    pub exclude_non_collection_nfts: bool,
    /// Exclude tracked DIDs
    #[arg(long)]
    pub exclude_dids: bool,
    /// Exclude tracked puzzle hashes (addresses)
    #[arg(long)]
    pub exclude_addresses: bool,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// Comma-separated IDs (nft... / did:chia:...) OR a file path
    pub value: String,
}

#[derive(Args, Debug)]
pub struct SyncArgs {
    /// Size of batch when querying coin records
    #[arg(long, default_value_t = 3200)]
    pub batch_size: usize,
    /// Do not query for new puzzle hash coins before syncing database coins
    #[arg(long)]
    pub skip_puzzle_hash_sync: bool,
}

#[derive(Args, Debug)]
pub struct QueryArgs {
    /// NFT launcher id (nft1...)
    pub nft_id: String,
}
