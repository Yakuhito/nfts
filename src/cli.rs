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
    /// Only list tracked NFTs
    #[arg(long)]
    pub nfts_only: bool,
    /// Only list tracked DIDs
    #[arg(long)]
    pub dids_only: bool,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    #[command(subcommand)]
    pub kind: AddKind,
}

#[derive(Subcommand, Debug)]
pub enum AddKind {
    /// Add NFT launcher IDs to tracking
    Nft(AddInputArgs),
    /// Add DID launcher IDs to tracking
    Did(AddInputArgs),
}

#[derive(Args, Debug)]
pub struct AddInputArgs {
    /// Comma-separated launcher IDs
    #[arg(long)]
    pub ids: Option<String>,
    /// File containing launcher IDs (one per line)
    #[arg(long)]
    pub file: Option<PathBuf>,
}
