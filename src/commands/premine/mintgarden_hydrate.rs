//! Fill missing off-chain metadata using MintGarden's original-metadata bytes.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use chia_wallet_sdk::prelude::Bytes32;
use sqlx::SqlitePool;

use crate::cli::PremineMintgardenHydrateArgs;
use crate::commands::premine::generate::load_onchain_metadata_refs;
use crate::db;
use crate::error::CliError;
use crate::premine::{NAMESDAO_DID_LAUNCHER_HEX, Source, accept_metadata_bytes};
use crate::utils::encode_nft_launcher_id;

const MINTGARDEN_API: &str = "https://api.mintgarden.io";

pub async fn run(pool: &SqlitePool, args: PremineMintgardenHydrateArgs) -> Result<(), CliError> {
    let http = reqwest::Client::builder()
        .user_agent("nfts-premine-mintgarden-hydrate/0.1")
        .timeout(Duration::from_secs(25))
        .build()?;

    let namesdao_did = bytes32_from_hex(NAMESDAO_DID_LAUNCHER_HEX)?;
    let mut targets = load_missing_targets(pool, &namesdao_did).await?;

    if let Some(path) = &args.nfts_file {
        let filter = read_nft_id_filter(path)?;
        targets.retain(|t| filter.contains(&t.nft_id));
    }

    if targets.is_empty() {
        println!("No NFT metadata rows need MintGarden hydration.");
        return Ok(());
    }

    println!(
        "Hydrating {} NFT(s) from MintGarden /nfts/{{id}}/metadata (concurrency {})...",
        targets.len(),
        args.concurrency
    );

    let mut ok = 0usize;
    let mut failures = Vec::new();
    let concurrency = args.concurrency.max(1);

    for chunk in targets.chunks(concurrency) {
        let mut handles = Vec::with_capacity(chunk.len());
        for target in chunk {
            let http = http.clone();
            let target = target.clone();
            handles.push(tokio::spawn(async move {
                let result = fetch_and_verify(&http, &target).await;
                (target, result)
            }));
        }

        for handle in handles {
            let (target, result) = handle
                .await
                .map_err(|err| CliError::Message(format!("hydrate task join error: {err}")))?;
            match result {
                Ok(text) => {
                    db::update_offchain_metadata_value(pool, &target.metadata_hash, &text).await?;
                    ok += 1;
                    println!(
                        "  OK {} ({})",
                        target.nft_id,
                        target.source.as_str()
                    );
                }
                Err(reason) => {
                    println!(
                        "  FAIL {} ({}) — {reason}",
                        target.nft_id,
                        target.source.as_str()
                    );
                    failures.push((target.nft_id, reason));
                }
            }
        }
    }

    println!(
        "MintGarden hydrate complete: ok={ok} failed={}",
        failures.len()
    );
    if failures.is_empty() {
        Ok(())
    } else {
        Err(CliError::Message(format!(
            "mintgarden-hydrate failed for {} NFT(s)",
            failures.len()
        )))
    }
}

#[derive(Debug, Clone)]
struct HydrateTarget {
    source: Source,
    nft_id: String,
    metadata_hash: Bytes32,
    urls: Vec<String>,
}

async fn load_missing_targets(
    pool: &SqlitePool,
    namesdao_did: &Bytes32,
) -> Result<Vec<HydrateTarget>, CliError> {
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT launcher_id, did_launcher_id
        FROM coins
        WHERE type = 'NFT'
          AND launcher_id IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut out = Vec::new();
    let mut seen_hash = HashSet::new();

    for row in rows {
        use sqlx::Row;
        let launcher_id =
            crate::utils::bytes32_from_db("launcher_id", row.get::<&[u8], _>("launcher_id"))?;
        let did = crate::utils::optional_bytes32_from_db(
            "did_launcher_id",
            row.get::<Option<&[u8]>, _>("did_launcher_id"),
        )?;
        let source = match did {
            Some(d) if d == *namesdao_did => Source::NamesDao,
            Some(_) => continue,
            None => Source::Cns,
        };

        let Some((metadata_hash, urls)) =
            load_onchain_metadata_refs(pool, &launcher_id).await?
        else {
            continue;
        };

        let offchain = sqlx::query(
            r#"
            SELECT metadata_hash, urls, value, next_retry, retries_so_far
            FROM offchain_metadata
            WHERE metadata_hash = ?1
            "#,
        )
        .bind(metadata_hash.to_vec())
        .fetch_optional(pool)
        .await?;

        let needs = match &offchain {
            None => {
                ensure_offchain_row(pool, &metadata_hash, &urls).await?;
                true
            }
            Some(r) => db::row_to_offchain_metadata(r)?.value.is_none(),
        };
        if !needs {
            continue;
        }
        if !seen_hash.insert(metadata_hash.to_vec()) {
            continue;
        }

        out.push(HydrateTarget {
            source,
            nft_id: encode_nft_launcher_id(&launcher_id)?,
            metadata_hash,
            urls,
        });
    }

    out.sort_by(|a, b| a.nft_id.cmp(&b.nft_id));
    Ok(out)
}

