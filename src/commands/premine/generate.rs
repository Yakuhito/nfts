use std::collections::{HashMap, HashSet};

use chia_wallet_sdk::chia::puzzle_types::singleton::LauncherSolution;
use chia_wallet_sdk::driver::{Nft, Puzzle, SpendContext};
use chia_wallet_sdk::prelude::{Bytes32, ChiaRpcClient, Coin as SdkCoin, CoinsetClient};
use chia_wallet_sdk::puzzles::SINGLETON_LAUNCHER_HASH;
use clvmr::NodePtr;
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::cli::PremineGenerateArgs;
use crate::db;
use crate::error::CliError;
use crate::models::{Coin, MetadataValue, OffchainMetadata, ParsedMetadata};
use crate::premine::{
    LegacyCandidate, MIGRATION_CUTOFF_UNIX, NAMESDAO_DID_LAUNCHER_HEX, Source, WarningRow,
    assert_unique_handles, build_base_premine, classify_legacy_name, parse_cns_expiration,
    parse_namesdao_expiry_height, strip_xch_suffix, write_premine_csvs_atomic,
};
use crate::utils::{encode_nft_launcher_id, encode_puzzle_hash_address};

pub async fn run(pool: &SqlitePool, args: PremineGenerateArgs) -> Result<(), CliError> {
    let client = CoinsetClient::mainnet();

    let Some(status) = client.get_blockchain_state().await?.blockchain_state else {
        return Err(CliError::Message(
            "failed to get blockchain state".to_string(),
        ));
    };
    let peak_height = status.peak.height;
    let tip_unix = block_timestamp(&client, peak_height).await?;

    let (effective_cutoff_unix, effective_cutoff_height, pre_cutoff) = if tip_unix
        < MIGRATION_CUTOFF_UNIX
    {
        eprintln!(
            "WARNING: chain tip ({tip_unix}) precedes Migration Cutoff ({MIGRATION_CUTOFF_UNIX} / 2026-07-20 09:00:00 UTC)."
        );
        eprintln!(
            "WARNING: continuing against the latest available state for rehearsal only. Do NOT publish these outputs as the live Base Premine."
        );
        (tip_unix, peak_height, true)
    } else {
        let cutoff_height =
            find_height_at_or_before(&client, peak_height, MIGRATION_CUTOFF_UNIX).await?;
        (MIGRATION_CUTOFF_UNIX, cutoff_height, false)
    };

    println!(
        "Effective cutoff: unix={effective_cutoff_unix} height={effective_cutoff_height} pre_cutoff_rehearsal={pre_cutoff}"
    );

    let namesdao_did = bytes32_from_hex(NAMESDAO_DID_LAUNCHER_HEX)?;

    println!("Loading NFT launcher ids...");
    let launchers = load_launcher_ids(pool, &namesdao_did).await?;
    println!(
        "Found {} CNS and {} NamesDAO launchers",
        launchers.iter().filter(|l| l.source == Source::Cns).count(),
        launchers
            .iter()
            .filter(|l| l.source == Source::NamesDao)
            .count()
    );

    println!("Repairing missing on-chain metadata / inner puzzle hashes from parent spends...");
    repair_missing_nft_fields(pool, &client, &launchers, effective_cutoff_height).await?;

    println!("Hydrating off-chain metadata...");
    let http = reqwest::Client::builder()
        .user_agent("nfts-premine/0.1")
        .build()?;
    let hydration_failures = hydrate_all_metadata(pool, &http, &launchers).await?;
    if !hydration_failures.is_empty() {
        eprintln!(
            "FATAL: {} NFT(s) still missing required off-chain metadata after exhausting URLs:",
            hydration_failures.len()
        );
        for failure in &hydration_failures {
            eprintln!(
                "  {} ({}) urls=[{}]",
                failure.nft_id,
                failure.source.as_str(),
                failure.urls.join(", ")
            );
        }
        return Err(CliError::Message(format!(
            "metadata hydration failed for {} NFT(s); refusing to emit Base Premine",
            hydration_failures.len()
        )));
    }

    println!("Resolving mint coin ids for registration timestamps...");
    let mint_by_launcher = resolve_mint_coin_ids(pool, &launchers).await?;
    let mut mint_coin_ids: Vec<Bytes32> = mint_by_launcher.values().copied().collect();
    mint_coin_ids.sort();
    mint_coin_ids.dedup();

    println!(
        "Fetching {} mint coin records (batch 100, concurrency 8)...",
        mint_coin_ids.len()
    );
    let timestamps_by_coin = fetch_coin_timestamps_batched(&client, &mint_coin_ids).await?;

    let mut registration_by_launcher: HashMap<Bytes32, u64> = HashMap::new();
    let mut missing_timestamp = 0usize;
    for (launcher_id, coin_id) in &mint_by_launcher {
        match timestamps_by_coin.get(coin_id).copied() {
            Some(ts) if ts > 0 => {
                registration_by_launcher.insert(*launcher_id, ts);
            }
            _ => {
                missing_timestamp += 1;
            }
        }
    }
    if missing_timestamp > 0 {
        return Err(CliError::Message(format!(
            "missing coin-record timestamps for {missing_timestamp} mint coin(s); refusing to generate"
        )));
    }
    println!(
        "Resolved registration timestamps for {} launchers",
        registration_by_launcher.len()
    );

    println!("Building Legacy Registration candidates (concurrency 8)...");
    let registration_by_launcher = std::sync::Arc::new(registration_by_launcher);
    let mut candidates = Vec::new();
    let mut warnings = Vec::new();

    for (chunk_idx, chunk) in launchers.chunks(8).enumerate() {
        let mut handles = Vec::with_capacity(chunk.len());
        for launcher in chunk {
            let pool = pool.clone();
            let launcher = launcher.clone();
            let registration_by_launcher = registration_by_launcher.clone();
            let cutoff_height = effective_cutoff_height;
            let cutoff_unix = effective_cutoff_unix;
            handles.push(tokio::spawn(async move {
                let Some(&registration_time) =
                    registration_by_launcher.get(&launcher.launcher_id)
                else {
                    return Err(CliError::Message(format!(
                        "missing registration timestamp for {}",
                        launcher.nft_id
                    )));
                };
                let mut local_warnings = Vec::new();
                let built = build_candidate(
                    &pool,
                    &launcher,
                    cutoff_height,
                    cutoff_unix,
                    registration_time,
                    &mut local_warnings,
                )
                .await?;
                Ok::<_, CliError>((built, local_warnings))
            }));
        }

        for handle in handles {
            match handle.await {
                Ok(Ok((built, mut local_warnings))) => {
                    warnings.append(&mut local_warnings);
                    if let Some(candidate) = built {
                        candidates.push(candidate);
                    }
                }
                Ok(Err(err)) => return Err(err),
                Err(err) => {
                    return Err(CliError::Message(format!(
                        "candidate task join failed: {err}"
                    )));
                }
            }
        }

        if (chunk_idx + 1) % 50 == 0 || (chunk_idx + 1) * 8 >= launchers.len() {
            println!(
                "  candidate progress: {}/{} launchers, {} candidates, {} warnings...",
                ((chunk_idx + 1) * 8).min(launchers.len()),
                launchers.len(),
                candidates.len(),
                warnings.len()
            );
        }
    }

    println!(
        "Resolved {} candidates ({} warnings so far)",
        candidates.len(),
        warnings.len()
    );

    let mut rows = build_base_premine(&candidates, effective_cutoff_unix);
    assert_unique_handles(&rows).map_err(CliError::Message)?;

    // BTreeMap already sorted by handle; double-check lexical order.
    rows.sort_by(|a, b| a.handle.cmp(&b.handle));

    write_premine_csvs_atomic(&args.output, &args.warnings, &rows, &warnings)?;

    println!(
        "Wrote {} Base Premine rows to {} and {} warnings to {}",
        rows.len(),
        args.output.display(),
        warnings.len(),
        args.warnings.display()
    );
    if pre_cutoff {
        eprintln!(
            "WARNING: outputs are provisional (pre-Migration-Cutoff rehearsal). Do not publish."
        );
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct LauncherRef {
    source: Source,
    launcher_id: Bytes32,
    nft_id: String,
}

#[derive(Debug)]
struct HydrationFailure {
    source: Source,
    nft_id: String,
    urls: Vec<String>,
}

async fn load_launcher_ids(
    pool: &SqlitePool,
    namesdao_did: &Bytes32,
) -> Result<Vec<LauncherRef>, CliError> {
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
            Some(_) => continue, // unknown DID collection
            None => Source::Cns,
        };
        out.push(LauncherRef {
            source,
            launcher_id,
            nft_id: encode_nft_launcher_id(&launcher_id)?,
        });
    }
    out.sort_by(|a, b| a.nft_id.cmp(&b.nft_id));
    Ok(out)
}

