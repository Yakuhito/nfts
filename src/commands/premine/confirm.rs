use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::cli::PremineConfirmArgs;
use crate::error::CliError;
use crate::premine::csv::parse_premine_csv;
use crate::premine::handle::is_valid_handle;
use crate::premine::{
    LegacyCandidate, MIGRATION_CUTOFF_UNIX, MINTGARDEN_CNS_COLLECTION,
    MINTGARDEN_NAMESDAO_COLLECTION, PremineRow, Source, assert_unique_handles, build_base_premine,
    classify_legacy_name, parse_cns_expiration, parse_namesdao_expiry_height, strip_xch_suffix,
};
use crate::utils::encode_puzzle_hash_address;
use chia_wallet_sdk::prelude::Bytes32;
use chia_wallet_sdk::utils::Address;

const MINTGARDEN_API: &str = "https://api.mintgarden.io";

pub async fn run(args: PremineConfirmArgs) -> Result<(), CliError> {
    let contents = tokio::fs::read_to_string(&args.input).await?;
    let input_mtime = std::fs::metadata(&args.input)?.modified().ok();
    let actual = parse_premine_csv(&contents)?;

    let http = reqwest::Client::builder()
        .user_agent("nfts-premine-confirm/0.1")
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    // Probe latest event timestamp to decide pre-cutoff rehearsal mode.
    let tip_unix = latest_known_event_unix(&http).await.unwrap_or(0);
    let (effective_cutoff_unix, pre_cutoff) = if tip_unix > 0 && tip_unix < MIGRATION_CUTOFF_UNIX {
        eprintln!(
            "WARNING: latest MintGarden ownership event ({tip_unix}) precedes Migration Cutoff ({MIGRATION_CUTOFF_UNIX})."
        );
        eprintln!(
            "WARNING: confirming against the latest externally available state (rehearsal). This is NOT final Migration Cutoff confirmation."
        );
        (tip_unix, true)
    } else if tip_unix == 0 {
        eprintln!(
            "WARNING: could not determine MintGarden tip time; using Migration Cutoff {MIGRATION_CUTOFF_UNIX}."
        );
        (MIGRATION_CUTOFF_UNIX, false)
    } else {
        (MIGRATION_CUTOFF_UNIX, false)
    };

    println!("Fetching CNS collection from MintGarden...");
    let cns_nfts = fetch_all_collection_nfts(&http, MINTGARDEN_CNS_COLLECTION).await?;
    println!("  {} CNS NFTs", cns_nfts.len());

    println!("Fetching NamesDAO collection from MintGarden...");
    let namesdao_nfts = fetch_all_collection_nfts(&http, MINTGARDEN_NAMESDAO_COLLECTION).await?;
    println!("  {} NamesDAO NFTs", namesdao_nfts.len());

    println!("Fetching CNS events...");
    let cns_events = fetch_all_events(&http, MINTGARDEN_CNS_COLLECTION).await?;
    println!("  {} CNS events", cns_events.len());

    println!("Fetching NamesDAO events...");
    let namesdao_events = fetch_all_events(&http, MINTGARDEN_NAMESDAO_COLLECTION).await?;
    println!("  {} NamesDAO events", namesdao_events.len());

    let mut recipients = HashMap::new();
    collect_recipients_at_cutoff(&cns_events, effective_cutoff_unix, &mut recipients);
    collect_recipients_at_cutoff(&namesdao_events, effective_cutoff_unix, &mut recipients);

    let mut candidates = Vec::new();
    for nft in cns_nfts {
        if let Some(c) = mintgarden_to_candidate(Source::Cns, &nft, &recipients)? {
            candidates.push(c);
        }
    }
    for nft in namesdao_nfts {
        if let Some(c) = mintgarden_to_candidate(Source::NamesDao, &nft, &recipients)? {
            candidates.push(c);
        }
    }

    let expected = build_base_premine(&candidates, effective_cutoff_unix);
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
            "OK: Base Premine matches MintGarden reconstruction ({} rows). pre_cutoff_rehearsal={pre_cutoff}",
            expected.len()
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
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https" {
        return false;
    }
    if url.host_str() != Some("mintgarden.io") {
        return false;
    }
    let mut segments = match url.path_segments() {
        Some(segments) => segments,
        None => return false,
    };
    matches!(
        (segments.next(), segments.next(), segments.next()),
        (Some("nfts"), Some(nft_id), None) if nft_id.starts_with("nft1") && !nft_id.is_empty()
    )
}

