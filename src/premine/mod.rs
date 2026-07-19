//! Deterministic Base Premine generation and MintGarden confirmation helpers.

pub mod constants;
pub mod csv;
pub mod expiration;
pub mod handle;
pub mod metadata_verify;
pub mod selection;

pub use constants::*;
pub use csv::{PremineRow, WarningRow, write_premine_csvs_atomic};
pub use expiration::{parse_cns_expiration, parse_namesdao_expiry_height};
pub use handle::{classify_legacy_name, strip_xch_suffix};
pub use metadata_verify::accept_metadata_bytes;
pub use selection::{LegacyCandidate, Source, assert_unique_handles, build_base_premine};
