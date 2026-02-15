use std::fmt::{Display, Formatter};
use std::path::PathBuf;

use bech32::{self, Bech32m, Hrp};
use chia_wallet_sdk::prelude::Bytes32;
use clap::{Args, Parser, Subcommand};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{FromRow, SqlitePool};

#[derive(Parser, Debug)]
#[command(name = "nfts")]
#[command(about = "Track NFTs and DIDs", version)]
struct Cli {
    /// SQLite database path
    #[arg(long, default_value = "nfts.db")]
    db: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List currently tracked items
    List(ListArgs),
    /// Add launcher ids to tracking
    Add(AddArgs),
}

#[derive(Args, Debug)]
struct ListArgs {
    /// Only list tracked NFTs
    #[arg(long)]
    nfts_only: bool,
    /// Only list tracked DIDs
    #[arg(long)]
    dids_only: bool,
}

#[derive(Args, Debug)]
struct AddArgs {
    #[command(subcommand)]
    kind: AddKind,
}

#[derive(Subcommand, Debug)]
enum AddKind {
    /// Add NFT launcher IDs to tracking
    Nft(AddInputArgs),
    /// Add DID launcher IDs to tracking
    Did(AddInputArgs),
}

#[derive(Args, Debug)]
struct AddInputArgs {
    /// Comma-separated launcher IDs
    #[arg(long)]
    ids: Option<String>,
    /// File containing launcher IDs (one per line)
    #[arg(long)]
    file: Option<PathBuf>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
struct Did {
    launcher_id: Vec<u8>,
    parent_coin_id: Option<Vec<u8>>,
    coin_id: Option<Vec<u8>>,
    spent_height: Option<i64>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
struct IntermediaryCoin {
    parent_coin_id: Vec<u8>,
    coin_id: Vec<u8>,
    spent_height: Option<i64>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
struct Nft {
    launcher_id: Vec<u8>,
    did_launcher_id: Option<Vec<u8>>,
    parent_coin_id: Option<Vec<u8>>,
    coin_id: Option<Vec<u8>>,
    inner_puzzle_hash: Option<Vec<u8>>,
    spent_height: Option<i64>,
}

#[derive(Debug)]
enum CliError {
    Message(String),
    Sqlx(sqlx::Error),
    Io(std::io::Error),
    Bech32(bech32::DecodeError),
    Bech32Encode(bech32::EncodeError),
    Hrp(bech32::primitives::hrp::Error),
}

impl Display for CliError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => write!(f, "{message}"),
            Self::Sqlx(err) => write!(f, "database error: {err}"),
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Bech32(err) => write!(f, "bech32 decode error: {err}"),
            Self::Bech32Encode(err) => write!(f, "bech32 encode error: {err}"),
            Self::Hrp(err) => write!(f, "bech32 hrp error: {err}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<sqlx::Error> for CliError {
    fn from(value: sqlx::Error) -> Self {
        Self::Sqlx(value)
    }
}

impl From<std::io::Error> for CliError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<bech32::DecodeError> for CliError {
    fn from(value: bech32::DecodeError) -> Self {
        Self::Bech32(value)
    }
}

impl From<bech32::EncodeError> for CliError {
    fn from(value: bech32::EncodeError) -> Self {
        Self::Bech32Encode(value)
    }
}

impl From<bech32::primitives::hrp::Error> for CliError {
    fn from(value: bech32::primitives::hrp::Error) -> Self {
        Self::Hrp(value)
    }
}

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

    ensure_schema(&pool).await?;

    match cli.command {
        Command::List(args) => list_tracked(&pool, args).await?,
        Command::Add(args) => add_tracked(&pool, args).await?,
    }

