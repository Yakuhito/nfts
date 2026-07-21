use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::cli::PremineConfirmArgs;
use crate::error::CliError;
use crate::premine::csv::parse_premine_csv;
use crate::premine::handle::is_valid_handle;
use crate::premine::{
    DEAD_ADDRESS, LegacyCandidate, MIGRATION_CUTOFF_UNIX, MINTGARDEN_CNS_COLLECTION,
    MINTGARDEN_NAMESDAO_COLLECTION, PremineRow, Source, assert_unique_handles, build_base_premine,
    classify_legacy_name, parse_cns_expiration, parse_namesdao_expiry_height, strip_xch_suffix,
};
use chia_wallet_sdk::utils::Address;

const MINTGARDEN_API: &str = "https://api.mintgarden.io";

pub async fn run(args: PremineConfirmArgs) -> Result<(), CliError> {
    let contents = tokio::fs::read_to_string(&args.input).await?;
    let input_mtime = std::fs::metadata(&args.input)?.modified().ok();
    let actual_all = parse_premine_csv(&contents)?;

    let ignored_handles: HashSet<String> = actual_all
        .iter()
        .filter(|row| row.recipient == DEAD_ADDRESS)
        .map(|row| row.handle.clone())
        .collect();
    if !ignored_handles.is_empty() {
        println!(
            "Ignoring {} burn-recipient handle(s) (DEAD_ADDRESS; dropped from published premine).",
            ignored_handles.len()
        );
    }
    let actual: Vec<PremineRow> = actual_all
        .into_iter()
        .filter(|row| !ignored_handles.contains(&row.handle))
        .collect();

    let http = reqwest::Client::builder()
        .user_agent("nfts-premine-confirm/0.1")
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    // Prefer wall-clock for cutoff mode: collection event feeds can lag or be inactive.
    let now_unix = Utc::now().timestamp().max(0) as u64;
    let tip_unix = latest_known_event_unix(&http).await.unwrap_or(0);
    let (effective_cutoff_unix, pre_cutoff) = if now_unix < MIGRATION_CUTOFF_UNIX {
        let tip = tip_unix.max(now_unix);
        eprintln!(
            "WARNING: wall clock ({now_unix}) precedes Migration Cutoff ({MIGRATION_CUTOFF_UNIX})."
        );
        eprintln!(
            "WARNING: confirming against the latest externally available state (rehearsal tip≈{tip}). This is NOT final Migration Cutoff confirmation."
        );
        (tip, true)
    } else {
        if tip_unix > 0 && tip_unix < MIGRATION_CUTOFF_UNIX {
            eprintln!(
                "NOTE: MintGarden latest collection event ({tip_unix}) lags Migration Cutoff; using cutoff {MIGRATION_CUTOFF_UNIX} because wall clock is past it."
            );
        }
        if tip_unix > MIGRATION_CUTOFF_UNIX {
            eprintln!(
                "NOTE: MintGarden tip ({tip_unix}) is after Migration Cutoff ({MIGRATION_CUTOFF_UNIX}); recipients use MintGarden current owners (post-cutoff transfers would mismatch)."
            );
        }
        (MIGRATION_CUTOFF_UNIX, false)
    };

    println!("Fetching CNS collection ids from MintGarden...");
    let mut cns_ids = fetch_collection_nft_ids(&http, MINTGARDEN_CNS_COLLECTION).await?;
    println!("  {} CNS NFT ids", cns_ids.len());

    println!("Fetching NamesDAO collection ids from MintGarden...");
    let mut namesdao_ids = fetch_collection_nft_ids(&http, MINTGARDEN_NAMESDAO_COLLECTION).await?;
    println!("  {} NamesDAO NFT ids", namesdao_ids.len());

    // Backfill any non-burn CSV NFT ids missing from collection indexes.
    let mut backfilled = 0usize;
    for row in &actual {
        let Some(nft_id) = nft_id_from_mintgarden_url(&row.allocation_explanation) else {
            continue;
        };
        let bucket = match row.allocation_type.as_str() {
            "cns" => &mut cns_ids,
            "namesdao" => &mut namesdao_ids,
            _ => continue,
        };
        if bucket.insert(nft_id) {
            backfilled += 1;
        }
    }
    if backfilled > 0 {
        println!("  Backfilled {backfilled} CSV NFT id(s) missing from collection indexes.");
    }

    let concurrency = args.concurrency.max(1);
    println!(
        "Fetching NFT details from MintGarden ({} CNS + {} NamesDAO, concurrency {concurrency})...",
        cns_ids.len(),
        namesdao_ids.len()
    );
    let cns_details = fetch_nft_details(&http, &cns_ids, concurrency).await?;
    let namesdao_details = fetch_nft_details(&http, &namesdao_ids, concurrency).await?;
    println!(
        "  Got {} CNS + {} NamesDAO details",
        cns_details.len(),
        namesdao_details.len()
    );

    let mut candidates = Vec::new();
    for nft in &cns_details {
        if let Some(c) = detail_to_candidate(Source::Cns, nft)? {
            candidates.push(c);
        }
    }
    for nft in &namesdao_details {
        if let Some(c) = detail_to_candidate(Source::NamesDao, nft)? {
            candidates.push(c);
        }
    }
    println!("  {} viable Legacy Candidates after classification", candidates.len());

    let expected_all = build_base_premine(&candidates, effective_cutoff_unix);
    let expected: Vec<PremineRow> = expected_all
        .into_iter()
        .filter(|row| {
            row.recipient != DEAD_ADDRESS && !ignored_handles.contains(&row.handle)
        })
        .collect();

    let mut errors = Vec::new();

    // Validate actual CSV independently.
    validate_actual_rows(&actual, &mut errors);

    let expected_map: HashMap<&str, &PremineRow> =
        expected.iter().map(|r| (r.handle.as_str(), r)).collect();
    let actual_map: HashMap<&str, &PremineRow> =
        actual.iter().map(|r| (r.handle.as_str(), r)).collect();

    for (handle, actual_row) in &actual_map {
        match expected_map.get(handle) {
            None => errors.push(format!(
                "unsupported row: handle={handle} recipient={} expiration={} allocation_type={} allocation_explanation={}",
                actual_row.recipient,
                actual_row.expiration,
                actual_row.allocation_type,
                actual_row.allocation_explanation
            )),
            Some(expected_row) => {
                if actual_row.recipient != expected_row.recipient
                    || actual_row.expiration != expected_row.expiration
                    || actual_row.allocation_type != expected_row.allocation_type
                    || actual_row.allocation_explanation != expected_row.allocation_explanation
                {
                    errors.push(format!(
                        "incorrect row for {handle}: actual recipient={} expiration={} allocation_type={} allocation_explanation={}; expected recipient={} expiration={} allocation_type={} allocation_explanation={}",
                        actual_row.recipient,
                        actual_row.expiration,
                        actual_row.allocation_type,
                        actual_row.allocation_explanation,
                        expected_row.recipient,
                        expected_row.expiration,
                        expected_row.allocation_type,
                        expected_row.allocation_explanation
                    ));
                }
            }
        }
    }

    for (handle, expected_row) in &expected_map {
        if !actual_map.contains_key(handle) {
            errors.push(format!(
                "missing expected row: handle={handle} recipient={} expiration={} allocation_type={} allocation_explanation={}",
                expected_row.recipient,
                expected_row.expiration,
                expected_row.allocation_type,
                expected_row.allocation_explanation
            ));
        }
    }

    // Ensure input file was not modified.
    if let (Some(before), Ok(meta)) = (input_mtime, std::fs::metadata(&args.input))
        && let Ok(after) = meta.modified()
        && after != before
    {
        errors.push("input Base Premine CSV was modified during confirmation".into());
    }

    if errors.is_empty() {
        println!(
            "OK: Base Premine matches MintGarden reconstruction ({} rows; {} burn handle(s) ignored). pre_cutoff_rehearsal={pre_cutoff}",
            expected.len(),
            ignored_handles.len()
        );
        Ok(())
    } else {
        eprintln!("CONFIRMATION FAILED with {} issue(s):", errors.len());
        for err in &errors {
            eprintln!("  - {err}");
        }
        Err(CliError::Message(format!(
            "premine confirm failed with {} mismatch(es)",
            errors.len()
        )))
    }
}

