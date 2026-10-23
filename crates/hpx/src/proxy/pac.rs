//! Proxy Auto-Config (PAC) support.
//!
//! A PAC file is a JavaScript program that defines a `FindProxyForURL`
//! function. The function is called with the request URL and its host, and
//! returns a string such as `"PROXY proxy.local:8080; DIRECT"` that lists
//! the proxies to try, in order.
//!
//! The script is evaluated for every request, because the result can depend
//! on the URL. The standard helper functions are provided as globals.

use std::net::Ipv4Addr;

use boa_engine::{Context, JsResult, JsString, JsValue, NativeFunction, Source};
use http::Uri;

// The JavaScript contexts of the PAC scripts, one per thread.
//
// A JavaScript context is not thread safe, so every thread keeps its own.
// The context is created the first time a thread evaluates a script.
thread_local! {
    static PAC_CONTEXTS: std::cell::RefCell<std::collections::HashMap<usize, Context>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// A PAC rule that is evaluated for every request.
#[derive(Debug, Clone)]
pub(crate) struct PacRule {
    /// Identity token, used for `Hash`/`Eq`.
    ///
    /// A PAC script cannot be compared structurally, so two rules are equal
    /// only when they were created by the same call.
    token: std::sync::Arc<()>,
    /// The script source.
    script: String,
}

impl PacRule {
    /// Compiles the PAC script into a rule.
    pub(crate) fn new(script: &str) -> JsResult<Self> {
        // The script is parsed once, so that a syntax error is reported when
        // the client is built instead of on the first request.
        let mut context = Context::default();
        register_helpers(&mut context)?;
        context.eval(Source::from_bytes(script))?;

        Ok(Self {
            token: std::sync::Arc::new(()),
            script: script.to_owned(),
        })
    }

    /// Selects the proxy for the given destination URI.
    pub(crate) fn select(&self, dst: &Uri) -> Option<Uri> {
        let url = dst.to_string();
        let host = dst.host().unwrap_or_default();
        let key = std::sync::Arc::as_ptr(&self.token) as usize;

        let selected = PAC_CONTEXTS.with(|contexts| {
            let mut contexts = contexts.borrow_mut();
            if !contexts.contains_key(&key) {
                let mut context = Context::default();
                register_helpers(&mut context).ok()?;
                context
                    .eval(Source::from_bytes(self.script.as_bytes()))
                    .ok()?;
                contexts.insert(key, context);
            }
            let context = contexts.get_mut(&key)?;
            evaluate(context, &self.script, &url, host)
        })?;

        match selected {
            PacProxy::Http(endpoint) => proxy_uri(&endpoint, "http"),
            PacProxy::Socks(endpoint) => proxy_uri(&endpoint, "socks5"),
        }
    }
}

/// Builds the proxy URI from a PAC endpoint.
///
/// The endpoint is normally `host:port`; a value that already carries a
/// scheme is used as-is.
fn proxy_uri(endpoint: &str, default_scheme: &str) -> Option<Uri> {
    if endpoint.contains("://") {
        Uri::try_from(endpoint.to_owned()).ok()
    } else {
        Uri::try_from(format!("{default_scheme}://{endpoint}")).ok()
    }
}

/// Evaluates the `FindProxyForURL` function of the script.
fn evaluate(context: &mut Context, script: &str, url: &str, host: &str) -> Option<PacProxy> {
    // The script defines the `FindProxyForURL` function; the evaluation
    // result is the completion value, which is not the function itself.
    context.eval(Source::from_bytes(script.as_bytes())).ok()?;

    let callable = context
        .global_object()
        .get(boa_engine::js_string!("FindProxyForURL"), context)
        .ok()?
        .as_callable()?
        .clone();

    let result = callable
        .call(
            &JsValue::undefined(),
            &[
                JsValue::from(JsString::from(url)),
                JsValue::from(JsString::from(host)),
            ],
            context,
        )
        .ok()?;

    let text = result.to_string(context).ok()?.to_std_string().ok()?;
    PacResult::parse(&text).proxies.into_iter().next()
}

impl PartialEq for PacRule {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.token, &other.token)
    }
}

impl Eq for PacRule {}

impl std::hash::Hash for PacRule {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (std::sync::Arc::as_ptr(&self.token) as usize).hash(state);
    }
}

/// A proxy entry of a PAC result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PacProxy {
    /// `PROXY host:port` — an HTTP proxy.
    Http(String),
    /// `SOCKS host:port` — a SOCKS proxy.
    Socks(String),
}

