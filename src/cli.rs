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
