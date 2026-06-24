//! Content Security Policy (CSP3) parser, matcher, and per-fetch enforcement.
//!
//! Uses `winnow` combinators for parsing. Implements the subset of CSP3
//! needed for network-path enforcement: fetch directives, source-list
//! keywords, host/scheme/nonce/hash sources, default-src fallback,
//! strict-dynamic semantics.

use std::collections::HashMap;

use url::Url;
use winnow::{ascii::multispace0, prelude::*, token::take_while};

// ---------------------------------------------------------------------
// Directive
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Directive {
    DefaultSrc,
    ScriptSrc,
    ScriptSrcElem,
    ScriptSrcAttr,
    StyleSrc,
    StyleSrcElem,
    StyleSrcAttr,
    ImgSrc,
    ConnectSrc,
    FrameSrc,
    ChildSrc,
    FontSrc,
    MediaSrc,
    ObjectSrc,
    WorkerSrc,
    ManifestSrc,
    PrefetchSrc,
}

const DIRECTIVE_NAMES: &[(&str, Directive)] = &[
    ("default-src", Directive::DefaultSrc),
    ("script-src-elem", Directive::ScriptSrcElem),
    ("script-src-attr", Directive::ScriptSrcAttr),
    ("script-src", Directive::ScriptSrc),
    ("style-src-elem", Directive::StyleSrcElem),
    ("style-src-attr", Directive::StyleSrcAttr),
    ("style-src", Directive::StyleSrc),
    ("img-src", Directive::ImgSrc),
    ("connect-src", Directive::ConnectSrc),
    ("frame-src", Directive::FrameSrc),
    ("child-src", Directive::ChildSrc),
    ("font-src", Directive::FontSrc),
    ("media-src", Directive::MediaSrc),
    ("object-src", Directive::ObjectSrc),
    ("worker-src", Directive::WorkerSrc),
    ("manifest-src", Directive::ManifestSrc),
    ("prefetch-src", Directive::PrefetchSrc),
];

impl Directive {
    pub fn from_token(s: &str) -> Option<Self> {
        let lower = s.to_ascii_lowercase();
        DIRECTIVE_NAMES
            .iter()
            .find(|(name, _)| *name == lower.as_str())
            .map(|(_, d)| *d)
    }

    pub fn as_str(&self) -> &'static str {
        for (name, d) in DIRECTIVE_NAMES {
            if d == self {
                return name;
            }
        }
        unreachable!()
    }

    /// CSP3 section 6.6.1.1 — fallback chain for fetch directives.
    pub fn fallback_chain(&self) -> &'static [Directive] {
        use Directive::*;
        match self {
            ScriptSrcElem => &[ScriptSrcElem, ScriptSrc, DefaultSrc],
            ScriptSrcAttr => &[ScriptSrcAttr, ScriptSrc, DefaultSrc],
            ScriptSrc => &[ScriptSrc, DefaultSrc],
            StyleSrcElem => &[StyleSrcElem, StyleSrc, DefaultSrc],
            StyleSrcAttr => &[StyleSrcAttr, StyleSrc, DefaultSrc],
            StyleSrc => &[StyleSrc, DefaultSrc],
            FrameSrc => &[FrameSrc, ChildSrc, DefaultSrc],
            ChildSrc => &[ChildSrc, DefaultSrc],
            WorkerSrc => &[WorkerSrc, ChildSrc, ScriptSrc, DefaultSrc],
            ImgSrc | ConnectSrc | FontSrc | MediaSrc | ObjectSrc | ManifestSrc | PrefetchSrc => {
                // ponytail: single-element array via const; avoids per-call alloc
                const CHAIN: [Directive; 1] = [Directive::DefaultSrc];
                match self {
                    ImgSrc => &[ImgSrc, DefaultSrc],
                    ConnectSrc => &[ConnectSrc, DefaultSrc],
                    FontSrc => &[FontSrc, DefaultSrc],
                    MediaSrc => &[MediaSrc, DefaultSrc],
                    ObjectSrc => &[ObjectSrc, DefaultSrc],
                    ManifestSrc => &[ManifestSrc, DefaultSrc],
                    PrefetchSrc => &[PrefetchSrc, DefaultSrc],
                    _ => &CHAIN,
                }
            }
            DefaultSrc => &[DefaultSrc],
        }
    }
}