/// The result of evaluating a PAC script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PacResult {
    /// The proxies to try, in order.
    pub(crate) proxies: Vec<PacProxy>,
}

impl PacResult {
    /// Parses the string returned by `FindProxyForURL`.
    ///
    /// The string is a `;`-separated list of entries such as `PROXY
    /// host:port`, `SOCKS host:port` or `DIRECT`. The `DIRECT` entry means
    /// "connect without a proxy" and ends the list.
    pub(crate) fn parse(result: &str) -> Self {
        let mut proxies = Vec::new();

        for entry in result.split(';') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }

            let (kind, endpoint) = match entry.split_once(char::is_whitespace) {
                Some((kind, endpoint)) => (kind.trim(), endpoint.trim()),
                None => (entry, ""),
            };

            if kind.eq_ignore_ascii_case("DIRECT") {
                // `DIRECT` ends the list: the remaining entries are only
                // used when the earlier ones fail.
                break;
            }

            if endpoint.is_empty() {
                continue;
            }

            if kind.eq_ignore_ascii_case("PROXY") {
                proxies.push(PacProxy::Http(endpoint.to_owned()));
            } else if kind.eq_ignore_ascii_case("SOCKS") {
                proxies.push(PacProxy::Socks(endpoint.to_owned()));
            }
        }

        Self { proxies }
    }
}

/// Registers the standard PAC helper functions as globals.
fn register_helpers(context: &mut Context) -> JsResult<()> {
    // `isPlainHostName(host)` — true when the host has no dot.
    context.register_global_callable(
        boa_engine::js_string!("isPlainHostName"),
        1,
        NativeFunction::from_copy_closure(|_this, args, context| {
            let host = string_arg(args, context, 0)?;
            Ok(JsValue::from(!host.contains('.')))
        }),
    )?;

    // `dnsDomainIs(host, domain)` — true when the host ends with the domain.
    context.register_global_callable(
        boa_engine::js_string!("dnsDomainIs"),
        2,
        NativeFunction::from_copy_closure(|_this, args, context| {
            let host = string_arg(args, context, 0)?.to_ascii_lowercase();
            let domain = string_arg(args, context, 1)?.to_ascii_lowercase();
            Ok(JsValue::from(host.ends_with(&domain)))
        }),
    )?;

    // `localHostOrDomain(host, hostdom)` — true when the host is the hostdom
    // or a host within it.
    context.register_global_callable(
        boa_engine::js_string!("localHostOrDomain"),
        2,
        NativeFunction::from_copy_closure(|_this, args, context| {
            let host = string_arg(args, context, 0)?.to_ascii_lowercase();
            let hostdom = string_arg(args, context, 1)?.to_ascii_lowercase();
            Ok(JsValue::from(
                host == hostdom || host.ends_with(&format!(".{hostdom}")),
            ))
        }),
    )?;

    // `isInNet(host, pattern, mask)` — true when the host is an IPv4
    // address inside the network described by the pattern and the mask.
    context.register_global_callable(
        boa_engine::js_string!("isInNet"),
        3,
        NativeFunction::from_copy_closure(|_this, args, context| {
            let host = string_arg(args, context, 0)?;
            let pattern = string_arg(args, context, 1)?;
            let mask = string_arg(args, context, 2)?;

            let (Some(host), Some(pattern), Some(mask)) = (
                host.parse::<Ipv4Addr>().ok(),
                pattern.parse::<Ipv4Addr>().ok(),
                mask.parse::<Ipv4Addr>().ok(),
            ) else {
                return Ok(JsValue::from(false));
            };

            let host = u32::from(host);
            let pattern = u32::from(pattern);
            let mask = u32::from(mask);

            Ok(JsValue::from(host & mask == pattern & mask))
        }),
    )?;

    // `shExpMatch(str, shexp)` — the shell-style glob match.
    context.register_global_callable(
        boa_engine::js_string!("shExpMatch"),
        2,
        NativeFunction::from_copy_closure(|_this, args, context| {
            let value = string_arg(args, context, 0)?;
            let pattern = string_arg(args, context, 1)?;
            Ok(JsValue::from(shell_match(&pattern, &value)))
        }),
    )?;

    // `weekdayRange`, `dateRange` and `timeRange` are time dependent. The
    // helpers are provided so that a PAC script using them still runs, but
    // they report that the range does not match.
    for name in ["weekdayRange", "dateRange", "timeRange"] {
        context.register_global_callable(
            boa_engine::js_string!(name),
            0,
            NativeFunction::from_copy_closure(|_this, _args, _context| Ok(JsValue::from(false))),
        )?;
    }

    // `isResolvable` and `dnsResolve` need a name lookup, which cannot be
    // done without blocking. The helpers report that the host does not
    // resolve.
    context.register_global_callable(
        boa_engine::js_string!("isResolvable"),
        1,
        NativeFunction::from_copy_closure(|_this, _args, _context| Ok(JsValue::from(false))),
    )?;
    context.register_global_callable(
        boa_engine::js_string!("dnsResolve"),
        1,
        NativeFunction::from_copy_closure(|_this, _args, _context| {
            Ok(JsValue::from(JsValue::null()))
        }),
    )?;
    context.register_global_callable(
        boa_engine::js_string!("myIpAddress"),
        0,
        NativeFunction::from_copy_closure(|_this, _args, _context| {
            Ok(JsValue::from(JsValue::null()))
        }),
    )?;

    Ok(())
}

