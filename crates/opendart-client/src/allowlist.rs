//! Deny-by-default allowlist of OpenDART paths this client may request.
//!
//! Anything not named here is rejected before an outbound request is ever
//! constructed, so a coding mistake elsewhere in the workspace cannot reach
//! the network with an unapproved path.

/// The only paths this client will ever request. Keep in sync with the
/// approved OpenDART read surface; add nothing speculatively.
pub const ALLOWED_PATHS: [&str; 3] = ["/api/list.json", "/api/corpCode.xml", "/api/company.json"];

/// Returns the canonical `'static` allowlisted path matching `path`, or
/// `None` if it is not on the allowlist.
///
/// Returning the canonical `&'static str` (rather than just `bool`) lets
/// callers build a request without holding on to the caller-supplied `&str`
/// (whose lifetime is not `'static`), while still guaranteeing the request
/// can only ever carry one of the three fixed paths.
pub fn resolve(path: &str) -> Option<&'static str> {
    ALLOWED_PATHS
        .iter()
        .find(|&&candidate| candidate == path)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_paths_resolve_to_themselves() {
        for path in ALLOWED_PATHS {
            assert_eq!(resolve(path), Some(path));
        }
    }

    #[test]
    fn unknown_paths_are_rejected() {
        assert_eq!(resolve("/api/unknownThing.json"), None);
        assert_eq!(resolve("/api/list.json/../../etc/passwd"), None);
        assert_eq!(resolve(""), None);
        assert_eq!(resolve("/api/list.json?"), None);
        assert_eq!(resolve("/api/document.xml"), None);
    }
}