fn validate_actual_rows(rows: &[PremineRow], errors: &mut Vec<String>) {
    if let Err(err) = assert_unique_handles(rows) {
        errors.push(err);
    }
    for row in rows {
        if !is_valid_handle(&row.handle) {
            errors.push(format!("invalid handle grammar: {}", row.handle));
        }
        if let Err(err) = Address::decode(&row.recipient) {
            errors.push(format!(
                "invalid recipient address for {}: {err}",
                row.handle
            ));
        } else if !row.recipient.starts_with("xch1") {
            errors.push(format!(
                "recipient for {} must be an xch1 address",
                row.handle
            ));
        }
        if row.allocation_type != Source::Cns.as_str()
            && row.allocation_type != Source::NamesDao.as_str()
        {
            errors.push(format!(
                "invalid allocation_type for {}: {:?}; expected cns or namesdao",
                row.handle, row.allocation_type
            ));
        }
        if !is_mintgarden_nft_url(&row.allocation_explanation) {
            errors.push(format!(
                "invalid allocation_explanation for {}: {:?}; expected https://mintgarden.io/nfts/{{nft_id}}",
                row.handle, row.allocation_explanation
            ));
        }
    }
}

fn is_mintgarden_nft_url(value: &str) -> bool {
    nft_id_from_mintgarden_url(value).is_some()
}