// ---------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashAlgo {
    Sha256,
    Sha384,
    Sha512,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    All,
    None_,
    Self_,
    UnsafeInline,
    UnsafeEval,
    UnsafeHashes,
    StrictDynamic,
    ReportSample,
    Scheme(String),
    Host(HostSource),
    Nonce(String),
    Hash(HashAlgo, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSource {
    pub scheme: Option<String>,
    pub host: HostPattern,
    pub port: Option<PortPattern>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostPattern {
    Wildcard(String),
    Exact(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortPattern {
    Wildcard,
    Exact(u16),
}

// ---------------------------------------------------------------------
// Winnow parsers (winnow 1.x API)
// ---------------------------------------------------------------------

type PResult<T> = winnow::Result<T, winnow::error::ContextError>;

fn parse_directive_name(input: &mut &str) -> PResult<Directive> {
    let name: &str =
        take_while(1.., |c: char| c.is_ascii_lowercase() || c == '-').parse_next(input)?;
    Directive::from_token(name).ok_or_else(winnow::error::ContextError::new)
}

fn parse_nonce_source(input: &mut &str) -> PResult<Source> {
    let _ = '\''.parse_next(input)?;
    let _ = "nonce-".parse_next(input)?;
    let value: &str = take_while(1.., |c: char| c != '\'').parse_next(input)?;
    let _ = '\''.parse_next(input)?;
    Ok(Source::Nonce(value.to_string()))
}

fn parse_hash_source(input: &mut &str) -> PResult<Source> {
    let _ = '\''.parse_next(input)?;
    let algo: &str = take_while(1.., |c: char| c.is_ascii_alphanumeric()).parse_next(input)?;
    let hash_algo = match algo {
        "sha256" => HashAlgo::Sha256,
        "sha384" => HashAlgo::Sha384,
        "sha512" => HashAlgo::Sha512,
        _ => return Err(winnow::error::ContextError::new()),
    };
    let _ = '-'.parse_next(input)?;
    let value: &str = take_while(1.., |c: char| c != '\'').parse_next(input)?;
    let _ = '\''.parse_next(input)?;
    Ok(Source::Hash(hash_algo, value.to_string()))
}

fn parse_host_pattern(raw: &str) -> HostPattern {
    if let Some(suffix) = raw.strip_prefix("*.") {
        HostPattern::Wildcard(suffix.to_ascii_lowercase())
    } else {
        HostPattern::Exact(raw.to_ascii_lowercase())
    }
}

fn parse_host_source_token(token: &str) -> Option<Source> {
    if token.starts_with('\'') {
        return None;
    }
    let mut rest = token;

    let mut scheme = None;
    if let Some(idx) = rest.find("://") {
        let s = &rest[..idx];
        if !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        {
            scheme = Some(s.to_ascii_lowercase());
            rest = &rest[idx + 3..];
        }
    }

    let (host_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], Some(rest[idx..].to_string())),
        None => (rest, None),
    };

    if host_port.is_empty() {
        return None;
    }

    let (host_part, port) = match host_port.rfind(':') {
        Some(idx) => {
            let port_str = &host_port[idx + 1..];
            if port_str == "*" {
                (&host_port[..idx], Some(PortPattern::Wildcard))
            } else if let Ok(n) = port_str.parse::<u16>() {
                (&host_port[..idx], Some(PortPattern::Exact(n)))
            } else {
                (host_port, None)
            }
        }
        None => (host_port, None),
    };

    if host_part.is_empty() {
        return None;
    }
    // Reject wildcards not at the start (e.g., "foo*bar").
    if host_part.contains('*') && !host_part.starts_with("*.") {
        return None;
    }
    // Validate host chars (strip leading wildcard before checking).
    let host_check = host_part.strip_prefix("*.").unwrap_or(host_part);
    if host_check != "*"
        && !host_check
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return None;
    }

    Some(Source::Host(HostSource {
        scheme,
        host: parse_host_pattern(host_part),
        port,
        path,
    }))
}

fn parse_source_token(input: &mut &str) -> PResult<Source> {
    let token: &str = take_while(1.., |c: char| !c.is_whitespace()).parse_next(input)?;

    let lower = token.to_ascii_lowercase();
    match lower.as_str() {
        "*" => return Ok(Source::All),
        "'none'" => return Ok(Source::None_),
        "'self'" => return Ok(Source::Self_),
        "'unsafe-inline'" => return Ok(Source::UnsafeInline),
        "'unsafe-eval'" => return Ok(Source::UnsafeEval),
        "'unsafe-hashes'" => return Ok(Source::UnsafeHashes),
        "'strict-dynamic'" => return Ok(Source::StrictDynamic),
        "'report-sample'" => return Ok(Source::ReportSample),
        _ => {}
    }

    // Nonce
    if let Some(rest) = token.strip_prefix("'nonce-") {
        if let Some(value) = rest.strip_suffix('\'') {
            return Ok(Source::Nonce(value.to_string()));
        }
    }

    // Hash
    for (algo, prefix) in [
        (HashAlgo::Sha256, "'sha256-"),
        (HashAlgo::Sha384, "'sha384-"),
        (HashAlgo::Sha512, "'sha512-"),
    ] {
        if let Some(rest) = token.strip_prefix(prefix) {
            if let Some(value) = rest.strip_suffix('\'') {
                return Ok(Source::Hash(algo, value.to_string()));
            }
        }
    }

    // Scheme-only: ends with ':'
    if let Some(scheme) = token.strip_suffix(':') {
        if !scheme.contains('/')
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        {
            return Ok(Source::Scheme(scheme.to_ascii_lowercase()));
        }
    }

    // Host source
    if let Some(src) = parse_host_source_token(token) {
        return Ok(src);
    }

    Err(winnow::error::ContextError::new())
}

fn parse_source_list(input: &mut &str) -> PResult<Vec<Source>> {
    let mut sources = Vec::new();
    loop {
        let _ = multispace0.parse_next(input)?;
        if input.is_empty() {
            break;
        }
        // Check if the next char starts a new directive (letter) or is a semicolon.
        if input.starts_with(';') {
            break;
        }
        match parse_source_token.parse_next(input) {
            Ok(src) => sources.push(src),
            Err(_) => break,
        }
    }
    Ok(sources)
}

fn parse_directive_line(input: &mut &str) -> PResult<(Directive, Vec<Source>)> {
    let _ = multispace0.parse_next(input)?;
    let directive = parse_directive_name.parse_next(input)?;
    let _ = multispace0.parse_next(input)?;
    let sources = parse_source_list.parse_next(input)?;
    Ok((directive, sources))
}

fn parse_policy_string(input: &mut &str) -> PResult<Vec<(Directive, Vec<Source>)>> {
    let mut pairs = Vec::new();
    loop {
        let _ = multispace0.parse_next(input)?;
        if input.is_empty() {
            break;
        }
        match parse_directive_line.parse_next(input) {
            Ok(pair) => pairs.push(pair),
            Err(_) => {
                // Skip unknown directive — consume until next semicolon or end.
                if let Some(idx) = input.find(';') {
                    *input = &input[idx + 1..];
                } else {
                    *input = &input[input.len()..];
                }
            }
        }
        // Skip separator
        let _ = multispace0.parse_next(input)?;
        if input.starts_with(';') {
            let _ = ';'.parse_next(input)?;
        }
    }
    Ok(pairs)
}

// ---------------------------------------------------------------------
// Policy / PolicySet
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Policy {
    pub directives: HashMap<Directive, Vec<Source>>,
    pub report_only: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PolicySet {
    pub policies: Vec<Policy>,
}

impl PolicySet {
    pub fn is_empty(&self) -> bool {
        self.policies.iter().all(|p| p.directives.is_empty())
    }

    /// Parse and push one or more policies from a header value.
    pub fn push_header(&mut self, value: &str, report_only: bool) {
        for piece in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let policy = Policy::parse_serialized(piece, report_only);
            if !policy.directives.is_empty() {
                self.policies.push(policy);
            }
        }
    }

    /// Parse and push a policy from a `<meta>` content attribute.
    pub fn push_meta(&mut self, content: &str) {
        for piece in content.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let policy = Policy::parse_serialized(piece, false);
            if !policy.directives.is_empty() {
                self.policies.push(policy);
            }
        }
    }
}

