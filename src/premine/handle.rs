#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateKind {
    /// Exact spelling already satisfies Handle grammar (no `-` / `_`).
    Exact = 0,
    /// Contains `-` or `_` and is valid after stripping those characters.
    Stripped = 1,
}

pub fn strip_xch_suffix(name: &str) -> String {
    name.replace(".xch", "")
}

pub fn is_valid_handle(handle: &str) -> bool {
    let len = handle.len();
    (3..=63).contains(&len)
        && handle
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Classify a legacy metadata name after `.xch` removal.
/// Returns `(handle, kind)` when the string is an Exact or Stripped Handle Candidate.
pub fn classify_legacy_name(name_after_xch_removal: &str) -> Option<(String, CandidateKind)> {
    if is_valid_handle(name_after_xch_removal) {
        // Exact candidates must not contain `-` or `_` — already true if valid handle.
        return Some((name_after_xch_removal.to_string(), CandidateKind::Exact));
    }

    let has_separator =
        name_after_xch_removal.contains('-') || name_after_xch_removal.contains('_');
    if !has_separator {
        return None;
    }

    let stripped: String = name_after_xch_removal
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .collect();
    if is_valid_handle(&stripped) {
        Some((stripped, CandidateKind::Stripped))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_every_literal_xch() {
        assert_eq!(strip_xch_suffix("foo.xch"), "foo");
        assert_eq!(strip_xch_suffix("foo.xch.xch"), "foo");
        assert_eq!(strip_xch_suffix("a.xchb"), "ab");
    }

    #[test]
    fn exact_candidate() {
        assert_eq!(
            classify_legacy_name("alice"),
            Some(("alice".into(), CandidateKind::Exact))
        );
    }

    #[test]
    fn stripped_candidate() {
        assert_eq!(
            classify_legacy_name("foo-bar"),
            Some(("foobar".into(), CandidateKind::Stripped))
        );
        assert_eq!(
            classify_legacy_name("foo_bar"),
            Some(("foobar".into(), CandidateKind::Stripped))
        );
    }

    #[test]
    fn rejects_invalid() {
        assert_eq!(classify_legacy_name("ab"), None);
        assert_eq!(classify_legacy_name("Foo"), None);
        assert_eq!(classify_legacy_name("foo.bar"), None);
        assert_eq!(classify_legacy_name(""), None);
    }

    #[test]
    fn exact_outranks_stripped_in_kind_ord() {
        assert!(CandidateKind::Exact < CandidateKind::Stripped);
    }
}