/// Reads the string argument at `index`.
fn string_arg(args: &[JsValue], context: &mut Context, index: usize) -> JsResult<String> {
    let value = args
        .get(index)
        .ok_or_else(|| boa_engine::JsError::from(boa_engine::JsNativeError::typ()))?;
    let text = value.to_string(context)?;
    Ok(text.to_std_string().unwrap_or_default())
}

/// The shell-style glob match used by `shExpMatch`.
///
/// `*` matches any sequence of characters, `?` matches a single character.
fn shell_match(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();

    // Iterative matching with backtracking on the `*` wildcards.
    let (mut p, mut v) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);

    while v < value.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = p;
            mark = v;
            p += 1;
        } else if star != usize::MAX {
            p = star + 1;
            mark += 1;
            v = mark;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }

    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_proxy_result() {
        let result = PacResult::parse("PROXY proxy.local:8080");
        assert_eq!(
            result.proxies,
            vec![PacProxy::Http("proxy.local:8080".to_owned())]
        );
    }

    #[test]
    fn parses_a_direct_result() {
        let result = PacResult::parse("DIRECT");
        assert!(result.proxies.is_empty());
    }

    #[test]
    fn parses_a_list_of_proxies() {
        let result = PacResult::parse("PROXY a.local:1; SOCKS b.local:2; DIRECT");
        assert_eq!(
            result.proxies,
            vec![
                PacProxy::Http("a.local:1".to_owned()),
                PacProxy::Socks("b.local:2".to_owned()),
            ]
        );
    }

    #[test]
    fn direct_ends_the_list() {
        let result = PacResult::parse("DIRECT; PROXY a.local:1");
        assert!(result.proxies.is_empty());
    }

    #[test]
    fn matches_shell_patterns() {
        assert!(shell_match("*.example.com", "www.example.com"));
        assert!(!shell_match("*.example.com", "example.com"));
        assert!(shell_match("host?", "host1"));
        assert!(!shell_match("host?", "host12"));
        assert!(!shell_match("*.example.com", "www.example.org"));
    }

    #[test]
    fn evaluates_a_pac_script() {
        let rule = PacRule::new(
            r#"
            function FindProxyForURL(url, host) {
                if (dnsDomainIs(host, "example.com")) {
                    return "PROXY proxy.example.com:8080";
                }
                if (isInNet(host, "10.0.0.0", "255.0.0.0")) {
                    return "DIRECT";
                }
                return "PROXY fallback.local:3128; DIRECT";
            }
            "#,
        )
        .unwrap();

        assert_eq!(
            rule.select(&"http://www.example.com/".parse().unwrap()),
            Some("http://proxy.example.com:8080/".parse().unwrap())
        );
        assert_eq!(rule.select(&"http://10.1.2.3/".parse().unwrap()), None);
        assert_eq!(
            rule.select(&"http://other.local/".parse().unwrap()),
            Some("http://fallback.local:3128/".parse().unwrap())
        );
    }

    #[test]
    fn reports_a_script_without_the_entry_point() {
        let rule = PacRule::new("var x = 1;").unwrap();
        assert_eq!(rule.select(&"http://example.com/".parse().unwrap()), None);
    }

    #[test]
    fn reports_a_broken_script() {
        assert!(PacRule::new("this is not javascript {{{").is_err());
    }
}
