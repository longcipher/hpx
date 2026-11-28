//! Local wire-fingerprint self-check for browser emulation profiles.
//!
//! A profile is only useful if the bytes it puts on the wire still match the
//! browser version it claims to emulate. This module turns the observable
//! parts of a request — the ordered HTTP/1.1 header names with their original
//! case, and the HTTP/2 SETTINGS / WINDOW_UPDATE / PRIORITY / pseudo-header
//! order — into two deterministic strings that CI can compare against golden
//! values. A change to a profile generator, a header composer, or a SETTINGS
//! default then shows up as an explicit diff instead of silently degrading
//! anti-bot fidelity.
//!
//! # Relationship to the published JA4H format
//!
//! The FoxIO JA4+ suite (including JA4H) is distributed under the FoxIO
//! License 1.1, which is non-commercial and requires an OEM license for
//! commercial use. This workspace is Apache-2.0 and is used in commercial
//! trading systems, so no FoxIO code, assets, or trademarks are included here.
//!
//! [`Ja4h`] is an *independent implementation of the published JA4H format*:
//! the `a_b_c_d` layout and its inputs are public documentation, and the
//! implementation below was written from that description. It exists so a
//! client can be checked against the fingerprint a server would compute.
//! [`HeaderSignature`] is hpx's own canonicalisation — a readable prefix plus
//! a truncated SHA-256 of the ordered header names — and is a regression
//! guard, not a claim of cross-implementation conformance.
//! [`Http2Signature`] uses the widely published Akamai HTTP/2 format
//! `S[;]|WU|P[,]#|PS[,]`.
//!
//! # Example
//!
//! ```
//! use hpx_emulation::verify::{HeaderSignature, Http2Signature};
//!
//! let headers = HeaderSignature::new("GET", "11", &["Host", "User-Agent", "Accept"]);
//! assert_eq!(headers.prefix(), "ge11nn030000");
//!
//! let h2 = Http2Signature::new(
//!     &[(1, 65536), (2, 0)],
//!     Some(15663105),
//!     &[],
//!     &[":method", ":path"],
//! );
//! assert!(h2.render().contains("|15663105|"));
//! ```

use sha2::{Digest, Sha256};

/// Truncate a SHA-256 digest to its first 12 lowercase hex characters.
fn short_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(12);
    for byte in digest.iter().take(6) {
        use std::fmt::Write as _;
        // `write!` on a `String` cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Canonical signature of the HTTP/1.1 header block.
///
/// The prefix encodes, in order: the first two lowercased letters of the
/// method, the two-digit HTTP version tag, `c`/`n` for a Cookie header,
/// `r`/`n` for a Referer header, a two-digit header count (excluding Cookie
/// and Referer), and a fixed `0000` tail.
///
/// The `0000` tail is the `Accept-Language` field of the published JA4H
/// layout. This signature does not compute it — use [`Ja4h`] for the
/// language-aware fingerprint of the same request.
///
/// The hash covers the ordered, case-preserving header names joined by `,`.
/// Changing the order or the case of a single header therefore changes the
/// signature, which is the property that catches WAF-visible regressions.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HeaderSignature {
    prefix: String,
    hash: String,
}

impl HeaderSignature {
    /// Compute a signature from wire-ordered header names (original case).
    ///
    /// `headers` must contain the header names as they appear on the wire;
    /// `Cookie` and `Referer` are detected case-insensitively.
    #[must_use]
    pub fn new(method: &str, version: &str, headers: &[&str]) -> Self {
        let mut method_short = method
            .chars()
            .take(2)
            .flat_map(char::to_lowercase)
            .collect::<String>();
        while method_short.len() < 2 {
            method_short.push('n');
        }

        let mut has_cookie = false;
        let mut has_referer = false;
        let mut counted = 0usize;
        let mut joined = String::new();

        for (index, name) in headers.iter().enumerate() {
            if index > 0 {
                joined.push(',');
            }
            joined.push_str(name);
            if name.eq_ignore_ascii_case("cookie") {
                has_cookie = true;
            } else if name.eq_ignore_ascii_case("referer") {
                has_referer = true;
            } else {
                counted += 1;
            }
        }

        let prefix = format!(
            "{method_short}{version}{}{}{:02}0000",
            if has_cookie { 'c' } else { 'n' },
            if has_referer { 'r' } else { 'n' },
            counted.min(99)
        );
        Self {
            prefix,
            hash: short_hash(joined.as_bytes()),
        }
    }