impl Policy {
    /// Parse a single serialized policy using winnow combinators.
    pub fn parse_serialized(s: &str, report_only: bool) -> Policy {
        let mut input = s;
        let pairs = parse_policy_string
            .parse_next(&mut input)
            .unwrap_or_default();
        let mut directives: HashMap<Directive, Vec<Source>> = HashMap::new();
        for (dir, sources) in pairs {
            directives.entry(dir).or_default().extend(sources);
        }
        Policy {
            directives,
            report_only,
        }
    }

    pub fn parse_header(s: &str) -> PolicySet {
        let mut set = PolicySet::default();
        set.push_header(s, false);
        set
    }

    pub fn parse_meta_content(s: &str) -> PolicySet {
        let mut set = PolicySet::default();
        set.push_meta(s);
        set
    }
}

// ---------------------------------------------------------------------
// CheckCtx / AllowDecision / Matcher
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CheckCtx<'a> {
    pub directive: Directive,
    pub url: &'a Url,
    pub page_origin: &'a Url,
    pub nonce: Option<&'a str>,
    pub parser_inserted: bool,
}

#[derive(Debug, Clone)]
pub struct AllowDecision {
    pub allowed: bool,
    pub matched_directive: Directive,
    pub report_only: bool,
}

impl AllowDecision {
    pub fn allow_no_policy() -> Self {
        Self {
            allowed: true,
            matched_directive: Directive::DefaultSrc,
            report_only: false,
        }
    }
}