fn nft_id_from_mintgarden_url(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    if url.host_str() != Some("mintgarden.io") {
        return None;
    }
    let mut segments = url.path_segments()?;
    match (segments.next(), segments.next(), segments.next()) {
        (Some("nfts"), Some(nft_id), None) if nft_id.starts_with("nft1") && !nft_id.is_empty() => {
            Some(nft_id.to_string())
        }
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct Page<T> {
    items: Vec<T>,
    #[allow(dead_code)]
    next: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct MgIdItem {
    encoded_id: String,
}

#[derive(Debug, Deserialize, Clone)]
struct MgEvent {
    #[serde(rename = "type")]
    event_type: i32,
    timestamp: String,
}

#[derive(Debug, Deserialize, Clone)]
struct MgAddress {
    encoded_id: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct MgNftDetail {
    #[allow(dead_code)]
    id: String,
    encoded_id: String,
    collection: Option<MgCollection>,
    owner_address: Option<MgAddress>,
    data: Option<MgNftData>,
    events: Option<Vec<MgEvent>>,
    minted_at: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct MgCollection {
    id: String,
}

#[derive(Debug, Deserialize, Clone)]
struct MgNftData {
    metadata_json: Option<JsonValue>,
}

async fn fetch_collection_nft_ids(
    http: &reqwest::Client,
    collection_id: &str,
) -> Result<HashSet<String>, CliError> {
    let url = format!("{MINTGARDEN_API}/collections/{collection_id}/nfts/ids");
    let items: Vec<MgIdItem> = get_json_with_retries(http, &url).await?;
    Ok(items.into_iter().map(|item| item.encoded_id).collect())
}

async fn fetch_nft_details(
    http: &reqwest::Client,
    ids: &HashSet<String>,
    concurrency: usize,
) -> Result<Vec<MgNftDetail>, CliError> {
    let mut ids: Vec<String> = ids.iter().cloned().collect();
    ids.sort();
    let http = http.clone();
    let mut out = Vec::with_capacity(ids.len());
    let mut failed = 0usize;

    for (chunk_idx, chunk) in ids.chunks(concurrency).enumerate() {
        let mut handles = Vec::with_capacity(chunk.len());
        for nft_id in chunk {
            let http = http.clone();
            let nft_id = nft_id.clone();
            handles.push(tokio::spawn(async move {
                let url = format!("{MINTGARDEN_API}/nfts/{nft_id}");
                match get_json_with_retries(&http, &url).await {
                    Ok(detail) => Ok(detail),
                    Err(err) => Err((nft_id, err)),
                }
            }));
        }

        for handle in handles {
            match handle
                .await
                .map_err(|err| CliError::Message(format!("confirm task join error: {err}")))?
            {
                Ok(detail) => out.push(detail),
                Err((nft_id, err)) => {
                    failed += 1;
                    eprintln!("  WARN: failed to fetch {nft_id}: {err}");
                }
            }
        }

        if (chunk_idx + 1) % 25 == 0 || chunk_idx + 1 == ids.len().div_ceil(concurrency) {
            println!(
                "  … details {}/{} (failed {failed})",
                out.len() + failed,
                ids.len()
            );
        }
    }

    if failed > 0 {
        return Err(CliError::Message(format!(
            "failed to fetch {failed} MintGarden NFT detail(s)"
        )));
    }
    Ok(out)
}

async fn get_json_with_retries<T: for<'de> Deserialize<'de>>(
    http: &reqwest::Client,
    url: &str,
) -> Result<T, CliError> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match http.get(url).send().await {
            Ok(resp) => {
                if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt <= 10 {
                    let secs = 45u64;
                    eprintln!("  MintGarden 429 on fetch; sleeping {secs}s (attempt {attempt}/10)...");
                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                    continue;
                }
                if resp.status().is_server_error() && attempt <= 10 {
                    let secs = 30u64;
                    eprintln!(
                        "  MintGarden {} on fetch; sleeping {secs}s (attempt {attempt}/10)...",
                        resp.status()
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                    continue;
                }
                if resp.status() == reqwest::StatusCode::NOT_FOUND {
                    return Err(CliError::Message(format!("MintGarden 404 for {url}")));
                }
                let resp = resp.error_for_status()?;
                // Pace successful fetches to reduce MintGarden 429s.
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                return Ok(resp.json().await?);
            }
            Err(err) if attempt <= 10 => {
                let secs = 30u64;
                eprintln!("  MintGarden request error ({err}); sleeping {secs}s (attempt {attempt}/10)...");
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            }
            Err(err) => return Err(CliError::Request(err)),
        }
    }
}

async fn latest_known_event_unix(http: &reqwest::Client) -> Result<u64, CliError> {
    let url = format!(
        "{MINTGARDEN_API}/events?collection={MINTGARDEN_CNS_COLLECTION}&type=0&type=1&type=2&type=3&size=1"
    );
    let resp: Page<MgEventProbe> = http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let Some(ev) = resp.items.first() else {
        return Ok(0);
    };
    parse_rfc3339_unix(&ev.timestamp)
}

#[derive(Debug, Deserialize)]
struct MgEventProbe {
    timestamp: String,
}

fn detail_to_candidate(
    source: Source,
    nft: &MgNftDetail,
) -> Result<Option<LegacyCandidate>, CliError> {
    let expected_collection = match source {
        Source::Cns => MINTGARDEN_CNS_COLLECTION,
        Source::NamesDao => MINTGARDEN_NAMESDAO_COLLECTION,
    };
    if nft.collection.as_ref().map(|c| c.id.as_str()) != Some(expected_collection) {
        // Not attributed to this collection on MintGarden detail — skip.
        return Ok(None);
    }

    let Some(meta) = nft.data.as_ref().and_then(|d| d.metadata_json.as_ref()) else {
        return Ok(None);
    };
    let Some(raw_name) = meta
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    let original_name = strip_xch_suffix(raw_name);
    let Some((handle, kind)) = classify_legacy_name(&original_name) else {
        return Ok(None);
    };

    let legacy_expiration = match source {
        Source::Cns => {
            let date = meta
                .get("attributes")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .find_map(|attr| {
                    if attr.get("trait_type")?.as_str()? == "Expiration" {
                        attr.get("value")?.as_str().map(|s| s.to_string())
                    } else {
                        None
                    }
                });
            date.and_then(|d| parse_cns_expiration(&d)).unwrap_or(0)
        }
        Source::NamesDao => {
            let height = meta
                .get("attributes")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .find_map(|attr| {
                    if attr.get("trait_type")?.as_str()? != "Expiry" {
                        return None;
                    }
                    let v = attr.get("value")?;
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                });
            height.map(parse_namesdao_expiry_height).unwrap_or(0)
        }
    };

    let Some(recipient) = nft
        .owner_address
        .as_ref()
        .and_then(|a| a.encoded_id.clone())
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    // Burn-recipient registrations are omitted from the published premine.
    if recipient == DEAD_ADDRESS {
        return Ok(None);
    }

    let registration_time = nft
        .minted_at
        .as_deref()
        .and_then(|s| parse_rfc3339_unix(s).ok())
        .or_else(|| {
            nft.events.as_ref().and_then(|events| {
                events
                    .iter()
                    .filter(|ev| ev.event_type == 0)
                    .filter_map(|ev| parse_rfc3339_unix(&ev.timestamp).ok())
                    .min()
            })
        })
        .unwrap_or(0);

    Ok(Some(LegacyCandidate {
        source,
        nft_id: nft.encoded_id.clone(),
        original_name,
        handle,
        kind,
        registration_time,
        legacy_expiration,
        recipient,
    }))
}

fn parse_rfc3339_unix(s: &str) -> Result<u64, CliError> {
    let dt = DateTime::parse_from_rfc3339(s)
        .map_err(|err| CliError::Message(format!("invalid timestamp {s:?}: {err}")))?;
    Ok(dt.with_timezone(&Utc).timestamp().max(0) as u64)
}