async fn hydrate_all_metadata(
    pool: &SqlitePool,
    http: &reqwest::Client,
    launchers: &[LauncherRef],
) -> Result<Vec<HydrationFailure>, CliError> {
    #[derive(Clone)]
    struct Need {
        source: Source,
        nft_id: String,
        metadata_hash: Bytes32,
        urls: Vec<String>,
    }

    let mut needs = Vec::new();
    let mut missing_onchain = Vec::new();
    for launcher in launchers {
        let Some((metadata_hash, urls)) =
            load_onchain_metadata_refs(pool, &launcher.launcher_id).await?
        else {
            missing_onchain.push(HydrationFailure {
                source: launcher.source,
                nft_id: launcher.nft_id.clone(),
                urls: vec!["<no on-chain metadata hash/urls in database>".into()],
            });
            continue;
        };

        let offchain = fetch_offchain_row(pool, &metadata_hash).await?;
        if offchain.as_ref().and_then(|o| o.value.as_ref()).is_some() {
            continue;
        }

        let urls = if let Some(existing) = offchain {
            merge_urls(existing.urls, urls)
        } else {
            ensure_offchain_row(pool, &metadata_hash, &urls).await?;
            urls
        };

        needs.push(Need {
            source: launcher.source,
            nft_id: launcher.nft_id.clone(),
            metadata_hash,
            urls,
        });
    }

    println!(
        "  {} metadata blob(s) need download; fetching with concurrency 32...",
        needs.len()
    );

    let mut failures = missing_onchain;
    for (chunk_idx, chunk) in needs.chunks(32).enumerate() {
        let mut handles = Vec::with_capacity(chunk.len());
        for need in chunk {
            let http = http.clone();
            let need = need.clone();
            handles.push(tokio::spawn(async move {
                let mut hydrated_text = None;
                for url in &need.urls {
                    match http.get(url).send().await {
                        Ok(resp) if resp.status().is_success() => {
                            let Ok(bytes) = resp.bytes().await else {
                                continue;
                            };
                            let digest = Sha256::digest(&bytes);
                            if digest.as_slice() != need.metadata_hash.as_ref() {
                                continue;
                            }
                            let Ok(text) = std::str::from_utf8(&bytes) else {
                                continue;
                            };
                            if serde_json::from_str::<JsonValue>(text).is_err() {
                                continue;
                            }
                            hydrated_text = Some(text.to_string());
                            break;
                        }
                        _ => continue,
                    }
                }
                (need, hydrated_text)
            }));
        }

        for handle in handles {
            let (need, hydrated_text) = handle
                .await
                .map_err(|err| CliError::Message(format!("hydrate task join error: {err}")))?;
            match hydrated_text {
                Some(text) => {
                    db::update_offchain_metadata_value(pool, &need.metadata_hash, &text).await?;
                }
                None => failures.push(HydrationFailure {
                    source: need.source,
                    nft_id: need.nft_id,
                    urls: need.urls,
                }),
            }
        }

        if chunk_idx > 0 && chunk_idx % 10 == 0 {
            println!(
                "  hydrate progress: ~{}/{}...",
                (chunk_idx + 1) * 32,
                needs.len()
            );
        }
    }

    Ok(failures)
}