impl PolicySet {
    /// Returns ALLOW only if every enforced policy allows.
    pub fn allows(&self, ctx: &CheckCtx<'_>) -> AllowDecision {
        if self.policies.is_empty() {
            return AllowDecision::allow_no_policy();
        }
        for policy in &self.policies {
            let decision = policy.allows(ctx);
            if !decision.allowed && !policy.report_only {
                return decision;
            }
        }
        AllowDecision::allow_no_policy()
    }
}

impl Policy {
    pub fn allows(&self, ctx: &CheckCtx<'_>) -> AllowDecision {
        for &candidate in ctx.directive.fallback_chain() {
            if let Some(sources) = self.directives.get(&candidate) {
                let allowed = match_sources(sources, ctx);
                return AllowDecision {
                    allowed,
                    matched_directive: candidate,
                    report_only: self.report_only,
                };
            }
        }
        AllowDecision::allow_no_policy()
    }
}

fn match_sources(sources: &[Source], ctx: &CheckCtx<'_>) -> bool {
    if sources.is_empty() {
        return false;
    }
    if sources.iter().all(|s| matches!(s, Source::None_)) {
        return false;
    }

    let strict_dynamic = is_script_directive(ctx.directive)
        && sources.iter().any(|s| matches!(s, Source::StrictDynamic));

    for src in sources {
        match src {
            Source::None_
            | Source::UnsafeInline
            | Source::UnsafeEval
            | Source::UnsafeHashes
            | Source::ReportSample
            | Source::StrictDynamic => continue,

            Source::All if !strict_dynamic => {
                if is_network_scheme(ctx.url.scheme()) {
                    return true;
                }
            }
            Source::All => continue,

            Source::Self_ if !strict_dynamic => {
                if is_same_origin(ctx.url, ctx.page_origin) {
                    return true;
                }
            }
            Source::Self_ => continue,

            Source::Scheme(s) if !strict_dynamic => {
                if ctx.url.scheme().eq_ignore_ascii_case(s) {
                    return true;
                }
            }
            Source::Scheme(_) => continue,

            Source::Host(h) if !strict_dynamic => {
                if host_source_matches(h, ctx.url) {
                    return true;
                }
            }
            Source::Host(_) => continue,

            Source::Nonce(token) => {
                if let Some(supplied) = ctx.nonce {
                    if supplied == token {
                        return true;
                    }
                }
            }

            Source::Hash(_, _) => continue,
        }
    }
    false
}

fn is_script_directive(d: Directive) -> bool {
    matches!(
        d,
        Directive::ScriptSrc | Directive::ScriptSrcElem | Directive::ScriptSrcAttr
    )
}

fn is_network_scheme(scheme: &str) -> bool {
    matches!(scheme, "http" | "https" | "ws" | "wss" | "ftp" | "ftps")
}

fn is_same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

