//! RFC 6265 cookie jar with domain scoping, expiry, host-only semantics,
//! and JSON persistence.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use url::Url;

/// A cookie jar that stores cookies per domain with full RFC 6265 compliance.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CookieJar {
    /// Cookies keyed by domain -> (name -> Cookie)
    cookies: HashMap<String, HashMap<String, Cookie>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cookie {
    name: String,
    value: String,
    path: String,
    /// Domain= attribute (leading dot stripped, lowercased). None = host-only.
    #[serde(default)]
    domain: Option<String>,
    secure: bool,
    #[allow(dead_code)]
    http_only: bool,
    /// Expiry as unix timestamp. None = session cookie.
    #[allow(dead_code)]
    expires: Option<u64>,
}

impl CookieJar {
    pub fn new() -> Self {
        Self::default()
    }

    /// Total cookie count across all domains.
    pub fn cookie_count(&self) -> usize {
        self.cookies.values().map(|m| m.len()).sum()
    }

    /// Parse and store cookies from Set-Cookie response headers.
    pub fn set_cookies(&mut self, url: &Url, set_cookie_headers: &[String]) {
        let request_domain = match url.host_str() {
            Some(d) => d.to_lowercase(),
            None => return,
        };

        let now = unix_now().max(0) as u64;
        for header in set_cookie_headers {
            if let Some(cookie) = parse_set_cookie(header, &request_domain, url.path()) {
                let store_domain = match &cookie.domain {
                    Some(d) if domain_matches(&request_domain, d) => d.clone(),
                    _ => request_domain.clone(),
                };
                // Past expiry = deletion (RFC 6265 section 5.3 step 11).
                if cookie.expires.is_some_and(|e| e <= now) {
                    if let Some(bucket) = self.cookies.get_mut(&store_domain) {
                        bucket.remove(&cookie.name);
                        if bucket.is_empty() {
                            self.cookies.remove(&store_domain);
                        }
                    }
                    continue;
                }
                self.cookies
                    .entry(store_domain)
                    .or_default()
                    .insert(cookie.name.clone(), cookie);
            }
        }
    }

    /// Get the Cookie header value for a given URL.
    pub fn cookies_for(&self, url: &Url) -> Option<String> {
        let request_domain = url.host_str()?.to_lowercase();
        let path = url.path();
        let is_secure = url.scheme() == "https";

        let mut pairs = Vec::new();

        for (stored_domain, cookies) in &self.cookies {
            if !domain_matches(&request_domain, stored_domain) {
                continue;
            }
            for cookie in cookies.values() {
                // Host-only cookies: only exact domain match.
                if cookie.domain.is_none() && request_domain != *stored_domain {
                    continue;
                }
                if cookie.secure && !is_secure {
                    continue;
                }
                if !path.starts_with(&cookie.path) {
                    continue;
                }
                pairs.push(format!("{}={}", cookie.name, cookie.value));
            }
        }

        if pairs.is_empty() {
            None
        } else {
            Some(pairs.join("; "))
        }
    }

    /// Persist the jar to a JSON file. Atomic via tempfile + rename.
    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Load a jar from a JSON file. Empty jar if file doesn't exist.
    pub fn load_from_file(path: &std::path::Path) -> std::io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = std::fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(std::io::Error::other)
    }

    /// Remove all cookies whose domain is a suffix match of `target_domain`.
    pub fn clear_for_domain(&mut self, target_domain: &str) -> usize {
        let target = target_domain.trim_start_matches('.').to_ascii_lowercase();
        let before = self.cookies.len();
        self.cookies
            .retain(|stored, _| !domain_matches(stored, &target));
        before - self.cookies.len()
    }
}