    /// The readable prefix (everything before the hash).
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The 12-character truncated SHA-256 of the ordered header names.
    #[must_use]
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// Render as `{prefix}_{hash}`.
    #[must_use]
    pub fn render(&self) -> String {
        format!("{}_{}", self.prefix, self.hash)
    }
}

/// Canonical signature of an HTTP/2 connection preface, in Akamai's
/// `S[;]|WU|P[,]#|PS[,]` shape.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Http2Signature {
    settings: Vec<(u16, u32)>,
    window_update: Option<u32>,
    priorities: Vec<(u32, bool, u32, u8)>,
    pseudo_order: Vec<String>,
}

impl Http2Signature {
    /// Build a signature from the frames a client would actually send.
    #[must_use]
    pub fn new(
        settings: &[(u16, u32)],
        window_update: Option<u32>,
        priorities: &[(u32, bool, u32, u8)],
        pseudo_order: &[&str],
    ) -> Self {
        Self {
            settings: settings.to_vec(),
            window_update,
            priorities: priorities.to_vec(),
            pseudo_order: pseudo_order.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    /// Render the Akamai-style fingerprint string.
    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write as _;

        let mut out = String::new();
        for (index, (id, value)) in self.settings.iter().enumerate() {
            if index > 0 {
                out.push(';');
            }
            let _ = write!(out, "{id}:{value}");
        }
        out.push('|');
        match self.window_update {
            Some(value) => {
                let _ = write!(out, "{value}");
            }
            None => out.push_str("00"),
        }
        out.push('|');
        if self.priorities.is_empty() {
            out.push('0');
        } else {
            for (index, (stream, exclusive, depends_on, weight)) in
                self.priorities.iter().enumerate()
            {
                if index > 0 {
                    out.push(',');
                }
                let _ = write!(
                    out,
                    "{}:{}:{}:{}",
                    stream,
                    u8::from(*exclusive),
                    depends_on,
                    weight
                );
            }
        }
        out.push('|');
        out.push_str(&self.pseudo_order.join(","));
        out
    }

    /// The SETTINGS pairs in wire order.
    #[must_use]
    pub fn settings(&self) -> &[(u16, u32)] {
        &self.settings
    }
}

/// HTTP protocol version as encoded in the JA4H prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HttpVersion {
    /// HTTP/1.0 → `10`.
    Http10,
    /// HTTP/1.1 → `11`.
    Http11,
    /// HTTP/2 → `20`.
    Http2,
    /// HTTP/3 → `30`.
    Http3,
}

impl HttpVersion {
    const fn code(self) -> &'static str {
        match self {
            Self::Http10 => "10",
            Self::Http11 => "11",
            Self::Http2 => "20",
            Self::Http3 => "30",
        }
    }
}

/// JA4H: an application-layer HTTP request fingerprint.
///
/// The format is `a_b_c_d`: `a` is a human-readable prefix and `b`, `c`, `d`
/// are 12-character truncated SHA-256 hashes. Header names in wire order
/// (case-sensitive, excluding `Cookie` and `Referer`) form `b`; sorted cookie
/// names form `c`; sorted `name=value` pairs form `d`.
///
/// This is an independent implementation of the published format; no FoxIO
/// code or trademarked assets are included.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ja4h {
    prefix: String,
    header_names: String,
    cookie_names: String,
    cookie_pairs: String,
}