    Ok(())
}

async fn ensure_schema(pool: &SqlitePool) -> Result<(), CliError> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS dids (
            launcher_id BLOB PRIMARY KEY NOT NULL,
            parent_coin_id BLOB NULL,
            coin_id BLOB NULL,
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
            parent_coin_id BLOB NULL,
            coin_id BLOB NULL,
            inner_puzzle_hash BLOB NULL,
            spent_height INTEGER NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn list_tracked(pool: &SqlitePool, args: ListArgs) -> Result<(), CliError> {
    if args.nfts_only && args.dids_only {
        return Err(CliError::Message(
            "cannot combine --nfts-only and --dids-only".to_string(),
        ));
    }

    if !args.nfts_only {
        let dids: Vec<Did> = sqlx::query_as(
            r#"
            SELECT launcher_id, parent_coin_id, coin_id, spent_height
            FROM dids
            WHERE spent_height IS NULL
            ORDER BY launcher_id
            "#,
        )
        .fetch_all(pool)
        .await?;

        println!("Tracked DIDs (unspent): {}", dids.len());
        for did in dids {
            let launcher_id = bytes32_from_db("did launcher_id", &did.launcher_id)?;
            let launcher_string = encode_launcher_bech32m(&launcher_id, "did:chia:")?;
            println!("- {launcher_string}");
        }
    }

    if !args.dids_only {
        let nfts: Vec<Nft> = sqlx::query_as(
            r#"
            SELECT launcher_id, did_launcher_id, parent_coin_id, coin_id, inner_puzzle_hash, spent_height
            FROM nfts
            WHERE spent_height IS NULL
              AND did_launcher_id IS NOT NULL
            ORDER BY launcher_id
            "#,
        )
        .fetch_all(pool)
        .await?;

        println!("Tracked NFTs (unspent with DID): {}", nfts.len());
        for nft in nfts {
            let launcher_id = bytes32_from_db("nft launcher_id", &nft.launcher_id)?;
            let launcher_string = encode_launcher_bech32m(&launcher_id, "nft")?;
            println!("- {launcher_string}");
        }
    }

    Ok(())
}

async fn add_tracked(pool: &SqlitePool, args: AddArgs) -> Result<(), CliError> {
    match args.kind {
        AddKind::Nft(input) => {
            let ids = collect_input_ids(&input).await?;
            for raw_id in ids {
                let launcher_id = parse_launcher_id(&raw_id)?;
                let launcher_bytes = launcher_id.to_vec();

                // TODO(yakuhito): Resolve launcher_id through coinset/chia-wallet-sdk into
                // the full NFT object (coin lineage, inner_puzzle_hash, did linkage),
                // then insert/update the full record in the database.
                sqlx::query(
                    r#"
                    INSERT INTO nfts (launcher_id, did_launcher_id, parent_coin_id, coin_id, inner_puzzle_hash, spent_height)
                    VALUES (?1, NULL, NULL, NULL, NULL, NULL)
                    ON CONFLICT(launcher_id) DO NOTHING
                    "#,
                )
                .bind(launcher_bytes)
                .execute(pool)
                .await?;
            }
        }
        AddKind::Did(input) => {
            let ids = collect_input_ids(&input).await?;
            for raw_id in ids {
                let launcher_id = parse_launcher_id(&raw_id)?;
                let launcher_bytes = launcher_id.to_vec();

                // TODO(yakuhito): Resolve launcher_id through coinset/chia-wallet-sdk into
                // the full DID object (coin lineage and current spend state),
                // then insert/update the full record in the database.
                sqlx::query(
                    r#"
                    INSERT INTO dids (launcher_id, parent_coin_id, coin_id, spent_height)
                    VALUES (?1, NULL, NULL, NULL)
                    ON CONFLICT(launcher_id) DO NOTHING
                    "#,
                )
                .bind(launcher_bytes)
                .execute(pool)
                .await?;
            }
        }
    }

    Ok(())
}

async fn collect_input_ids(args: &AddInputArgs) -> Result<Vec<String>, CliError> {
    let mut ids = Vec::new();

    if let Some(csv_ids) = &args.ids {
        ids.extend(
            csv_ids
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(ToString::to_string),
        );
    }

    if let Some(file_path) = &args.file {
        let content = tokio::fs::read_to_string(file_path).await?;
        ids.extend(
            content
                .lines()
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(ToString::to_string),
        );
    }

    if ids.is_empty() {
        return Err(CliError::Message(
            "provide launcher IDs with --ids and/or --file".to_string(),
        ));
    }

    Ok(ids)
}

fn parse_launcher_id(input: &str) -> Result<Bytes32, CliError> {
    if input.starts_with("did:chia:") {
        // Accept full DID bech32m string (`did:chia:...`) first.
        if let Ok(value) = parse_bech32m_payload(input) {
            return Ok(value);
        }
        // Also accept `did:chia:` URI prefix followed by plain launcher id value.
        if let Some(stripped) = input.strip_prefix("did:chia:") {
            return parse_launcher_id(stripped);
        }
    }

    if input.starts_with("nft1") {
        return parse_bech32m_payload(input);
    }

    if input.len() == 64 && input.chars().all(|c| c.is_ascii_hexdigit()) {
        let bytes = decode_hex_32(input).ok_or_else(|| {
            CliError::Message(format!("invalid hex launcher id (expected 32 bytes): {input}"))
        })?;
        return Ok(Bytes32::new(bytes));
    }

    parse_bech32m_payload(input)
}

fn parse_bech32m_payload(value: &str) -> Result<Bytes32, CliError> {
    let (_hrp, bytes) = bech32::decode(value)?;
    if bytes.len() != 32 {
        return Err(CliError::Message(format!(
            "launcher id must decode to 32 bytes, got {} bytes: {value}",
            bytes.len()
        )));
    }
    let mut fixed = [0u8; 32];
    fixed.copy_from_slice(&bytes);
    Ok(Bytes32::new(fixed))
}

fn encode_launcher_bech32m(launcher_id: &Bytes32, hrp: &str) -> Result<String, CliError> {
    let hrp = Hrp::parse(hrp)?;
    Ok(bech32::encode::<Bech32m>(hrp, launcher_id.as_ref())?)
}

fn bytes32_from_db(field_name: &str, value: &[u8]) -> Result<Bytes32, CliError> {
    if value.len() != 32 {
        return Err(CliError::Message(format!(
            "{field_name} has invalid length {}, expected 32",
            value.len()
        )));
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(value);
    Ok(Bytes32::new(bytes))
}

fn decode_hex_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }

    let mut out = [0u8; 32];
    for (idx, slot) in out.iter_mut().enumerate() {
        let start = idx * 2;
        let end = start + 2;
        let part = &hex[start..end];
        *slot = u8::from_str_radix(part, 16).ok()?;
    }
    Some(out)
}
