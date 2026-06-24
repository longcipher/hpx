use std::{collections::HashSet, sync::LazyLock};

static BLOCKED_DOMAINS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    include_str!("blocklist.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .collect()
});

/// Check if `domain` or any of its parent domains are in the tracker blocklist.
pub fn is_blocked(domain: &str) -> bool {
    let lower = domain.to_ascii_lowercase();
    let mut parts: &str = &lower;
    loop {
        if BLOCKED_DOMAINS.contains(parts) {
            return true;
        }
        match parts.find('.') {
            Some(pos) => parts = &parts[pos + 1..],
            None => break,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_blocked() {
        assert!(is_blocked("google-analytics.com"));
    }

    #[test]
    fn subdomain_blocked() {
        assert!(is_blocked("www.google-analytics.com"));
        assert!(is_blocked("ssl.google-analytics.com"));
    }

    #[test]
    fn case_insensitive() {
        assert!(is_blocked("Google-Analytics.COM"));
        assert!(is_blocked("GOOGLE-ANALYTICS.com"));
    }

    #[test]
    fn clean_domain_not_blocked() {
        assert!(!is_blocked("example.com"));
        assert!(!is_blocked("rust-lang.org"));
        assert!(!is_blocked("github.com"));
    }

    #[test]
    fn partial_match_not_blocked() {
        // "analytics.com" shouldn't match just because "google-analytics.com" is listed
        assert!(!is_blocked("analytics.com"));
    }

    #[test]
    fn facebook_blocked() {
        assert!(is_blocked("connect.facebook.net"));
        assert!(is_blocked("pixel.facebook.com"));
    }

    #[test]
    fn empty_string_not_blocked() {
        assert!(!is_blocked(""));
    }

    #[test]
    fn ad_networks_blocked() {
        assert!(is_blocked("doubleclick.net"));
        assert!(is_blocked("adnxs.com"));
        assert!(is_blocked("criteo.com"));
        assert!(is_blocked("taboola.com"));
    }
}