fn host_source_matches(src: &HostSource, url: &Url) -> bool {
    if let Some(want) = &src.scheme {
        if !url.scheme().eq_ignore_ascii_case(want) {
            return false;
        }
    } else if !is_network_scheme(url.scheme()) {
        return false;
    }

    let url_host = match url.host_str() {
        Some(h) => h.to_ascii_lowercase(),
        None => return false,
    };
    let host_ok = match &src.host {
        HostPattern::Exact(want) => want == "*" || want == &url_host,
        HostPattern::Wildcard(suffix) => {
            url_host.ends_with(suffix)
                && url_host.len() > suffix.len()
                && url_host.chars().nth(url_host.len() - suffix.len() - 1) == Some('.')
        }
    };
    if !host_ok {
        return false;
    }

    let url_port = url.port_or_known_default();
    if let Some(p) = &src.port {
        match p {
            PortPattern::Wildcard => {}
            PortPattern::Exact(n) => {
                if url_port != Some(*n) {
                    return false;
                }
            }
        }
    } else {
        let default_port = match url.scheme() {
            "http" | "ws" | "ftp" => Some(80),
            "https" | "wss" | "ftps" => Some(443),
            _ => None,
        };
        if url_port != default_port {
            return false;
        }
    }

    true
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const WALMART_CSP: &str = "child-src 'self' blob:; \
        connect-src 'self' *.akamaihd.net *.perimeterx.net; \
        script-src 'self' 'strict-dynamic' 'nonce-MRjHHgrLk9lNoNBv' *.walmartimages.com; \
        style-src 'self' 'unsafe-inline' *.walmartimages.com; \
        img-src 'self' data: *.walmartimages.com *.scene7.com; \
        frame-src 'self' *.youtube.com";

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn ctx<'a>(
        directive: Directive,
        u: &'a Url,
        origin: &'a Url,
        nonce: Option<&'a str>,
        parser_inserted: bool,
    ) -> CheckCtx<'a> {
        CheckCtx {
            directive,
            url: u,
            page_origin: origin,
            nonce,
            parser_inserted,
        }
    }

    #[test]
    fn parses_retailer_csp_directives() {
        let set = Policy::parse_meta_content(WALMART_CSP);
        assert_eq!(set.policies.len(), 1);
        let p = &set.policies[0];
        assert!(!p.report_only);
        assert!(p.directives.contains_key(&Directive::ScriptSrc));
        assert!(p.directives.contains_key(&Directive::ConnectSrc));
        assert!(p.directives.contains_key(&Directive::FrameSrc));
    }

    #[test]
    fn parses_strict_dynamic_and_nonce() {
        let set = Policy::parse_meta_content(WALMART_CSP);
        let script_src = &set.policies[0].directives[&Directive::ScriptSrc];
        assert!(
            script_src
                .iter()
                .any(|s| matches!(s, Source::StrictDynamic))
        );
        assert!(
            script_src
                .iter()
                .any(|s| matches!(s, Source::Nonce(n) if n == "MRjHHgrLk9lNoNBv"))
        );
        assert!(script_src.iter().any(|s| matches!(s, Source::Self_)));
    }

    #[test]
    fn parses_host_source_with_subdomain_wildcard() {
        let set = Policy::parse_meta_content("connect-src *.example.com:8443");
        let cs = &set.policies[0].directives[&Directive::ConnectSrc];
        assert_eq!(cs.len(), 1);
        let Source::Host(h) = &cs[0] else {
            panic!("expected host source")
        };
        assert_eq!(h.host, HostPattern::Wildcard("example.com".to_string()));
        assert_eq!(h.port, Some(PortPattern::Exact(8443)));
    }

    #[test]
    fn parses_scheme_only_source() {
        let set = Policy::parse_meta_content("img-src data: blob: https:");
        let img = &set.policies[0].directives[&Directive::ImgSrc];
        assert_eq!(img.len(), 3);
        assert!(
            img.iter()
                .any(|s| matches!(s, Source::Scheme(x) if x == "data"))
        );
        assert!(
            img.iter()
                .any(|s| matches!(s, Source::Scheme(x) if x == "blob"))
        );
        assert!(
            img.iter()
                .any(|s| matches!(s, Source::Scheme(x) if x == "https"))
        );
    }

    #[test]
    fn parses_hash_sources() {
        let set =
            Policy::parse_meta_content("script-src 'sha256-abc123==' 'sha384-XYZ' 'sha512-q+w'");
        let ss = &set.policies[0].directives[&Directive::ScriptSrc];
        assert_eq!(ss.len(), 3);
        assert!(matches!(&ss[0], Source::Hash(HashAlgo::Sha256, h) if h == "abc123=="));
        assert!(matches!(&ss[1], Source::Hash(HashAlgo::Sha384, h) if h == "XYZ"));
        assert!(matches!(&ss[2], Source::Hash(HashAlgo::Sha512, h) if h == "q+w"));
    }

    #[test]
    fn parses_none_keyword() {
        let set = Policy::parse_meta_content("object-src 'none'");
        let os = &set.policies[0].directives[&Directive::ObjectSrc];
        assert_eq!(os.len(), 1);
        assert!(matches!(&os[0], Source::None_));
    }

    #[test]
    fn parses_multiple_policies_from_one_header() {
        let mut set = PolicySet::default();
        set.push_header("script-src 'self', script-src https:", false);
        assert_eq!(set.policies.len(), 2);
    }

    #[test]
    fn report_only_flag_propagates() {
        let mut set = PolicySet::default();
        set.push_header("script-src 'self'", true);
        assert!(set.policies[0].report_only);
    }

    #[test]
    fn unknown_directive_is_dropped() {
        let set = Policy::parse_meta_content("script-src 'self'; bogus-thing 'self'");
        assert_eq!(set.policies[0].directives.len(), 1);
    }

    #[test]
    fn fallback_chain_script_src_elem() {
        let chain = Directive::ScriptSrcElem.fallback_chain();
        assert_eq!(
            chain,
            &[
                Directive::ScriptSrcElem,
                Directive::ScriptSrc,
                Directive::DefaultSrc
            ]
        );
    }

    #[test]
    fn fallback_chain_frame_src() {
        let chain = Directive::FrameSrc.fallback_chain();
        assert_eq!(
            chain,
            &[
                Directive::FrameSrc,
                Directive::ChildSrc,
                Directive::DefaultSrc
            ]
        );
    }

    #[test]
    fn empty_policy_set_allows_everything() {
        let set = PolicySet::default();
        let u = url("https://akamai.com/sensor.js");
        let origin = url("https://www.walmart.com/");
        let d = set.allows(&ctx(Directive::ScriptSrcElem, &u, &origin, None, true));
        assert!(d.allowed);
    }

    #[test]
    fn self_matches_same_origin() {
        let set = Policy::parse_meta_content("script-src 'self'");
        let origin = url("https://example.com/");
        let same = url("https://example.com/app.js");
        let other = url("https://other.com/x.js");
        assert!(
            set.allows(&ctx(Directive::ScriptSrcElem, &same, &origin, None, true))
                .allowed
        );
        assert!(
            !set.allows(&ctx(Directive::ScriptSrcElem, &other, &origin, None, true))
                .allowed
        );
    }

    #[test]
    fn host_wildcard_matches_subdomain_only() {
        let set = Policy::parse_meta_content("img-src *.example.com");
        let origin = url("https://example.com/");
        let sub = url("https://images.example.com/a.png");
        let bare = url("https://example.com/a.png");
        assert!(
            set.allows(&ctx(Directive::ImgSrc, &sub, &origin, None, false))
                .allowed
        );
        assert!(
            !set.allows(&ctx(Directive::ImgSrc, &bare, &origin, None, false))
                .allowed
        );
    }

    #[test]
    fn scheme_only_source_matches() {
        let set = Policy::parse_meta_content("img-src data: https:");
        let origin = url("https://example.com/");
        let data = url("data:image/png;base64,iVBORw0K");
        let any_https = url("https://random.cdn.net/x.png");
        let http = url("http://random.cdn.net/x.png");
        assert!(
            set.allows(&ctx(Directive::ImgSrc, &data, &origin, None, false))
                .allowed
        );
        assert!(
            set.allows(&ctx(Directive::ImgSrc, &any_https, &origin, None, false))
                .allowed
        );
        assert!(
            !set.allows(&ctx(Directive::ImgSrc, &http, &origin, None, false))
                .allowed
        );
    }

    #[test]
    fn none_blocks_everything() {
        let set = Policy::parse_meta_content("object-src 'none'");
        let origin = url("https://example.com/");
        let any = url("https://example.com/x.swf");
        assert!(
            !set.allows(&ctx(Directive::ObjectSrc, &any, &origin, None, false))
                .allowed
        );
    }

    #[test]
    fn fallback_chain_uses_default_src() {
        let set = Policy::parse_meta_content("default-src 'self'");
        let origin = url("https://example.com/");
        let self_url = url("https://example.com/x.png");
        let other = url("https://other.com/x.png");
        assert!(
            set.allows(&ctx(Directive::ImgSrc, &self_url, &origin, None, false))
                .allowed
        );
        assert!(
            !set.allows(&ctx(Directive::ImgSrc, &other, &origin, None, false))
                .allowed
        );
    }

    #[test]
    fn nonce_authorizes_under_normal_policy() {
        let set = Policy::parse_meta_content("script-src 'nonce-abc123'");
        let origin = url("https://example.com/");
        let any = url("https://cdn.elsewhere.com/app.js");
        assert!(
            set.allows(&ctx(
                Directive::ScriptSrcElem,
                &any,
                &origin,
                Some("abc123"),
                true
            ))
            .allowed
        );
        assert!(
            !set.allows(&ctx(
                Directive::ScriptSrcElem,
                &any,
                &origin,
                Some("WRONG"),
                true
            ))
            .allowed
        );
    }

    #[test]
    fn strict_dynamic_blocks_parser_injected_script() {
        let set = Policy::parse_meta_content(WALMART_CSP);
        let origin = url("https://www.walmart.com/");
        let injected = url("https://www.walmart.com/akam/13/3e35295b");

        let d = set.allows(&ctx(
            Directive::ScriptSrcElem,
            &injected,
            &origin,
            None,
            true,
        ));
        assert!(!d.allowed);
        assert_eq!(d.matched_directive, Directive::ScriptSrc);

        let d = set.allows(&ctx(
            Directive::ScriptSrcElem,
            &injected,
            &origin,
            Some("MRjHHgrLk9lNoNBv"),
            true,
        ));
        assert!(d.allowed);
    }

    #[test]
    fn strict_dynamic_ignores_self_and_host_allowlist() {
        let set = Policy::parse_meta_content(WALMART_CSP);
        let origin = url("https://www.walmart.com/");
        let images = url("https://i5.walmartimages.com/foo.js");
        assert!(
            !set.allows(&ctx(Directive::ScriptSrcElem, &images, &origin, None, true))
                .allowed
        );
        assert!(
            set.allows(&ctx(
                Directive::ScriptSrcElem,
                &images,
                &origin,
                Some("MRjHHgrLk9lNoNBv"),
                true,
            ))
            .allowed
        );
    }

    #[test]
    fn strict_dynamic_does_not_apply_to_non_script() {
        let set = Policy::parse_meta_content(WALMART_CSP);
        let origin = url("https://www.walmart.com/");
        let img = url("https://i5.walmartimages.com/foo.png");
        assert!(
            set.allows(&ctx(Directive::ImgSrc, &img, &origin, None, false))
                .allowed
        );
    }

    #[test]
    fn host_with_wildcard_port_matches_any() {
        let set = Policy::parse_meta_content("connect-src example.com:*");
        let origin = url("https://other.com/");
        let p443 = url("https://example.com/x");
        let p8443 = url("https://example.com:8443/x");
        assert!(
            set.allows(&ctx(Directive::ConnectSrc, &p443, &origin, None, false))
                .allowed
        );
        assert!(
            set.allows(&ctx(Directive::ConnectSrc, &p8443, &origin, None, false))
                .allowed
        );
    }

    #[test]
    fn host_without_port_matches_only_default_port() {
        let set = Policy::parse_meta_content("connect-src example.com");
        let origin = url("https://other.com/");
        let p443 = url("https://example.com/x");
        let p8443 = url("https://example.com:8443/x");
        assert!(
            set.allows(&ctx(Directive::ConnectSrc, &p443, &origin, None, false))
                .allowed
        );
        assert!(
            !set.allows(&ctx(Directive::ConnectSrc, &p8443, &origin, None, false))
                .allowed
        );
    }

    #[test]
    fn report_only_policy_never_blocks() {
        let mut set = PolicySet::default();
        set.push_header("script-src 'none'", true);
        let origin = url("https://example.com/");
        let any = url("https://example.com/x.js");
        assert!(
            set.allows(&ctx(Directive::ScriptSrcElem, &any, &origin, None, true))
                .allowed
        );
    }

    #[test]
    fn multiple_policies_intersect() {
        let mut set = PolicySet::default();
        set.push_header("script-src 'self' https://cdn.com", false);
        set.push_header("script-src 'self'", false);
        let origin = url("https://example.com/");
        let cdn = url("https://cdn.com/x.js");
        let self_url = url("https://example.com/x.js");
        assert!(
            !set.allows(&ctx(Directive::ScriptSrcElem, &cdn, &origin, None, true))
                .allowed
        );
        assert!(
            set.allows(&ctx(
                Directive::ScriptSrcElem,
                &self_url,
                &origin,
                None,
                true
            ))
            .allowed
        );
    }

    #[test]
    fn parses_bare_star_host() {
        let set = Policy::parse_meta_content("img-src *");
        let img = &set.policies[0].directives[&Directive::ImgSrc];
        assert!(matches!(&img[0], Source::All));
    }

    #[test]
    fn empty_source_list_blocks() {
        let set = Policy::parse_meta_content("script-src");
        let ss = &set.policies[0].directives[&Directive::ScriptSrc];
        assert_eq!(ss.len(), 0);
    }

    // ---- Property tests: parse arbitrary CSP headers without panic ----

    #[test]
    fn prop_parse_arbitrary_csp_strings_no_panic() {
        let cases = [
            "",
            ";",
            ";;;",
            "script-src",
            "script-src 'self'",
            "default-src *; script-src 'none'",
            "img-src data: blob: https: http:",
            "connect-src *.example.com:443 wss://ws.example.com",
            "script-src 'nonce-abc' 'sha256-hash' 'strict-dynamic'",
            "font-src 'self' https://fonts.gstatic.com",
            "object-src 'none'; frame-src 'self' https://www.youtube.com",
            "style-src 'self' 'unsafe-inline'; style-src-elem 'self'",
            "worker-src blob: 'self'; child-src blob:",
            "default-src 'self'; script-src 'self' 'unsafe-eval' 'unsafe-inline'",
            "prefetch-src 'self'; manifest-src 'self'",
            "script-src 'self' *.cdn.example.com:8443/path",
            "img-src https: http: data: blob:",
            "connect-src *",
            "script-src 'sha256-abc123==' 'sha384-XYZ' 'sha512-q+w'",
            "default-src 'none'; script-src 'nonce-base64token==' 'self'",
            // Edge cases
            "bogus-directive 'self'",
            "script-src 'nonce-",
            "script-src 'sha256-",
            "script-src bad://host",
            "img-src *:8080",
            "connect-src example.com:*",
            "script-src 'nonce-' 'sha256-' 'unknown-keyword'",
            ";;; script-src;;; ;;;",
            &format!("script-src {}", "a ".repeat(200)),
        ];

        for input in &cases {
            let set = Policy::parse_meta_content(input);
            // Must not panic; also exercise the matcher.
            let origin = url("https://example.com/");
            let test_url = url("https://example.com/x");
            let _ = set.allows(&ctx(
                Directive::ScriptSrcElem,
                &test_url,
                &origin,
                None,
                true,
            ));
        }
    }

    #[cfg(feature = "proptest")]
    mod proptest_tests {
        use proptest::prelude::*;

        use super::*;

        proptest! {
            #[test]
            fn parse_arbitrary_csp_does_not_panic(s in ".*{0,500}") {
                let set = Policy::parse_meta_content(&s);
                let origin = url("https://example.com/");
                let test_url = url("https://example.com/x");
                let _ = set.allows(&ctx(Directive::ScriptSrcElem, &test_url, &origin, None, true));
                let _ = set.allows(&ctx(Directive::ImgSrc, &test_url, &origin, None, false));
                let _ = set.allows(&ctx(Directive::ConnectSrc, &test_url, &origin, None, false));
            }

            #[test]
            fn parse_directive_sources_no_panic(
                directive in "(default-src|script-src|img-src|connect-src|style-src|frame-src|font-src|object-src|worker-src|media-src|child-src|manifest-src|prefetch-src|script-src-elem|script-src-attr|style-src-elem|style-src-attr)",
                sources in "([^;]{0,200})"
            ) {
                let input = format!("{directive} {sources}");
                let set = Policy::parse_meta_content(&input);
                let origin = url("https://example.com/");
                let test_url = url("https://example.com/x");
                let _ = set.allows(&ctx(Directive::ScriptSrcElem, &test_url, &origin, Some("abc"), true));
            }
        }
    }
}