impl Ja4h {
    /// Compute a JA4H from one HTTP request.
    ///
    /// `headers` must be in wire order with original case. `Cookie` and
    /// `Referer` are represented in the prefix and excluded from the header
    /// hash; pass cookie pairs separately. `accept_language` is the raw value
    /// of the `Accept-Language` header (only the primary tag is used).
    #[must_use]
    pub fn new(
        method: &str,
        version: HttpVersion,
        headers: &[&str],
        cookies: &[(&str, &str)],
        accept_language: Option<&str>,
    ) -> Self {
        let has_cookie = if cookies.is_empty() { 'n' } else { 'c' };
        let has_referer = if headers
            .iter()
            .any(|name| name.eq_ignore_ascii_case("referer"))
        {
            'r'
        } else {
            'n'
        };

        let counted: Vec<&str> = headers
            .iter()
            .copied()
            .filter(|name| {
                !name.eq_ignore_ascii_case("cookie") && !name.eq_ignore_ascii_case("referer")
            })
            .collect();
        let count = counted.len().min(99);
        let language = primary_language(accept_language);
        let prefix = format!(
            "{}{}{has_cookie}{has_referer}{count:02}{language}",
            method_code(method),
            version.code()
        );

        let header_names = if counted.is_empty() {
            "000000000000".to_owned()
        } else {
            short_hash(counted.join(",").as_bytes())
        };

        let (cookie_names, cookie_pairs) = if cookies.is_empty() {
            ("000000000000".to_owned(), "000000000000".to_owned())
        } else {
            let mut pairs: Vec<(&str, &str)> = cookies.to_vec();
            pairs.sort_unstable_by(|left, right| left.0.cmp(right.0));
            let names: Vec<&str> = pairs.iter().map(|(name, _)| *name).collect();
            let values: Vec<String> = pairs
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            (
                short_hash(names.join(",").as_bytes()),
                short_hash(values.join(",").as_bytes()),
            )
        };

        Self {
            prefix,
            header_names,
            cookie_names,
            cookie_pairs,
        }
    }

    /// The human-readable `a` section.
    #[inline]
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The ordered header-name hash (`b` section).
    #[inline]
    #[must_use]
    pub fn header_names(&self) -> &str {
        &self.header_names
    }

    /// The sorted cookie-name hash (`c` section).
    #[inline]
    #[must_use]
    pub fn cookie_names(&self) -> &str {
        &self.cookie_names
    }

    /// The sorted cookie name+value hash (`d` section).
    #[inline]
    #[must_use]
    pub fn cookie_pairs(&self) -> &str {
        &self.cookie_pairs
    }

    /// Render the complete fingerprint.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "{}_{}_{}_{}",
            self.prefix, self.header_names, self.cookie_names, self.cookie_pairs
        )
    }
}

fn method_code(method: &str) -> &'static str {
    match method.to_ascii_uppercase().as_str() {
        "GET" => "ge",
        "HEAD" => "he",
        "OPTIONS" => "op",
        "TRACE" => "tr",
        "DELETE" => "de",
        "PUT" => "pu",
        "POST" => "po",
        "PATCH" => "pa",
        "CONNECT" => "co",
        _ => "00",
    }
}

fn primary_language(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "0000".to_owned();
    };
    let primary = value.split(',').next().unwrap_or_default().trim();
    let filtered: String = primary
        .chars()
        .filter(|c| *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    let mut out: String = filtered.chars().take(4).collect();
    while out.len() < 4 {
        out.push('0');
    }
    out
}

/// A single field-level mismatch between two signatures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureDiff {
    /// Which part of the wire changed.
    pub field: &'static str,
    /// The recorded (golden) value.
    pub expected: String,
    /// The value produced by the current profile.
    pub actual: String,
}