fn merge_urls(a: Vec<String>, b: Vec<String>) -> Vec<String> {
    let mut set = HashSet::new();
    let mut out = Vec::new();
    for url in a.into_iter().chain(b) {
        if !url.is_empty() && set.insert(url.clone()) {
            out.push(url);
        }
    }
    out
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

async fn fetch_offchain_row(
    pool: &SqlitePool,
    metadata_hash: &Bytes32,
) -> Result<Option<OffchainMetadata>, CliError> {
    let row = sqlx::query(
        r#"
        SELECT metadata_hash, urls, value, next_retry, retries_so_far
        FROM offchain_metadata
        WHERE metadata_hash = ?1
        "#,
    )
    .bind(metadata_hash.to_vec())
    .fetch_optional(pool)
    .await?;
    row.map(|r| db::row_to_offchain_metadata(&r)).transpose()
}

pub(crate) async fn load_onchain_metadata_refs(
    pool: &SqlitePool,
    launcher_id: &Bytes32,
) -> Result<Option<(Bytes32, Vec<String>)>, CliError> {
    let rows = sqlx::query(
        r#"
        SELECT metadata
        FROM coins
        WHERE type = 'NFT'
          AND launcher_id = ?1
          AND metadata IS NOT NULL
        ORDER BY created_height
        "#,
    )
    .bind(launcher_id.to_vec())
    .fetch_all(pool)
    .await?;

    for row in rows {
        use sqlx::Row;
        let metadata: sqlx::types::Json<JsonValue> = row.get("metadata");
        let parsed: ParsedMetadata = serde_json::from_value(metadata.0).map_err(|err| {
            CliError::Message(format!("failed to parse coin metadata JSON: {err}"))
        })?;
        if let Some(pair) = extract_mh_mu(&parsed)? {
            return Ok(Some(pair));
        }
    }
    Ok(None)
}

fn extract_mh_mu(metadata: &ParsedMetadata) -> Result<Option<(Bytes32, Vec<String>)>, CliError> {
    let Some(mh) = metadata.0.get("mh") else {
        return Ok(None);
    };
    let Some(mu) = metadata.0.get("mu") else {
        return Ok(None);
    };
    let hash = match mh {
        MetadataValue::Bytes32(h) => *h,
        MetadataValue::String(raw) => bytes32_from_hex(raw.trim_start_matches("0x"))?,
        _ => {
            return Err(CliError::Message("invalid mh in on-chain metadata".into()));
        }
    };
    let urls = match mu {
        MetadataValue::StringArray(urls) => urls.clone(),
        MetadataValue::String(url) => vec![url.clone()],
        _ => Vec::new(),
    };
    Ok(Some((hash, urls)))
}

async fn build_candidate(
    pool: &SqlitePool,
    launcher: &LauncherRef,
    cutoff_height: u32,
    cutoff_unix: u64,
    registration_time: u64,
    warnings: &mut Vec<WarningRow>,
) -> Result<Option<LegacyCandidate>, CliError> {
    let (metadata_hash, urls) =
        match load_onchain_metadata_refs(pool, &launcher.launcher_id).await? {
            Some(v) => v,
            None => {
                return Err(CliError::Message(format!(
                    "missing on-chain metadata for {}",
                    launcher.nft_id
                )));
            }
        };
    let offchain = fetch_offchain_row(pool, &metadata_hash)
        .await?
        .and_then(|o| o.value)
        .ok_or_else(|| {
            CliError::Message(format!(
                "missing hydrated off-chain metadata for {}",
                launcher.nft_id
            ))
        })?;
    let meta: JsonValue = serde_json::from_str(&offchain).map_err(|err| {
        CliError::Message(format!(
            "invalid off-chain JSON for {}: {err}",
            launcher.nft_id
        ))
    })?;

    let urls_joined = urls.join("|");
    let raw_name = meta
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(raw_name) = raw_name else {
        warnings.push(WarningRow {
            reason: "missing-name".into(),
            source: launcher.source.as_str().into(),
            nft_id: launcher.nft_id.clone(),
            original_name: String::new(),
            metadata_urls: urls_joined,
        });
        return Ok(None);
    };

    let original_name = strip_xch_suffix(raw_name);
    let Some((handle, kind)) = classify_legacy_name(&original_name) else {
        if original_name.starts_with('_') {
            warnings.push(WarningRow {
                reason: "leading-underscore".into(),
                source: launcher.source.as_str().into(),
                nft_id: launcher.nft_id.clone(),
                original_name: original_name.clone(),
                metadata_urls: urls_joined,
            });
        }
        return Ok(None);
    };

    let (legacy_expiration, missing_expiration) =
        extract_legacy_expiration(&meta, launcher.source)?;
    if missing_expiration {
        warnings.push(WarningRow {
            reason: "missing-expiration".into(),
            source: launcher.source.as_str().into(),
            nft_id: launcher.nft_id.clone(),
            original_name: original_name.clone(),
            metadata_urls: urls_joined.clone(),
        });
    }

    let recipient_ph = recipient_at_cutoff(pool, &launcher.launcher_id, cutoff_height).await?;
    let recipient = encode_puzzle_hash_address(&recipient_ph)?;

    // Activity is relative to the effective cutoff; keep legacy_expiration as metadata-derived.
    let _ = cutoff_unix;

    Ok(Some(LegacyCandidate {
        source: launcher.source,
        nft_id: launcher.nft_id.clone(),
        original_name,
        handle,
        kind,
        registration_time,
        legacy_expiration,
        recipient,
    }))
}

fn extract_legacy_expiration(meta: &JsonValue, source: Source) -> Result<(u64, bool), CliError> {
    let attrs = meta
        .get("attributes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    match source {
        Source::Cns => {
            let value = attrs.iter().find_map(|attr| {
                let trait_type = attr.get("trait_type")?.as_str()?;
                if trait_type == "Expiration" {
                    attr.get("value")?.as_str().map(|s| s.to_string())
                } else {
                    None
                }
            });
            match value {
                Some(date) => match parse_cns_expiration(&date) {
                    Some(ts) => Ok((ts, false)),
                    None => Ok((0, true)),
                },
                None => Ok((0, true)),
            }
        }
        Source::NamesDao => {
            let value = attrs.iter().find_map(|attr| {
                let trait_type = attr.get("trait_type")?.as_str()?;
                if trait_type != "Expiry" {
                    return None;
                }
                let v = attr.get("value")?;
                if let Some(n) = v.as_u64() {
                    Some(n)
                } else if let Some(s) = v.as_str() {
                    s.parse::<u64>().ok()
                } else {
                    None
                }
            });
            match value {
                Some(height) => Ok((parse_namesdao_expiry_height(height), false)),
                None => Ok((0, true)),
            }
        }
    }
}

/// Map each launcher to its eve NFT coin id (parent = launcher). The eve coin's
/// confirmation timestamp is Registration Time.
async fn resolve_mint_coin_ids(
    pool: &SqlitePool,
    launchers: &[LauncherRef],
) -> Result<HashMap<Bytes32, Bytes32>, CliError> {
    let launcher_hash: Bytes32 = SINGLETON_LAUNCHER_HASH.into();
    let rows = sqlx::query(
        r#"
        SELECT launcher_id, parent_coin_id, puzzle_hash, coin_id, created_height
        FROM coins
        WHERE type = 'NFT'
          AND launcher_id IS NOT NULL
        ORDER BY created_height, coin_id
        "#,
    )
    .fetch_all(pool)
    .await?;

    let wanted: HashSet<Bytes32> = launchers.iter().map(|l| l.launcher_id).collect();
    let mut eve_by_launcher: HashMap<Bytes32, (u32, Bytes32)> = HashMap::new();

    for row in rows {
        use sqlx::Row;
        let launcher_id =
            crate::utils::bytes32_from_db("launcher_id", row.get::<&[u8], _>("launcher_id"))?;
        if !wanted.contains(&launcher_id) {
            continue;
        }
        let puzzle_hash =
            crate::utils::bytes32_from_db("puzzle_hash", row.get::<&[u8], _>("puzzle_hash"))?;
        let coin_id = crate::utils::bytes32_from_db("coin_id", row.get::<&[u8], _>("coin_id"))?;
        let parent_coin_id =
            crate::utils::bytes32_from_db("parent_coin_id", row.get::<&[u8], _>("parent_coin_id"))?;
        if puzzle_hash == launcher_hash || coin_id == launcher_id {
            continue;
        }
        if parent_coin_id != launcher_id {
            continue;
        }
        let created = u32::try_from(row.get::<i64, _>("created_height")).map_err(|_| {
            CliError::Message(format!(
                "invalid created_height for coin 0x{}",
                hex::encode(coin_id)
            ))
        })?;

        match eve_by_launcher.get(&launcher_id) {
            Some((prev_h, _)) if *prev_h <= created => {}
            _ => {
                eve_by_launcher.insert(launcher_id, (created, coin_id));
            }
        }
    }

    let mut out = HashMap::new();
    let mut missing = Vec::new();
    for launcher in launchers {
        match eve_by_launcher.get(&launcher.launcher_id) {
            Some((_, coin_id)) => {
                out.insert(launcher.launcher_id, *coin_id);
            }
            None => missing.push(launcher.nft_id.clone()),
        }
    }
    if !missing.is_empty() {
        let sample: Vec<_> = missing.iter().take(5).cloned().collect();
        return Err(CliError::Message(format!(
            "missing eve NFT coin for {} launcher(s); sample: {}",
            missing.len(),
            sample.join(", ")
        )));
    }
    Ok(out)
}

async fn fetch_coin_timestamps_batched(
    client: &CoinsetClient,
    coin_ids: &[Bytes32],
) -> Result<HashMap<Bytes32, u64>, CliError> {
    let batches: Vec<Vec<Bytes32>> = coin_ids.chunks(100).map(|c| c.to_vec()).collect();
    let mut out: HashMap<Bytes32, u64> = HashMap::with_capacity(coin_ids.len());

    for (chunk_idx, chunk) in batches.chunks(8).enumerate() {
        let mut handles = Vec::with_capacity(chunk.len());
        for batch in chunk {
            let client = client.clone();
            let batch = batch.clone();
            handles.push(tokio::spawn(async move {
                let mut last_err = None;
                for attempt in 0..3 {
                    match client
                        .get_coin_records_by_names(batch.clone(), None, None, Some(true))
                        .await
                    {
                        Ok(resp) => {
                            let Some(records) = resp.coin_records else {
                                last_err = Some(CliError::Message(
                                    "get_coin_records_by_names returned no coin_records".into(),
                                ));
                                continue;
                            };
                            let mut map = HashMap::with_capacity(records.len());
                            for rec in records {
                                map.insert(rec.coin.coin_id(), rec.timestamp);
                            }
                            return Ok::<_, CliError>(map);
                        }
                        Err(err) => {
                            last_err = Some(err.into());
                            tokio::time::sleep(std::time::Duration::from_millis(
                                200 * (attempt + 1) as u64,
                            ))
                            .await;
                        }
                    }
                }
                Err(last_err.unwrap_or_else(|| {
                    CliError::Message("get_coin_records_by_names failed".into())
                }))
            }));
        }

        for handle in handles {
            match handle.await {
                Ok(Ok(map)) => out.extend(map),
                Ok(Err(err)) => return Err(err),
                Err(err) => {
                    return Err(CliError::Message(format!(
                        "coin-record batch join failed: {err}"
                    )));
                }
            }
        }

        let done = ((chunk_idx + 1) * 8 * 100).min(coin_ids.len());
        if (chunk_idx + 1) % 5 == 0 || done == coin_ids.len() {
            println!(
                "  coin-record progress: {done}/{} ids, {} timestamps...",
                coin_ids.len(),
                out.len()
            );
        }
    }

    Ok(out)
}

async fn repair_missing_nft_fields(
    pool: &SqlitePool,
    client: &CoinsetClient,
    launchers: &[LauncherRef],
    cutoff_height: u32,
) -> Result<(), CliError> {
    let launcher_hash: Bytes32 = SINGLETON_LAUNCHER_HASH.into();

    // Bulk-load non-launcher NFT coins once, then pick cutoff/fallback per launcher.
    let rows = sqlx::query(
        r#"
        SELECT type, launcher_id, did_launcher_id, parent_coin_id, puzzle_hash, coin_id,
               created_height, spent_height, metadata, inner_puzzle_hash
        FROM coins
        WHERE type = 'NFT'
          AND launcher_id IS NOT NULL
        ORDER BY created_height, coin_id
        "#,
    )
    .fetch_all(pool)
    .await?;
    let all_coins: Vec<Coin> = rows.iter().map(db::row_to_coin).collect::<Result<_, _>>()?;

    let mut by_launcher: HashMap<Bytes32, Vec<&Coin>> = HashMap::new();
    for coin in &all_coins {
        let Some(launcher_id) = coin.launcher_id else {
            continue;
        };
        if coin.puzzle_hash == launcher_hash {
            continue;
        }
        by_launcher.entry(launcher_id).or_default().push(coin);
    }

    let mut work: Vec<Coin> = Vec::new();
    let mut seen_coins = HashSet::new();
    for launcher in launchers {
        let Some(coins) = by_launcher.get(&launcher.launcher_id) else {
            continue;
        };
        let needs_meta = coins.iter().all(|c| c.metadata.is_none());
        let cutoff_coin = coins.iter().rev().find(|coin| {
            coin.created_height <= cutoff_height
                && coin.spent_height.is_none_or(|h| h > cutoff_height)
        });
        let Some(cutoff_coin) = cutoff_coin else {
            continue;
        };
        let needs_inner = cutoff_coin.inner_puzzle_hash.is_none();
        if !needs_meta && !needs_inner {
            continue;
        }

        if seen_coins.insert(cutoff_coin.coin_id) {
            work.push((*cutoff_coin).clone());
        }
        if needs_meta
            && let Some(fallback) = coins.iter().find(|c| c.coin_id != cutoff_coin.coin_id)
            && seen_coins.insert(fallback.coin_id)
        {
            work.push((*fallback).clone());
        }
    }

    println!(
        "  {} coin(s) need parent-spend repair; recovering with concurrency 8...",
        work.len()
    );

    let mut repaired = 0usize;
    for (chunk_idx, chunk) in work.chunks(8).enumerate() {
        let mut handles = Vec::with_capacity(chunk.len());
        for coin in chunk {
            let client = client.clone();
            let coin = coin.clone();
            handles.push(tokio::spawn(async move {
                let mut last_err = None;
                for attempt in 0..3 {
                    match recover_nft_fields_from_parent(&client, &coin).await {
                        Ok(fields) => return Ok((coin.coin_id, fields)),
                        Err(err) => {
                            last_err = Some(err);
                            tokio::time::sleep(std::time::Duration::from_millis(
                                200 * (attempt + 1) as u64,
                            ))
                            .await;
                        }
                    }
                }
                Err(last_err.unwrap_or_else(|| CliError::Message("repair failed".into())))
            }));
        }

        for handle in handles {
            if let Ok(Ok((coin_id, (metadata, inner)))) = handle.await {
                db::update_coin_nft_fields(pool, &coin_id, &metadata, &inner).await?;
                repaired += 1;
            }
        }

        if (chunk_idx + 1) % 20 == 0 || (chunk_idx + 1) * 8 >= work.len() {
            println!(
                "  repair progress: {}/{} coins attempted, {repaired} updated...",
                ((chunk_idx + 1) * 8).min(work.len()),
                work.len()
            );
        }
    }

    println!("  repair pass updated {repaired} coin row(s)");
    Ok(())
}

async fn recover_nft_fields_from_parent(
    client: &CoinsetClient,
    coin: &Coin,
) -> Result<(ParsedMetadata, Bytes32), CliError> {
    let launcher_hash: Bytes32 = SINGLETON_LAUNCHER_HASH.into();
    let Some(parent_spend) = client
        .get_puzzle_and_solution(coin.parent_coin_id, Some(coin.created_height))
        .await?
        .coin_solution
    else {
        return Err(CliError::Message(format!(
            "missing parent spend for coin 0x{}",
            hex::encode(coin.coin_id)
        )));
    };

    let ctx = &mut SpendContext::new();
    let puzzle = ctx.alloc(&parent_spend.puzzle_reveal)?;
    let solution = ctx.alloc(&parent_spend.solution)?;

    let nft = if parent_spend.coin.puzzle_hash == launcher_hash {
        let sol = ctx.extract::<LauncherSolution<NodePtr>>(solution)?;
        let eve = SdkCoin::new(
            parent_spend.coin.coin_id(),
            sol.singleton_puzzle_hash,
            sol.amount,
        );
        if eve.coin_id() != coin.coin_id {
            return Err(CliError::Message(
                "launcher eve coin id mismatch during repair".into(),
            ));
        }
        // Eve NFT must itself be spent to reveal its puzzle in the usual sync path.
        // Fall back to fetching this coin's own spend if spent; if unspent, parse via
        // get_puzzle_and_solution is unavailable — use child parse from a synthetic path.
        // For unspent eve coins, fetch the eve spend is impossible; instead parse the
        // launcher solution's singleton puzzle hash only gets the outer hash.
        // Fetch coin record; if spent, parse from its spend; if unspent, we need another way.
        let Some(record) = client
            .get_coin_record_by_name(coin.coin_id)
            .await?
            .coin_record
        else {
            return Err(CliError::Message(
                "missing coin record during repair".into(),
            ));
        };
        if !record.spent {
            return Err(CliError::Message(
                "cannot recover fields for unspent eve NFT without child spend".into(),
            ));
        }
        let Some(eve_spend) = client
            .get_puzzle_and_solution(coin.coin_id, Some(record.spent_block_index))
            .await?
            .coin_solution
        else {
            return Err(CliError::Message("missing eve spend during repair".into()));
        };
        let eve_puzzle = ctx.alloc(&eve_spend.puzzle_reveal)?;
        let eve_solution = ctx.alloc(&eve_spend.solution)?;
        let eve_puzzle = Puzzle::parse(ctx, eve_puzzle);
        let Some((nft, _, _)) = Nft::parse(ctx, eve, eve_puzzle, eve_solution)? else {
            return Err(CliError::Message("failed to parse eve NFT".into()));
        };
        nft
    } else {
        let puzzle = Puzzle::parse(ctx, puzzle);
        let Some(nft) = Nft::parse_child(ctx, parent_spend.coin, puzzle, solution)? else {
            return Err(CliError::Message(format!(
                "failed to parse child NFT for coin 0x{}",
                hex::encode(coin.coin_id)
            )));
        };
        nft
    };

    let metadata = ctx.extract::<ParsedMetadata>(nft.info.metadata.ptr())?;
    Ok((metadata, nft.info.p2_puzzle_hash))
}

async fn load_nft_coins(pool: &SqlitePool, launcher_id: &Bytes32) -> Result<Vec<Coin>, CliError> {
    let rows = sqlx::query(
        r#"
        SELECT type, launcher_id, did_launcher_id, parent_coin_id, puzzle_hash, coin_id,
               created_height, spent_height, metadata, inner_puzzle_hash
        FROM coins
        WHERE type = 'NFT'
          AND launcher_id = ?1
        ORDER BY created_height, coin_id
        "#,
    )
    .bind(launcher_id.to_vec())
    .fetch_all(pool)
    .await?;
    rows.iter().map(db::row_to_coin).collect()
}

async fn coin_at_cutoff(
    pool: &SqlitePool,
    launcher_id: &Bytes32,
    cutoff_height: u32,
) -> Result<Option<Coin>, CliError> {
    let launcher_hash: Bytes32 = SINGLETON_LAUNCHER_HASH.into();
    let coins = load_nft_coins(pool, launcher_id).await?;
    let mut best = None;
    for coin in coins {
        if coin.puzzle_hash == launcher_hash {
            continue;
        }
        if coin.created_height > cutoff_height {
            continue;
        }
        if coin.spent_height.is_some_and(|h| h <= cutoff_height) {
            continue;
        }
        best = Some(coin);
    }
    Ok(best)
}

async fn recipient_at_cutoff(
    pool: &SqlitePool,
    launcher_id: &Bytes32,
    cutoff_height: u32,
) -> Result<Bytes32, CliError> {
    let coin = coin_at_cutoff(pool, launcher_id, cutoff_height)
        .await?
        .ok_or_else(|| {
            CliError::Message(format!(
                "no NFT coin present at cutoff for 0x{}",
                hex::encode(launcher_id)
            ))
        })?;

    coin.inner_puzzle_hash.ok_or_else(|| {
        CliError::Message(format!(
            "missing inner_puzzle_hash at cutoff for 0x{} (repair/re-sync required)",
            hex::encode(launcher_id)
        ))
    })
}

async fn block_timestamp(client: &CoinsetClient, height: u32) -> Result<u64, CliError> {
    // Non-transaction blocks have no timestamp; walk backward to the nearest tx block.
    let mut h = height;
    loop {
        let Some(block) = client.get_block_record_by_height(h).await?.block_record else {
            return Err(CliError::Message(format!(
                "missing block record for height {h}"
            )));
        };
        if let Some(ts) = block.timestamp {
            return Ok(ts);
        }
        let prev = block.prev_transaction_block_height;
        if prev < h {
            h = prev;
            continue;
        }
        if h == 0 {
            return Err(CliError::Message(format!(
                "no transaction-block timestamp at or before height {height}"
            )));
        }
        h -= 1;
    }
}

async fn find_height_at_or_before(
    client: &CoinsetClient,
    peak_height: u32,
    target_unix: u64,
) -> Result<u32, CliError> {
    let mut lo = 0u32;
    let mut hi = peak_height;
    let mut best = 0u32;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let ts = match block_timestamp(client, mid).await {
            Ok(ts) => ts,
            Err(_) => {
                let mut found = None;
                for h in (mid.saturating_sub(32)..=mid).rev() {
                    if let Ok(ts) = block_timestamp(client, h).await {
                        found = Some((h, ts));
                        break;
                    }
                }
                match found {
                    Some((h, ts)) => {
                        if ts <= target_unix {
                            best = h;
                            lo = mid.saturating_add(1);
                        } else {
                            if mid == 0 {
                                break;
                            }
                            hi = mid - 1;
                        }
                        continue;
                    }
                    None => {
                        if mid == 0 {
                            break;
                        }
                        hi = mid - 1;
                        continue;
                    }
                }
            }
        };
        if ts <= target_unix {
            best = mid;
            lo = mid.saturating_add(1);
        } else {
            if mid == 0 {
                break;
            }
            hi = mid - 1;
        }
    }
    Ok(best)
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