async fn ensure_offchain_row(
    pool: &SqlitePool,
    metadata_hash: &Bytes32,
    urls: &[String],
) -> Result<(), CliError> {
    sqlx::query(
        r#"
        INSERT INTO offchain_metadata (metadata_hash, urls, value, next_retry, retries_so_far)
        VALUES (?1, ?2, NULL, NULL, 0)
        ON CONFLICT(metadata_hash) DO NOTHING
        "#,
    )
    .bind(metadata_hash.to_vec())
    .bind(sqlx::types::Json(urls.to_vec()))
    .execute(pool)
    .await?;
    Ok(())
}

async fn fetch_and_verify(
    http: &reqwest::Client,
    target: &HydrateTarget,
) -> Result<String, String> {
    let mut expected = [0u8; 32];
    expected.copy_from_slice(target.metadata_hash.as_ref());

    // 1) MintGarden original-metadata endpoint (preferred).
    let mg_url = format!("{MINTGARDEN_API}/nfts/{}/metadata", target.nft_id);
    match http.get(&mg_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(bytes) = resp.bytes().await
                && let Some(text) = accept_metadata_bytes(&expected, &bytes)
            {
                return Ok(text);
            }
        }
        Ok(resp) => {
            // fall through to URL/gateway attempts after noting status
            let _ = resp.status();
        }
        Err(_) => {}
    }

    // 2) On-chain metadata URLs, plus public IPFS gateways for any /ipfs/{cid} path.
    let mut candidates = target.urls.clone();
    for url in &target.urls {
        if let Some(cid) = ipfs_cid(url) {
            for gateway in [
                "https://ipfs.io/ipfs/",
                "https://dweb.link/ipfs/",
                "https://nftstorage.link/ipfs/",
                "https://w3s.link/ipfs/",
            ] {
                candidates.push(format!("{gateway}{cid}"));
            }
        }
    }

    let mut last_err = "no candidate URL succeeded".to_string();
    for url in candidates {
        match http.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(bytes) => {
                    if let Some(text) = accept_metadata_bytes(&expected, &bytes) {
                        return Ok(text);
                    }
                    last_err = format!("hash mismatch from {url}");
                }
                Err(err) => last_err = format!("body error from {url}: {err}"),
            },
            Ok(resp) => last_err = format!("HTTP {} from {url}", resp.status()),
            Err(err) => last_err = format!("request error from {url}: {err}"),
        }
    }

    Err(last_err)
}

fn ipfs_cid(url: &str) -> Option<&str> {
    let idx = url.find("/ipfs/")?;
    let rest = &url[idx + "/ipfs/".len()..];
    let cid = rest.split(['?', '#', '/']).next()?;
    if cid.is_empty() {
        None
    } else {
        Some(cid)
    }
}

fn read_nft_id_filter(path: &PathBuf) -> Result<HashSet<String>, CliError> {
    let text = std::fs::read_to_string(path)?;
    let mut out = HashSet::new();
    for line in text.lines() {
        let id = line.trim();
        if id.is_empty() || id.starts_with('#') {
            continue;
        }
        if !id.starts_with("nft1") {
            return Err(CliError::Message(format!(
                "invalid nft id in {}: {id}",
                path.display()
            )));
        }
        out.insert(id.to_string());
    }
    Ok(out)
}

fn bytes32_from_hex(hex_str: &str) -> Result<Bytes32, CliError> {
    let normalized = hex_str.trim().trim_start_matches("0x");
    let bytes =
        hex::decode(normalized).map_err(|err| CliError::Message(format!("invalid hex: {err}")))?;
    if bytes.len() != 32 {
        return Err(CliError::Message(format!(
            "expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(Bytes32::new(arr))
}