/// Compare a recorded wire signature against a freshly computed one.
///
/// An empty vector means the profile still produces byte-for-byte the same
/// observable shape. This is the assertion CI should use.
#[must_use]
pub fn diff(expected: &str, actual: &str) -> Vec<SignatureDiff> {
    if expected == actual {
        return Vec::new();
    }
    vec![SignatureDiff {
        field: "wire_signature",
        expected: expected.to_owned(),
        actual: actual.to_owned(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_prefix_encodes_method_version_and_flags() {
        let plain = HeaderSignature::new("GET", "11", &["Host", "User-Agent", "Accept"]);
        assert_eq!(plain.prefix(), "ge11nn030000");

        let with_all = HeaderSignature::new(
            "POST",
            "20",
            &["Cookie", "Referer", "Host", "Accept-Language"],
        );
        assert_eq!(with_all.prefix(), "po20cr020000");
    }

    #[test]
    fn header_hash_is_order_and_case_sensitive() {
        let a = HeaderSignature::new("GET", "11", &["Host", "User-Agent", "Accept"]);
        let b = HeaderSignature::new("GET", "11", &["Host", "Accept", "User-Agent"]);
        let c = HeaderSignature::new("GET", "11", &["host", "User-Agent", "Accept"]);
        assert_ne!(a.hash(), b.hash());
        assert_ne!(a.hash(), c.hash());
        assert_eq!(a.hash().len(), 12);
    }

    #[test]
    fn header_hash_is_stable_and_hex() {
        let a = HeaderSignature::new("GET", "11", &["Host", "Accept"]);
        let b = HeaderSignature::new("GET", "11", &["Host", "Accept"]);
        assert_eq!(a.render(), b.render());
        assert!(a.hash().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn header_count_is_capped_at_two_digits() {
        let names: Vec<&str> = (0..150).map(|_| "X-Custom").collect();
        let signature = HeaderSignature::new("GET", "11", &names);
        assert!(signature.prefix().ends_with("990000"));
    }

    #[test]
    fn h2_signature_renders_akamai_shape() {
        let signature = Http2Signature::new(
            &[(1, 65536), (2, 0), (4, 6291456)],
            Some(15663105),
            &[],
            &[":method", ":authority", ":scheme", ":path"],
        );
        assert_eq!(
            signature.render(),
            "1:65536;2:0;4:6291456|15663105|0|:method,:authority,:scheme,:path"
        );
    }

    #[test]
    fn h2_signature_handles_missing_window_and_priorities() {
        let signature = Http2Signature::new(&[(1, 65536)], None, &[], &[":method"]);
        assert_eq!(signature.render(), "1:65536|00|0|:method");
    }

    #[test]
    fn h2_signature_renders_priorities() {
        let signature = Http2Signature::new(
            &[(1, 65536)],
            Some(15663105),
            &[(3, false, 0, 200), (5, true, 3, 100)],
            &[":method"],
        );
        assert_eq!(
            signature.render(),
            "1:65536|15663105|3:0:0:200,5:1:3:100|:method"
        );
    }

    #[test]
    fn settings_order_changes_the_signature() {
        let a = Http2Signature::new(&[(1, 65536), (2, 0)], None, &[], &[":method"]);
        let b = Http2Signature::new(&[(2, 0), (1, 65536)], None, &[], &[":method"]);
        assert_ne!(a.render(), b.render());
        assert_eq!(a.settings(), &[(1, 65536), (2, 0)]);
    }

    #[test]
    fn diff_reports_only_when_signatures_diverge() {
        assert!(diff("abc_def", "abc_def").is_empty());
        let found = diff("abc_def", "abc_012");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].field, "wire_signature");
        assert_eq!(found[0].expected, "abc_def");
        assert_eq!(found[0].actual, "abc_012");
    }

    #[test]
    fn short_hash_is_first_twelve_hex_of_sha256() {
        assert_eq!(short_hash(b"abc"), "ba7816bf8f01");
        assert_eq!(short_hash(b""), "e3b0c44298fc");
    }

    use super::{HttpVersion, Ja4h};

    /// The published JA4H example: an HTTP/1.1 POST with twelve ordered
    /// header names — ten of them counted, plus `Cookie` and `Referer` — and
    /// five cookies. The golden hashes are the published values.
    #[test]
    fn ja4h_matches_published_golden_values() {
        let headers = [
            "Host",
            "Content-Length",
            "Accept",
            "X-Requested-With",
            "User-Agent",
            "Content-Type",
            "Origin",
            "Accept-Encoding",
            "Accept-Language",
            "Connection",
            "Referer",
            "Cookie",
        ];
        let cookies = [
            ("1P_JAR", "2023-06-07-17"),
            (
                "AEC",
                "AUEFqZdaLLwaXJHyxA8-Cu0i0N4klp_vV3XOuyEYeiWlp4QaeIvSv6t4XKM",
            ),
            ("OGPC", "19027681-1:"),
            (
                "NID",
                "511=rRELE2o91XNLo6eayqEN7Lf2ue7EcSHVkew3oxf4jzyF8vix2BzxTRvda8MYBFEkLyC1xjTcqSIjbC-wV2r120jr2HFau_dHvMxUm9fk6W2J2mddtlMpGMA8qGuAZWt1DSpCFFwHZSKBryGnvRJUeXkc-jw4sXdWhgCKxeu3f01Na4YsBYGf",
            ),
            ("DV", "A84BtBIPqhgmIDlq9acmfs7ik-duiZjdmUPDG3eW3QIAAAA"),
        ];
        let fp = Ja4h::new(
            "POST",
            HttpVersion::Http11,
            &headers,
            &cookies,
            Some("en-US,en;q=0.9"),
        );
        assert_eq!(fp.header_names(), "47d05ed57293");
        assert_eq!(fp.cookie_names(), "21864220ae3d");
        assert_eq!(fp.cookie_pairs(), "e97928733c74");
        assert_eq!(fp.prefix(), "po11cr10enus");
        assert_eq!(fp.render().split('_').count(), 4);
        assert_eq!(fp.render().len(), 12 + 1 + 12 + 1 + 12 + 1 + 12);
    }

    #[test]
    fn ja4h_cookie_hashes_are_sorted_by_name() {
        let cookies = [("z", "last"), ("a", "first"), ("m", "middle")];
        let fp = Ja4h::new("GET", HttpVersion::Http2, &[], &cookies, None);
        assert_eq!(fp.prefix(), "ge20cn000000");
        assert_ne!(fp.cookie_names(), "000000000000");
        assert_ne!(fp.cookie_pairs(), "000000000000");

        let mut reversed = cookies;
        reversed.reverse();
        let same = Ja4h::new("GET", HttpVersion::Http2, &[], &reversed, None);
        assert_eq!(fp.render(), same.render(), "cookie order must not matter");
    }

    #[test]
    fn ja4h_prefix_encodes_method_version_cookie_referer_and_language() {
        let fp = Ja4h::new(
            "GET",
            HttpVersion::Http2,
            &["Host", "Referer", "Accept-Language"],
            &[],
            Some("da, en-GB;q=0.8"),
        );
        assert_eq!(fp.prefix(), "ge20nr02da00");

        let padded = Ja4h::new("PUT", HttpVersion::Http3, &[], &[], Some("en"));
        assert_eq!(padded.prefix(), "pu30nn00en00");

        let no_language = Ja4h::new("OPTIONS", HttpVersion::Http10, &[], &[], None);
        assert_eq!(no_language.prefix(), "op10nn000000");
    }

    #[test]
    fn ja4h_header_hash_is_order_and_case_sensitive() {
        let first = Ja4h::new(
            "GET",
            HttpVersion::Http11,
            &["Host", "Accept", "User-Agent"],
            &[],
            None,
        );
        let reordered = Ja4h::new(
            "GET",
            HttpVersion::Http11,
            &["Host", "User-Agent", "Accept"],
            &[],
            None,
        );
        let lowercased = Ja4h::new(
            "GET",
            HttpVersion::Http11,
            &["host", "Accept", "User-Agent"],
            &[],
            None,
        );
        assert_ne!(first.header_names(), reordered.header_names());
        assert_ne!(first.header_names(), lowercased.header_names());
    }

    #[test]
    fn ja4h_caps_header_count_at_99() {
        let names: Vec<String> = (0..150).map(|index| format!("X-Header-{index}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let fp = Ja4h::new("GET", HttpVersion::Http11, &refs, &[], None);
        assert!(fp.prefix().contains("99"), "{}", fp.prefix());
    }
}
