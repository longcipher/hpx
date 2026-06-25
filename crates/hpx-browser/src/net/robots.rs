use scc::HashMap as SccHashMap;

#[derive(Debug, Clone, Default)]
pub struct RobotsRules {
    pub user_agent: String,
    pub allow: Vec<String>,
    pub disallow: Vec<String>,
    pub sitemaps: Vec<String>,
}

pub struct RobotsCache {
    cache: SccHashMap<String, RobotsRules>,
}

impl RobotsCache {
    pub fn new() -> Self {
        Self {
            cache: SccHashMap::new(),
        }
    }

    /// Parse robots.txt content into rules. Handles `User-agent: *` only.
    pub fn parse(content: &str) -> RobotsRules {
        let mut rules = RobotsRules {
            user_agent: "*".to_string(),
            ..RobotsRules::default()
        };

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let Some((key, value)) = line.split_once(':') else {
                continue;
            };

            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();

            if value.is_empty() {
                continue;
            }

            match key.as_str() {
                "user-agent" => {
                    if rules.user_agent == "*" {
                        rules.user_agent = value.to_string();
                    }
                }
                "allow" => rules.allow.push(value.to_string()),
                "disallow" => rules.disallow.push(value.to_string()),
                "sitemap" => rules.sitemaps.push(value.to_string()),
                _ => {}
            }
        }

        // Sort longer (more specific) paths first so matching is deterministic.
        rules.allow.sort_by_key(|b| std::cmp::Reverse(b.len()));
        rules.disallow.sort_by_key(|b| std::cmp::Reverse(b.len()));

        rules
    }

    /// Check if `path` is allowed under the given rules.
    /// Returns `true` (allowed) by default; explicit disallow blocks.
    /// More-specific rules (longer prefix) win over less-specific ones.
    pub fn is_allowed(rules: &RobotsRules, path: &str) -> bool {
        // Find the longest matching disallow rule.
        let disallow_match = rules.disallow.iter().find(|d| path.starts_with(d.as_str()));

        // Find the longest matching allow rule.
        let allow_match = rules.allow.iter().find(|a| path.starts_with(a.as_str()));

        // Both lists are sorted longest-first, so the first match is the most
        // specific. If allow is more specific (or equal), it wins.
        match (disallow_match, allow_match) {
            (Some(dis), Some(al)) => al.len() >= dis.len(),
            (Some(_), None) => false,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_comments_and_blanks() {
        let r = RobotsCache::parse("# comment\n\nUser-agent: *\nDisallow: /admin\n# another\n");
        assert_eq!(r.disallow, vec!["/admin"]);
        assert!(r.allow.is_empty());
    }

    #[test]
    fn parse_multiple_disallow_and_allow() {
        let r = RobotsCache::parse(
            "User-agent: *\nDisallow: /tmp\nDisallow: /private\nAllow: /tmp/pub\n",
        );
        assert!(r.disallow.contains(&"/tmp".to_string()));
        assert!(r.disallow.contains(&"/private".to_string()));
        assert!(r.allow.contains(&"/tmp/pub".to_string()));
    }

    #[test]
    fn parse_sitemap() {
        let r = RobotsCache::parse("User-agent: *\nSitemap: https://example.com/sitemap.xml\n");
        assert_eq!(r.sitemaps, vec!["https://example.com/sitemap.xml"]);
    }

    #[test]
    fn disallow_blocks_path_prefix() {
        let r = RobotsCache::parse("User-agent: *\nDisallow: /admin\n");
        assert!(!RobotsCache::is_allowed(&r, "/admin"));
        assert!(!RobotsCache::is_allowed(&r, "/admin/settings"));
        assert!(RobotsCache::is_allowed(&r, "/public"));
    }

    #[test]
    fn allow_overrides_disallow_when_more_specific() {
        let r = RobotsCache::parse("User-agent: *\nDisallow: /tmp\nAllow: /tmp/pub\n");
        assert!(!RobotsCache::is_allowed(&r, "/tmp/secret"));
        assert!(RobotsCache::is_allowed(&r, "/tmp/pub"));
        assert!(RobotsCache::is_allowed(&r, "/tmp/pub/file.txt"));
    }

    #[test]
    fn disallow_empty_means_allow_all() {
        let r = RobotsCache::parse("User-agent: *\nDisallow:\n");
        assert!(RobotsCache::is_allowed(&r, "/anything"));
    }

    #[test]
    fn default_allows_everything() {
        let r = RobotsCache::parse("");
        assert!(RobotsCache::is_allowed(&r, "/whatever"));
    }

    #[test]
    fn cache_store_and_get() {
        let cache = RobotsCache::new();
        let rules = RobotsCache::parse("User-agent: *\nDisallow: /secret\n");
        let _ = cache
            .cache
            .insert_sync("example.com".to_string(), rules.clone());
        let entry = cache.cache.get_sync("example.com").unwrap();
        assert_eq!(entry.disallow, vec!["/secret"]);
    }
}
