#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateKind {
    /// Exact spelling already satisfies Handle grammar (no `-` / `_`).
    Exact = 0,
    /// Contains `-` or `_` and is valid after stripping those characters.
    Stripped = 1,
}

/// If `name` contains `.xch`, keep only the substring before the first `.xch`
/// (NamesDAO CHIP-0007 names look like `scott.xch 8722634`). Otherwise return `name` unchanged.
pub fn strip_xch_suffix(name: &str) -> String {
    match name.split_once(".xch") {
        Some((before, _)) => before.to_string(),
        None => name.to_string(),
    }
}

pub fn is_valid_handle(handle: &str) -> bool {
    let len = handle.len();
    (3..=63).contains(&len)
        && handle
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Classify a legacy metadata name after `.xch` normalization via [`strip_xch_suffix`].
/// Returns `(handle, kind)` when the string is an Exact or Stripped Handle Candidate.
///
/// Names that begin with one or more `_` are ineligible (NamesDAO reserved / alias forms
/// such as `_lucas` / `__lucas` must not strip into a public handle).
pub fn classify_legacy_name(name_after_xch_removal: &str) -> Option<(String, CandidateKind)> {
    if name_after_xch_removal.starts_with('_') {
        return None;
    }

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
    fn strips_at_first_xch() {
        assert_eq!(strip_xch_suffix("foo.xch"), "foo");
        assert_eq!(strip_xch_suffix("foo.xch.xch"), "foo");
        assert_eq!(strip_xch_suffix("scott.xch 8722634"), "scott");
        assert_eq!(strip_xch_suffix("_scott.xch 4508310"), "_scott");
        assert_eq!(strip_xch_suffix("a.xchb"), "a");
        assert_eq!(strip_xch_suffix("___adrianscott"), "___adrianscott");
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
    fn rejects_leading_underscores() {
        assert_eq!(classify_legacy_name("_lucas"), None);
        assert_eq!(classify_legacy_name("__lucas"), None);
        assert_eq!(classify_legacy_name("___adrianscott"), None);
        assert_eq!(classify_legacy_name("_scott"), None);
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