#[derive(Debug, Deserialize)]
struct Page<T> {
    items: Vec<T>,
    next: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct MgNft {
    id: String,
    encoded_id: String,
    metadata: Option<JsonValue>,
    minted_at: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct MgEvent {
    nft_id: String,
    event_index: i64,
    #[serde(rename = "type")]
    event_type: i32,
    timestamp: String,
    address: MgAddress,
}

#[derive(Debug, Deserialize, Clone)]
struct MgAddress {
    id: String,
    encoded_id: Option<String>,
}

async fn fetch_all_collection_nfts(
    http: &reqwest::Client,
    collection_id: &str,
) -> Result<Vec<MgNft>, CliError> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let mut url = format!(
            "{MINTGARDEN_API}/collections/{collection_id}/nfts?include_metadata=true&size=50"
        );
        if let Some(p) = &page {
            url.push_str(&format!("&page={}", urlencoding_minimal(p)));
        }
        let resp: Page<MgNft> = get_json_with_retries(http, &url).await?;
        out.extend(resp.items);
        match resp.next {
            Some(next) if !next.is_empty() => page = Some(next),
            _ => break,
        }
    }
    Ok(out)
}

async fn fetch_all_events(
    http: &reqwest::Client,
    collection_id: &str,
) -> Result<Vec<MgEvent>, CliError> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    loop {
        // type 0 mint, 1 transfer, 2 trade, 3 burn
        let mut url = format!(
            "{MINTGARDEN_API}/events?collection={collection_id}&type=0&type=1&type=2&type=3&size=100"
        );
        if let Some(p) = &page {
            url.push_str(&format!("&page={}", urlencoding_minimal(p)));
        }
        let resp: Page<MgEvent> = get_json_with_retries(http, &url).await?;
        out.extend(resp.items);
        match resp.next {
            Some(next) if !next.is_empty() => page = Some(next),
            _ => break,
        }
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
                let resp = resp.error_for_status()?;
                // Pace pagination so we don't trip MintGarden rate limits mid-confirm.
                tokio::time::sleep(std::time::Duration::from_millis(750)).await;
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
    let resp: Page<MgEvent> = http
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

fn collect_recipients_at_cutoff(
    events: &[MgEvent],
    cutoff_unix: u64,
    out: &mut HashMap<String, String>,
) {
    // Last ownership-changing event at or before cutoff wins.
    let mut best: HashMap<String, (u64, i64, String)> = HashMap::new();
    for ev in events {
        if !(0..=3).contains(&ev.event_type) {
            continue;
        }
        let Ok(ts) = parse_rfc3339_unix(&ev.timestamp) else {
            continue;
        };
        if ts > cutoff_unix {
            continue;
        }
        let recipient = ev
            .address
            .encoded_id
            .clone()
            .unwrap_or_else(|| puzzle_hash_hex_to_xch(&ev.address.id).unwrap_or_default());
        if recipient.is_empty() {
            continue;
        }
        let entry = best
            .entry(ev.nft_id.clone())
            .or_insert((0, -1, String::new()));
        if ts > entry.0 || (ts == entry.0 && ev.event_index > entry.1) {
            *entry = (ts, ev.event_index, recipient);
        }
    }
    for (nft_id, (_, _, recipient)) in best {
        out.insert(nft_id, recipient);
    }
}

fn mintgarden_to_candidate(
    source: Source,
    nft: &MgNft,
    recipients: &HashMap<String, String>,
) -> Result<Option<LegacyCandidate>, CliError> {
    let Some(meta) = nft.metadata.as_ref() else {
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

    let Some(recipient) = recipients.get(&nft.id).cloned() else {
        // No ownership event at/before cutoff — skip (cannot assign recipient).
        return Ok(None);
    };

    let registration_time = nft
        .minted_at
        .as_deref()
        .and_then(|s| parse_rfc3339_unix(s).ok())
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

fn puzzle_hash_hex_to_xch(hex_str: &str) -> Result<String, CliError> {
    let normalized = hex_str.trim().trim_start_matches("0x");
    let bytes = hex::decode(normalized)
        .map_err(|err| CliError::Message(format!("invalid puzzle hash hex: {err}")))?;
    if bytes.len() != 32 {
        return Err(CliError::Message(format!(
            "expected 32-byte puzzle hash, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    encode_puzzle_hash_address(&Bytes32::new(arr))
}

fn urlencoding_minimal(value: &str) -> String {
    // MintGarden cursors are typically URL-safe; still escape reserved chars.
    value
        .chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}
