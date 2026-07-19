use crate::premine::constants::{
    BASE_PREMINE_EXTRA_SECONDS, LAUNCH_INSTANT_UNIX, NAMESDAO_ANCHOR_HEIGHT,
    NAMESDAO_BLOCK_9000001_UNIX, NAMESDAO_EXPIRED_BELOW_HEIGHT, NAMESDAO_SECONDS_PER_BLOCK,
};

/// CNS calendar date `YYYY-MM-DD` → 23:59:59 UTC that day.
pub fn parse_cns_expiration(date: &str) -> Option<u64> {
    let date = date.trim();
    let parts: Vec<_> = date.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;
    let naive = chrono::NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(23, 59, 59)?;
    Some(naive.and_utc().timestamp() as u64)
}

/// NamesDAO `Expiry` height → legacy UNIX seconds using the fixed projection.
pub fn parse_namesdao_expiry_height(expiry_height: u64) -> u64 {
    if expiry_height < NAMESDAO_EXPIRED_BELOW_HEIGHT {
        return 0;
    }
    let delta = expiry_height as f64 - NAMESDAO_ANCHOR_HEIGHT as f64;
    let seconds = (delta * NAMESDAO_SECONDS_PER_BLOCK).ceil() as i64;
    let result = NAMESDAO_BLOCK_9000001_UNIX as i64 + seconds;
    result.max(0) as u64
}

/// Base Premine Expiration = max(legacy, Launch Instant) + 122 days.
pub fn base_premine_expiration(legacy_expiration: u64) -> u64 {
    legacy_expiration.max(LAUNCH_INSTANT_UNIX) + BASE_PREMINE_EXTRA_SECONDS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::premine::constants::NAMESDAO_BLOCK_9000001_UNIX;

    #[test]
    fn cns_date_is_end_of_utc_day() {
        // 2024-02-06 23:59:59 UTC
        assert_eq!(parse_cns_expiration("2024-02-06"), Some(1_707_263_999));
    }

    #[test]
    fn namesdao_below_threshold_is_zero() {
        assert_eq!(parse_namesdao_expiry_height(8_999_999), 0);
    }

    #[test]
    fn namesdao_at_9000000_uses_formula() {
        // ceil((9000000 - 9000001) * 18.75) = ceil(-18.75) = -18
        assert_eq!(
            parse_namesdao_expiry_height(9_000_000),
            NAMESDAO_BLOCK_9000001_UNIX - 18
        );
    }

    #[test]
    fn namesdao_anchor_height_is_exact() {
        assert_eq!(
            parse_namesdao_expiry_height(9_000_001),
            NAMESDAO_BLOCK_9000001_UNIX
        );
    }

    #[test]
    fn base_expiration_uses_launch_when_legacy_earlier() {
        assert_eq!(
            base_premine_expiration(0),
            LAUNCH_INSTANT_UNIX + BASE_PREMINE_EXTRA_SECONDS
        );
        // 2026-12-20 09:00:00 UTC
        assert_eq!(base_premine_expiration(0), 1_797_757_200);
    }

    #[test]
    fn base_expiration_uses_legacy_when_later() {
        let legacy = LAUNCH_INSTANT_UNIX + 10;
        assert_eq!(
            base_premine_expiration(legacy),
            legacy + BASE_PREMINE_EXTRA_SECONDS
        );
    }
}