/// True iff `request_domain` is the same as or a subdomain of `cookie_domain`.
fn domain_matches(request_domain: &str, cookie_domain: &str) -> bool {
    let r = request_domain.trim_start_matches('.').to_lowercase();
    let c = cookie_domain.trim_start_matches('.').to_lowercase();
    r == c || r.ends_with(&format!(".{c}"))
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse cookie `Expires=` HTTP-date into a unix timestamp.
fn parse_http_date(s: &str) -> Option<u64> {
    let s = s.trim();
    const FORMATS: &[&str] = &[
        "%a, %d %b %Y %H:%M:%S GMT",
        "%a, %d-%b-%y %H:%M:%S GMT",
        "%a, %d-%b-%Y %H:%M:%S GMT",
        "%A, %d-%b-%y %H:%M:%S GMT",
        "%a %b %e %H:%M:%S %Y",
    ];
    for f in FORMATS {
        if let Ok(dt) = jiff::civil::DateTime::strptime(f, s) {
            // HTTP dates are always GMT/UTC. Attach UTC timezone to get a timestamp.
            if let Ok(zoned) = dt.to_zoned(jiff::tz::TimeZone::UTC) {
                let secs = zoned.timestamp().as_second();
                if secs >= 0 {
                    return Some(secs as u64);
                }
            }
        }
    }
    jiff::fmt::rfc2822::parse(s)
        .ok()
        .map(|zoned| zoned.timestamp().as_second().max(0) as u64)
}

/// Parse a Set-Cookie header value into a Cookie.
fn parse_set_cookie(header: &str, request_domain: &str, default_path: &str) -> Option<Cookie> {
    let mut parts = header.split(';');

    let name_value = parts.next()?.trim();
    let (name, value) = name_value.split_once('=')?;
    let name = name.trim().to_string();
    let value = value.trim().to_string();

    if name.is_empty() {
        return None;
    }

    let mut path = default_path.to_string();
    let mut domain: Option<String> = None;
    let mut secure = false;
    let mut http_only = false;
    let mut max_age: Option<i64> = None;
    let mut expires_str: Option<String> = None;

    for attr in parts {
        let attr = attr.trim();
        let (attr_name, attr_value) = attr
            .split_once('=')
            .map(|(n, v)| (n.trim().to_lowercase(), Some(v.trim())))
            .unwrap_or_else(|| (attr.to_lowercase(), None));

        match attr_name.as_str() {
            "path" => {
                if let Some(v) = attr_value {
                    path = v.to_string();
                }
            }
            "max-age" => {
                if let Some(v) = attr_value {
                    max_age = v.parse::<i64>().ok();
                }
            }
            "expires" => {
                if let Some(v) = attr_value {
                    expires_str = Some(v.to_string());
                }
            }
            "domain" => {
                if let Some(v) = attr_value {
                    let v = v.trim_start_matches('.').to_lowercase();
                    if !v.is_empty() {
                        if domain_matches(request_domain, &v) {
                            domain = Some(v);
                        } else {
                            return None; // cross-origin reject
                        }
                    }
                }
            }
            "secure" => secure = true,
            "httponly" => http_only = true,
            _ => {}
        }
    }

    let now = unix_now();
    let expires: Option<u64> = if let Some(ma) = max_age {
        Some((now + ma).max(0) as u64)
    } else if let Some(es) = &expires_str {
        parse_http_date(es)
    } else {
        None
    };

    Some(Cookie {
        name,
        value,
        path,
        domain,
        secure,
        http_only,
        expires,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get_cookie() {
        let mut jar = CookieJar::new();
        let url = Url::parse("https://example.com/path").unwrap();
        jar.set_cookies(&url, &["session=abc123; Path=/; Secure".to_string()]);
        assert_eq!(jar.cookies_for(&url), Some("session=abc123".to_string()));
    }

    #[test]
    fn no_cookies_for_different_domain() {
        let mut jar = CookieJar::new();
        let url1 = Url::parse("https://example.com/").unwrap();
        let url2 = Url::parse("https://other.com/").unwrap();
        jar.set_cookies(&url1, &["key=val".to_string()]);
        assert!(jar.cookies_for(&url2).is_none());
    }

    #[test]
    fn secure_cookie_not_sent_over_http() {
        let mut jar = CookieJar::new();
        let https_url = Url::parse("https://example.com/").unwrap();
        let http_url = Url::parse("http://example.com/").unwrap();
        jar.set_cookies(&https_url, &["token=secret; Secure".to_string()]);
        assert!(jar.cookies_for(&https_url).is_some());
        assert!(jar.cookies_for(&http_url).is_none());
    }

    #[test]
    fn domain_attribute_carries_to_parent() {
        let mut jar = CookieJar::new();
        let set_url = Url::parse("https://e.mail.ru/login").unwrap();
        jar.set_cookies(&set_url, &["t=hash; Domain=.mail.ru; Path=/".to_string()]);
        let parent = Url::parse("https://mail.ru/?afterReload").unwrap();
        assert_eq!(jar.cookies_for(&parent), Some("t=hash".to_string()));
    }

    #[test]
    fn domain_attribute_carries_to_sibling_subdomain() {
        let mut jar = CookieJar::new();
        let set_url = Url::parse("https://e.mail.ru/").unwrap();
        jar.set_cookies(&set_url, &["t=v; Domain=mail.ru".to_string()]);
        let sibling = Url::parse("https://m.mail.ru/").unwrap();
        assert_eq!(jar.cookies_for(&sibling), Some("t=v".to_string()));
    }

    #[test]
    fn domain_attribute_rejects_unrelated_origin() {
        let mut jar = CookieJar::new();
        let set_url = Url::parse("https://e.mail.ru/").unwrap();
        jar.set_cookies(&set_url, &["evil=1; Domain=evil.com".to_string()]);
        let evil = Url::parse("https://evil.com/").unwrap();
        assert!(jar.cookies_for(&evil).is_none());
        assert!(jar.cookies_for(&set_url).is_none());
    }

    #[test]
    fn host_only_cookie_does_not_carry_to_parent() {
        let mut jar = CookieJar::new();
        let set_url = Url::parse("https://e.mail.ru/").unwrap();
        jar.set_cookies(&set_url, &["s=1".to_string()]);
        let parent = Url::parse("https://mail.ru/").unwrap();
        assert!(jar.cookies_for(&parent).is_none());
        assert_eq!(jar.cookies_for(&set_url), Some("s=1".to_string()));
    }

    #[test]
    fn domain_attribute_strips_leading_dot() {
        let mut jar1 = CookieJar::new();
        let mut jar2 = CookieJar::new();
        let url = Url::parse("https://e.mail.ru/").unwrap();
        jar1.set_cookies(&url, &["t=1; Domain=.mail.ru".to_string()]);
        jar2.set_cookies(&url, &["t=1; Domain=mail.ru".to_string()]);
        let parent = Url::parse("https://mail.ru/").unwrap();
        assert_eq!(jar1.cookies_for(&parent), jar2.cookies_for(&parent));
        assert_eq!(jar1.cookies_for(&parent), Some("t=1".to_string()));
    }

    #[test]
    fn domain_matches_helper() {
        assert!(domain_matches("e.mail.ru", "mail.ru"));
        assert!(domain_matches("mail.ru", "mail.ru"));
        assert!(domain_matches("a.b.c.example.com", "example.com"));
        assert!(domain_matches("e.mail.ru", ".mail.ru"));
        assert!(!domain_matches("mail.ru", "e.mail.ru"));
        assert!(!domain_matches("evilmail.ru", "mail.ru"));
        assert!(!domain_matches("mail.ru", "evil.com"));
    }

    #[test]
    fn clear_for_domain_evicts_exact_and_subdomains() {
        let mut jar = CookieJar::new();
        let twitter = Url::parse("https://twitter.com/").unwrap();
        let mobile_twitter = Url::parse("https://mobile.twitter.com/").unwrap();
        let x = Url::parse("https://x.com/").unwrap();
        let other = Url::parse("https://example.com/").unwrap();

        jar.set_cookies(
            &twitter,
            &["guest_id=v1%3A123; Domain=.twitter.com".to_string()],
        );
        jar.set_cookies(&mobile_twitter, &["mobile_pref=dark".to_string()]);
        jar.set_cookies(&x, &["ct0=xyz".to_string()]);
        jar.set_cookies(&other, &["session=abc".to_string()]);

        let evicted = jar.clear_for_domain("twitter.com");
        assert_eq!(evicted, 2);
        assert!(jar.cookies_for(&x).is_some());
        assert!(jar.cookies_for(&other).is_some());
        assert!(jar.cookies_for(&twitter).is_none());
        assert!(jar.cookies_for(&mobile_twitter).is_none());
    }

    #[test]
    fn expired_set_cookie_deletes_instead_of_storing_empty() {
        let imdb = Url::parse("https://www.imdb.com/").unwrap();
        let mut jar = CookieJar::new();
        jar.set_cookies(
            &imdb,
            &[
                "aws-waf-token=; Path=/; Domain=www.imdb.com; Expires=Thu, 01 Jan 1970 00:00:01 GMT"
                    .to_string(),
                "aws-waf-token=; Path=/; Domain=imdb.com; Expires=Thu, 01 Jan 1970 00:00:01 GMT"
                    .to_string(),
            ],
        );
        assert!(jar.cookies_for(&imdb).is_none());
        jar.set_cookies(&imdb, &["aws-waf-token=REALTOKEN123; Path=/".to_string()]);
        assert_eq!(
            jar.cookies_for(&imdb).unwrap_or_default(),
            "aws-waf-token=REALTOKEN123"
        );
    }

    #[test]
    fn expires_2digit_year_stored_not_deleted() {
        let ts = parse_http_date("Mon, 31-May-27 00:05:44 GMT");
        assert!(ts.is_some() && ts.unwrap() > 1_800_000_000);
        assert!(parse_http_date("Wed, 09 Jun 2027 10:18:14 GMT").is_some());
    }

    #[test]
    fn persistence_round_trip() {
        let dir = std::env::temp_dir().join("hpx_browser_cookie_test");
        let path = dir.join("cookies.json");
        let mut jar = CookieJar::new();
        let url = Url::parse("https://example.com/").unwrap();
        jar.set_cookies(&url, &["k=v; Path=/".to_string()]);
        jar.save_to_file(&path).unwrap();
        let loaded = CookieJar::load_from_file(&path).unwrap();
        assert_eq!(loaded.cookies_for(&url), Some("k=v".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
