use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::premine::csv::{PremineRow, mintgarden_nft_url};
use crate::premine::expiration::base_premine_expiration;
use crate::premine::handle::CandidateKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    Cns,
    NamesDao,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cns => "cns",
            Self::NamesDao => "namesdao",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyCandidate {
    pub source: Source,
    pub nft_id: String,
    pub original_name: String,
    pub handle: String,
    pub kind: CandidateKind,
    pub registration_time: u64,
    pub legacy_expiration: u64,
    pub recipient: String,
}

impl LegacyCandidate {
    pub fn is_active_at(&self, cutoff_unix: u64) -> bool {
        self.legacy_expiration > cutoff_unix
    }
}

fn successor_cmp(a: &LegacyCandidate, b: &LegacyCandidate) -> std::cmp::Ordering {
    // Latest Registration Time; identical timestamp → lexicographically smallest NFT ID.
    a.registration_time
        .cmp(&b.registration_time)
        .then_with(|| b.nft_id.cmp(&a.nft_id))
}

/// For identical original strings: latest active Registration Time, else latest expired.
pub fn select_successor<'a>(
    records: &[&'a LegacyCandidate],
    cutoff_unix: u64,
) -> Option<&'a LegacyCandidate> {
    if records.is_empty() {
        return None;
    }
    let actives: Vec<_> = records
        .iter()
        .copied()
        .filter(|r| r.is_active_at(cutoff_unix))
        .collect();
    let pool: Vec<&LegacyCandidate> = if actives.is_empty() {
        records.to_vec()
    } else {
        actives
    };
    pool.into_iter().max_by(|a, b| successor_cmp(a, b))
}

/// Between distinct strings for one Handle: Exact>Stripped, Active>Expired,
/// earliest Registration Time, then lexicographically smallest NFT ID.
pub fn select_collision_winner<'a>(
    records: &[&'a LegacyCandidate],
    cutoff_unix: u64,
) -> Option<&'a LegacyCandidate> {
    records.iter().copied().min_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| {
                let a_active = a.is_active_at(cutoff_unix);
                let b_active = b.is_active_at(cutoff_unix);
                b_active.cmp(&a_active)
            })
            .then_with(|| a.registration_time.cmp(&b.registration_time))
            .then_with(|| a.nft_id.cmp(&b.nft_id))
    })
}

/// Resolve CNS completely, then NamesDAO for remaining Handles.
pub fn build_base_premine(candidates: &[LegacyCandidate], cutoff_unix: u64) -> Vec<PremineRow> {
    let mut claimed: BTreeMap<String, LegacyCandidate> = BTreeMap::new();

    for source in [Source::Cns, Source::NamesDao] {
        let source_records: Vec<&LegacyCandidate> =
            candidates.iter().filter(|c| c.source == source).collect();

        let mut by_original: HashMap<&str, Vec<&LegacyCandidate>> = HashMap::new();
        for record in source_records {
            by_original
                .entry(record.original_name.as_str())
                .or_default()
                .push(record);
        }

        let mut authoritative = Vec::new();
        for group in by_original.values() {
            if let Some(winner) = select_successor(group, cutoff_unix) {
                authoritative.push(winner);
            }
        }

        let mut by_handle: HashMap<&str, Vec<&LegacyCandidate>> = HashMap::new();
        for record in authoritative {
            by_handle
                .entry(record.handle.as_str())
                .or_default()
                .push(record);
        }

        for (handle, group) in by_handle {
            if claimed.contains_key(handle) {
                continue;
            }
            if let Some(winner) = select_collision_winner(&group, cutoff_unix) {
                claimed.insert(handle.to_string(), winner.clone());
            }
        }
    }

    claimed
        .into_values()
        .map(|c| PremineRow {
            handle: c.handle,
            recipient: c.recipient,
            expiration: base_premine_expiration(c.legacy_expiration),
            allocation_type: c.source.as_str().to_string(),
            allocation_explanation: mintgarden_nft_url(&c.nft_id),
        })
        .collect()
}

