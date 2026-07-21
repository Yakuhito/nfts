//! Absolute XCHandles Premine constants.

/// Migration Cutoff: 2026-07-20 09:00:00 UTC
pub const MIGRATION_CUTOFF_UNIX: u64 = 1_784_538_000;

/// Launch Instant: 2026-08-20 09:00:00 UTC
pub const LAUNCH_INSTANT_UNIX: u64 = 1_787_216_400;

/// Base Premine window after the later of legacy expiration and Launch Instant.
pub const BASE_PREMINE_EXTRA_DAYS: u64 = 122;
pub const SECONDS_PER_DAY: u64 = 86_400;
pub const BASE_PREMINE_EXTRA_SECONDS: u64 = BASE_PREMINE_EXTRA_DAYS * SECONDS_PER_DAY;

/// NamesDAO expiry height projection (hardcoded; do not look up block timestamps).
pub const NAMESDAO_EXPIRED_BELOW_HEIGHT: u64 = 9_000_000;
pub const NAMESDAO_ANCHOR_HEIGHT: u64 = 9_000_001;
pub const NAMESDAO_BLOCK_9000001_UNIX: u64 = 1_783_933_174;
pub const NAMESDAO_SECONDS_PER_BLOCK: f64 = 18.75;

/// MintGarden collection IDs.
pub const MINTGARDEN_CNS_COLLECTION: &str =
    "col10r992w4cvasaxjs7ldc0n5hlhl5dklc3x3l2tp405ra6adzczqksnw49f2";
pub const MINTGARDEN_NAMESDAO_COLLECTION: &str =
    "col1u9pemm2avjcz8t9emhga4vys5knugsfnctpkk2jyx05jc8d6ch2swe4qvm";

/// Burn / null recipient. Base Premine may still list these rows; published premine drops them.
pub const DEAD_ADDRESS: &str =
    "xch1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqm6ks6e8mvy";

/// NamesDAO DID launcher id (hex, no 0x).
pub const NAMESDAO_DID_LAUNCHER_HEX: &str =
    "8ec8c193d7d8753707af7fc1936056eea8a3589c91250ce03f464f8d506b6fea";