pub fn assert_unique_handles(rows: &[PremineRow]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.handle.as_str()) {
            return Err(format!("duplicate handle: {}", row.handle));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(
        source: Source,
        nft_id: &str,
        original: &str,
        handle: &str,
        kind: CandidateKind,
        reg: u64,
        exp: u64,
        recipient: &str,
    ) -> LegacyCandidate {
        LegacyCandidate {
            source,
            nft_id: nft_id.into(),
            original_name: original.into(),
            handle: handle.into(),
            kind,
            registration_time: reg,
            legacy_expiration: exp,
            recipient: recipient.into(),
        }
    }

    const CUTOFF: u64 = 1_000;

    #[test]
    fn succession_prefers_latest_active() {
        let a = cand(
            Source::Cns,
            "nft1a",
            "alice",
            "alice",
            CandidateKind::Exact,
            100,
            2_000,
            "xch1a",
        );
        let b = cand(
            Source::Cns,
            "nft1b",
            "alice",
            "alice",
            CandidateKind::Exact,
            200,
            2_000,
            "xch1b",
        );
        let c = cand(
            Source::Cns,
            "nft1c",
            "alice",
            "alice",
            CandidateKind::Exact,
            300,
            500,
            "xch1c",
        );
        let refs = vec![&a, &b, &c];
        assert_eq!(select_successor(&refs, CUTOFF).unwrap().nft_id, "nft1b");
    }

    #[test]
    fn succession_latest_expired_when_none_active() {
        let a = cand(
            Source::Cns,
            "nft1a",
            "alice",
            "alice",
            CandidateKind::Exact,
            100,
            50,
            "xch1a",
        );
        let b = cand(
            Source::Cns,
            "nft1b",
            "alice",
            "alice",
            CandidateKind::Exact,
            200,
            50,
            "xch1b",
        );
        let refs = vec![&a, &b];
        assert_eq!(select_successor(&refs, CUTOFF).unwrap().nft_id, "nft1b");
    }

    #[test]
    fn collision_exact_beats_stripped_regardless_of_activity() {
        let exact_expired = cand(
            Source::Cns,
            "nft1e",
            "foobar",
            "foobar",
            CandidateKind::Exact,
            500,
            50,
            "xch1e",
        );
        let stripped_active = cand(
            Source::Cns,
            "nft1s",
            "foo-bar",
            "foobar",
            CandidateKind::Stripped,
            100,
            2_000,
            "xch1s",
        );
        let refs = vec![&exact_expired, &stripped_active];
        assert_eq!(
            select_collision_winner(&refs, CUTOFF).unwrap().nft_id,
            "nft1e"
        );
    }

    #[test]
    fn collision_active_beats_expired_within_category() {
        let older_active = cand(
            Source::Cns,
            "nft1a",
            "alice",
            "alice",
            CandidateKind::Exact,
            100,
            2_000,
            "xch1a",
        );
        let newer_expired = cand(
            Source::Cns,
            "nft1b",
            "al-ice",
            "alice",
            CandidateKind::Exact,
            200,
            50,
            "xch1b",
        );
        let refs = vec![&older_active, &newer_expired];
        assert_eq!(
            select_collision_winner(&refs, CUTOFF).unwrap().nft_id,
            "nft1a"
        );
    }

    #[test]
    fn collision_earliest_registration_when_activity_equal() {
        let early = cand(
            Source::Cns,
            "nft1b",
            "alice",
            "alice",
            CandidateKind::Exact,
            100,
            2_000,
            "xch1b",
        );
        let late = cand(
            Source::Cns,
            "nft1a",
            "alice2",
            "alice",
            CandidateKind::Exact,
            200,
            2_000,
            "xch1a",
        );
        let refs = vec![&late, &early];
        assert_eq!(
            select_collision_winner(&refs, CUTOFF).unwrap().nft_id,
            "nft1b"
        );
    }

    #[test]
    fn collision_smallest_nft_id_on_timestamp_tie() {
        let a = cand(
            Source::Cns,
            "nft1aaa",
            "alice",
            "alice",
            CandidateKind::Exact,
            100,
            2_000,
            "xch1a",
        );
        let b = cand(
            Source::Cns,
            "nft1bbb",
            "alice2",
            "alice",
            CandidateKind::Exact,
            100,
            2_000,
            "xch1b",
        );
        let refs = vec![&b, &a];
        assert_eq!(
            select_collision_winner(&refs, CUTOFF).unwrap().nft_id,
            "nft1aaa"
        );
    }

    #[test]
    fn cns_blocks_namesdao_even_when_namesdao_is_exact() {
        let cns = cand(
            Source::Cns,
            "nft1cns",
            "foo-bar",
            "foobar",
            CandidateKind::Stripped,
            100,
            2_000,
            "xch1cns",
        );
        let ndao = cand(
            Source::NamesDao,
            "nft1ndao",
            "foobar",
            "foobar",
            CandidateKind::Exact,
            50,
            2_000,
            "xch1ndao",
        );
        let rows = build_base_premine(&[cns, ndao], CUTOFF);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].recipient, "xch1cns");
        assert_eq!(rows[0].allocation_type, "cns");
        assert_eq!(
            rows[0].allocation_explanation,
            "https://mintgarden.io/nfts/nft1cns"
        );
    }

    #[test]
    fn namesdao_row_records_namesdao_allocation_provenance() {
        let ndao = cand(
            Source::NamesDao,
            "nft1ndao",
            "bob",
            "bob",
            CandidateKind::Exact,
            50,
            2_000,
            "xch1ndao",
        );
        let rows = build_base_premine(&[ndao], CUTOFF);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].allocation_type, "namesdao");
        assert_eq!(
            rows[0].allocation_explanation,
            "https://mintgarden.io/nfts/nft1ndao"
        );
    }

    #[test]
    fn output_sorted_by_handle() {
        let a = cand(
            Source::Cns,
            "nft1z",
            "zzz",
            "zzz",
            CandidateKind::Exact,
            1,
            2_000,
            "xch1z",
        );
        let b = cand(
            Source::Cns,
            "nft1a",
            "aaa",
            "aaa",
            CandidateKind::Exact,
            1,
            2_000,
            "xch1a",
        );
        let rows = build_base_premine(&[a, b], CUTOFF);
        assert_eq!(rows[0].handle, "aaa");
        assert_eq!(rows[1].handle, "zzz");
    }
}
