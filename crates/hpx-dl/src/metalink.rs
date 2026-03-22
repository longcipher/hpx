//! Metalink v4 (RFC 5854) parser.
//!
//! Parses Metalink XML documents to extract file URLs with priorities,
//! checksums, and file sizes. Produces [`MetalinkFile`] entries that can
//! be converted into [`DownloadRequest`]s.

use std::path::PathBuf;

use quick_xml::{Reader, events::Event};
use tracing::warn;

use crate::{
    error::DownloadError,
    types::{ChecksumSpec, DownloadRequest, HashAlgorithm},
};

/// A file entry from a Metalink document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetalinkFile {
    /// File name as declared in the `name` attribute.
    pub name: String,
    /// File size in bytes, if declared.
    pub size: Option<u64>,
    /// Download URLs with optional priority.
    pub urls: Vec<MetalinkUrl>,
    /// Content hashes for integrity verification.
    pub hashes: Vec<ChecksumSpec>,
}

/// A URL with optional priority from a Metalink document.
///
/// Lower numeric priority values mean higher priority in the Metalink spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetalinkUrl {
    /// The download URL.
    pub url: String,
    /// Priority value (lower = higher priority).
    pub priority: Option<u32>,
}

/// Parse a Metalink v4 (or v3) XML document into file entries.
///
/// Supports both `urn:ietf:params:xml:ns:metalink` (v4) and
/// `http://www.metalinker.org/` (v3) namespaces, as well as documents
/// without a namespace.
///
/// # Errors
///
/// Returns [`DownloadError::MetalinkParse`] if the XML is malformed or
/// required elements cannot be parsed.
pub fn parse_metalink(xml: &[u8]) -> Result<Vec<MetalinkFile>, DownloadError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);

    let mut files = Vec::new();
    let mut buf = Vec::new();

    // Parser state
    let mut in_file = false;
    let mut current_name = String::new();
    let mut current_size: Option<u64> = None;
    let mut current_urls: Vec<MetalinkUrl> = Vec::new();
    let mut current_hashes: Vec<ChecksumSpec> = Vec::new();
    let mut current_element: ElementKind = ElementKind::Other;
    let mut current_hash_type: Option<String> = None;
    let mut current_url_priority: Option<u32> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let local_name = resolve_local_name(e.name());
                match local_name.as_str() {
                    "file" => {
                        in_file = true;
                        current_name = extract_attr(e, b"name").unwrap_or_default();
                        current_size = None;
                        current_urls.clear();
                        current_hashes.clear();
                    }
                    "url" if in_file => {
                        current_element = ElementKind::Url;
                        current_url_priority =
                            extract_attr(e, b"priority").and_then(|v| v.parse().ok());
                    }
                    "hash" if in_file => {
                        current_element = ElementKind::Hash;
                        current_hash_type = extract_attr(e, b"type");
                    }
                    "size" if in_file => {
                        current_element = ElementKind::Size;
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local_name = resolve_local_name(e.name());
                if local_name.as_str() == "file" {
                    let name = extract_attr(e, b"name").unwrap_or_default();
                    // Self-closing <file/> — push immediately
                    files.push(MetalinkFile {
                        name,
                        size: None,
                        urls: Vec::new(),
                        hashes: Vec::new(),
                    });
                }
            }
            Ok(Event::End(ref e)) => {
                let local_name = resolve_local_name(e.name());
                match local_name.as_str() {
                    "file" if in_file => {
                        files.push(MetalinkFile {
                            name: std::mem::take(&mut current_name),
                            size: current_size,
                            urls: std::mem::take(&mut current_urls),
                            hashes: std::mem::take(&mut current_hashes),
                        });
                        in_file = false;
                    }
                    "url" | "hash" | "size" => {
                        current_element = ElementKind::Other;
                        current_hash_type = None;
                        current_url_priority = None;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.decode().map_err(|err| {
                    DownloadError::MetalinkParse(format!("text decode error: {err}"))
                })?;
                match current_element {
                    ElementKind::Url if in_file => {
                        current_urls.push(MetalinkUrl {
                            url: text.into_owned(),
                            priority: current_url_priority,
                        });
                    }
                    ElementKind::Hash if in_file => {
                        if let Some(ref hash_type) = current_hash_type {
                            if let Some(algorithm) = map_hash_algorithm(hash_type) {
                                current_hashes.push(ChecksumSpec {
                                    algorithm,
                                    expected: text.into_owned(),
                                });
                            } else {
                                warn!(hash_type = %hash_type, "unsupported hash algorithm in metalink");
                            }
                        }
                    }
                    ElementKind::Size if in_file => {
                        current_size = text.parse().ok();
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(DownloadError::MetalinkParse(format!(
                    "XML parse error at position {}: {e}",
                    reader.buffer_position()
                )));
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(files)
}

impl MetalinkFile {
    /// Convert this metalink file entry into download requests.
    ///
    /// The first URL becomes the primary download source; remaining URLs
    /// are added as mirror fallbacks. If a hash is present, the first one
    /// is used for checksum verification.
    ///
    /// # Errors
    ///
    /// This method does not currently return errors, but returns a `Result`
    /// for forward compatibility.
    #[must_use]
    pub fn into_download_requests(self, destination: impl Into<PathBuf>) -> Vec<DownloadRequest> {
        let mut requests = Vec::new();
        let dest = destination.into();

        if self.urls.is_empty() {
            return requests;
        }

        let mirrors: Vec<String> = self.urls[1..].iter().map(|u| u.url.clone()).collect();

        let request = DownloadRequest::builder(&self.urls[0].url, dest)
            .checksum_opt(self.hashes.first().cloned())
            .mirrors(mirrors)
            .build();

        requests.push(request);
        requests
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Element kinds we track during parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementKind {
    Url,
    Hash,
    Size,
    Other,
}

/// Strip an optional namespace prefix and return the local element name.
fn resolve_local_name(name: quick_xml::name::QName<'_>) -> String {
    let raw = std::str::from_utf8(name.into_inner()).unwrap_or("");
    if let Some(pos) = raw.rfind(':') {
        raw[pos + 1..].to_string()
    } else {
        raw.to_string()
    }
}

/// Extract an attribute value by its local name, ignoring namespace prefixes.
fn extract_attr(event: &quick_xml::events::BytesStart<'_>, attr_name: &[u8]) -> Option<String> {
    for attr in event.attributes().flatten() {
        let key = attr.key;
        let local = if let Some(pos) = key.into_inner().iter().rposition(|&b| b == b':') {
            &key.into_inner()[pos + 1..]
        } else {
            key.into_inner()
        };
        if local == attr_name {
            return std::str::from_utf8(&attr.value)
                .ok()
                .map(std::string::ToString::to_string);
        }
    }
    None
}

/// Map a Metalink hash type string to our `HashAlgorithm`.
fn map_hash_algorithm(name: &str) -> Option<HashAlgorithm> {
    match name.to_lowercase().as_str() {
        "md5" => Some(HashAlgorithm::Md5),
        "sha-1" | "sha1" => Some(HashAlgorithm::Sha1),
        "sha-256" | "sha256" => Some(HashAlgorithm::Sha256),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Extension trait for DownloadRequestBuilder to support optional checksum
// ---------------------------------------------------------------------------

/// Helper extension so we can pass `Option<ChecksumSpec>` cleanly.
trait BuilderExt {
    fn checksum_opt(self, checksum: Option<ChecksumSpec>) -> Self;
}

impl BuilderExt for crate::types::DownloadRequestBuilder {
    fn checksum_opt(mut self, checksum: Option<ChecksumSpec>) -> Self {
        if let Some(spec) = checksum {
            self = self.checksum(spec);
        }
        self
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Test 1: Parse valid Metalink v4 with single file --

    const METALINK_V4_SINGLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="example.iso">
    <size>14471447</size>
    <hash type="md5">d41d8cd98f00b204e9800998ecf8427e</hash>
    <hash type="sha-256">e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855</hash>
    <url priority="1">https://mirror1.example.com/example.iso</url>
    <url priority="2">https://mirror2.example.com/example.iso</url>
  </file>
</metalink>"#;

    #[test]
    fn parse_v4_single_file() {
        let files = parse_metalink(METALINK_V4_SINGLE.as_bytes()).expect("parse");

        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.name, "example.iso");
        assert_eq!(f.size, Some(14_471_447));
        assert_eq!(f.urls.len(), 2);
        assert_eq!(f.urls[0].url, "https://mirror1.example.com/example.iso");
        assert_eq!(f.urls[0].priority, Some(1));
        assert_eq!(f.urls[1].url, "https://mirror2.example.com/example.iso");
        assert_eq!(f.urls[1].priority, Some(2));
        assert_eq!(f.hashes.len(), 2);
        assert_eq!(f.hashes[0].algorithm, HashAlgorithm::Md5);
        assert_eq!(f.hashes[0].expected, "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(f.hashes[1].algorithm, HashAlgorithm::Sha256);
    }

    // -- Test 2: Parse valid Metalink v4 with multiple files --

    const METALINK_V4_MULTI: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="file-a.bin">
    <size>1024</size>
    <url priority="1">https://example.com/file-a.bin</url>
  </file>
  <file name="file-b.bin">
    <size>2048</size>
    <url priority="1">https://example.com/file-b.bin</url>
  </file>
</metalink>"#;

    #[test]
    fn parse_v4_multiple_files() {
        let files = parse_metalink(METALINK_V4_MULTI.as_bytes()).expect("parse");

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, "file-a.bin");
        assert_eq!(files[0].size, Some(1024));
        assert_eq!(files[1].name, "file-b.bin");
        assert_eq!(files[1].size, Some(2048));
    }

    // -- Test 3: Parse with checksums and priorities --

    #[test]
    fn parse_checksums_and_priorities() {
        let xml = r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="data.tar.gz">
    <hash type="sha-256">abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789</hash>
    <url priority="1">https://fast-mirror.example.com/data.tar.gz</url>
    <url priority="3">https://slow-mirror.example.com/data.tar.gz</url>
    <url priority="2">https://mid-mirror.example.com/data.tar.gz</url>
  </file>
</metalink>"#;

        let files = parse_metalink(xml.as_bytes()).expect("parse");
        assert_eq!(files.len(), 1);

        let f = &files[0];
        assert_eq!(f.urls.len(), 3);
        assert_eq!(f.urls[0].priority, Some(1));
        assert_eq!(f.urls[1].priority, Some(3));
        assert_eq!(f.urls[2].priority, Some(2));
        assert_eq!(f.hashes.len(), 1);
        assert_eq!(f.hashes[0].algorithm, HashAlgorithm::Sha256);
    }

    // -- Test 4: Parse empty Metalink (no files) --

    #[test]
    fn parse_empty_metalink() {
        let xml = r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
</metalink>"#;

        let files = parse_metalink(xml.as_bytes()).expect("parse");
        assert!(files.is_empty());
    }

    // -- Test 5: Parse malformed XML → error --

    #[test]
    fn parse_malformed_xml_returns_error() {
        let xml = b"not valid xml at all <<<";
        let result = parse_metalink(xml);
        assert!(result.is_err());
        match result.unwrap_err() {
            DownloadError::MetalinkParse(msg) => {
                assert!(msg.contains("XML parse error"), "unexpected message: {msg}");
            }
            other => panic!("expected MetalinkParse error, got: {other}"),
        }
    }

    // -- Test 6: into_download_requests produces correct requests --

    #[test]
    fn into_download_requests_basic() {
        let file = MetalinkFile {
            name: "example.iso".to_string(),
            size: Some(14_471_447),
            urls: vec![
                MetalinkUrl {
                    url: "https://mirror1.example.com/example.iso".to_string(),
                    priority: Some(1),
                },
                MetalinkUrl {
                    url: "https://mirror2.example.com/example.iso".to_string(),
                    priority: Some(2),
                },
            ],
            hashes: vec![ChecksumSpec {
                algorithm: HashAlgorithm::Sha256,
                expected: "abcd1234".to_string(),
            }],
        };

        let requests = file.into_download_requests("/tmp/example.iso");
        assert_eq!(requests.len(), 1);

        let req = &requests[0];
        assert_eq!(req.url, "https://mirror1.example.com/example.iso");
        assert_eq!(req.destination, PathBuf::from("/tmp/example.iso"));
        assert_eq!(req.mirrors, vec!["https://mirror2.example.com/example.iso"]);
        assert!(req.checksum.is_some());
        let checksum = req.checksum.as_ref().expect("checksum present");
        assert_eq!(checksum.algorithm, HashAlgorithm::Sha256);
        assert_eq!(checksum.expected, "abcd1234");
    }

    #[test]
    fn into_download_requests_no_urls() {
        let file = MetalinkFile {
            name: "empty.bin".to_string(),
            size: None,
            urls: vec![],
            hashes: vec![],
        };

        let requests = file.into_download_requests("/tmp/empty.bin");
        assert!(requests.is_empty());
    }

    #[test]
    fn into_download_requests_single_url_no_mirrors() {
        let file = MetalinkFile {
            name: "solo.bin".to_string(),
            size: Some(512),
            urls: vec![MetalinkUrl {
                url: "https://example.com/solo.bin".to_string(),
                priority: Some(1),
            }],
            hashes: vec![],
        };

        let requests = file.into_download_requests("/tmp/solo.bin");
        assert_eq!(requests.len(), 1);
        assert!(requests[0].mirrors.is_empty());
        assert!(requests[0].checksum.is_none());
    }

    // -- Test 7: Metalink v3 compatibility (different namespace) --

    #[test]
    fn parse_v3_namespace() {
        // v3 wraps files in <files> and urls in <resources>, but the element
        // local names we care about (file, url, hash, size) are the same.
        // Our parser ignores unknown containers, so it should still work if
        // the elements appear at the right nesting level.  For a simplified
        // v3 where <file> is directly found, this test exercises namespace
        // stripping.
        let xml = r#"<?xml version="1.0"?>
<metalink xmlns="http://www.metalinker.org/">
  <file name="legacy.bin">
    <size>9999</size>
    <url priority="1">https://legacy.example.com/legacy.bin</url>
    <url priority="10">https://fallback.example.com/legacy.bin</url>
  </file>
</metalink>"#;

        let files = parse_metalink(xml.as_bytes()).expect("parse");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "legacy.bin");
        assert_eq!(files[0].size, Some(9999));
        assert_eq!(files[0].urls.len(), 2);
        assert_eq!(files[0].urls[0].priority, Some(1));
        assert_eq!(files[0].urls[1].priority, Some(10));
    }

    // -- Additional edge-case tests --

    #[test]
    fn parse_no_namespace() {
        let xml = r#"<?xml version="1.0"?>
<metalink>
  <file name="plain.xml">
    <url>https://example.com/plain.xml</url>
  </file>
</metalink>"#;

        let files = parse_metalink(xml.as_bytes()).expect("parse");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "plain.xml");
        assert_eq!(files[0].urls.len(), 1);
    }

    #[test]
    fn parse_url_without_priority() {
        let xml = r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="test.bin">
    <url>https://example.com/test.bin</url>
  </file>
</metalink>"#;

        let files = parse_metalink(xml.as_bytes()).expect("parse");
        assert_eq!(files[0].urls[0].priority, None);
    }

    #[test]
    fn parse_unsupported_hash_ignored() {
        let xml = r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="test.bin">
    <hash type="sha-512">abcdef</hash>
    <hash type="sha-256">123456</hash>
  </file>
</metalink>"#;

        let files = parse_metalink(xml.as_bytes()).expect("parse");
        // sha-512 should be silently ignored
        assert_eq!(files[0].hashes.len(), 1);
        assert_eq!(files[0].hashes[0].algorithm, HashAlgorithm::Sha256);
    }

    #[test]
    fn parse_file_without_size() {
        let xml = r#"<?xml version="1.0"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="nosize.bin">
    <url>https://example.com/nosize.bin</url>
  </file>
</metalink>"#;

        let files = parse_metalink(xml.as_bytes()).expect("parse");
        assert_eq!(files[0].size, None);
    }

    #[test]
    fn hash_algorithm_mapping() {
        assert_eq!(map_hash_algorithm("md5"), Some(HashAlgorithm::Md5));
        assert_eq!(map_hash_algorithm("MD5"), Some(HashAlgorithm::Md5));
        assert_eq!(map_hash_algorithm("sha-1"), Some(HashAlgorithm::Sha1));
        assert_eq!(map_hash_algorithm("sha1"), Some(HashAlgorithm::Sha1));
        assert_eq!(map_hash_algorithm("sha-256"), Some(HashAlgorithm::Sha256));
        assert_eq!(map_hash_algorithm("sha256"), Some(HashAlgorithm::Sha256));
        assert_eq!(map_hash_algorithm("sha-512"), None);
        assert_eq!(map_hash_algorithm("unknown"), None);
    }

    #[test]
    fn parse_realistic_metalink() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="ubuntu-24.04-desktop-amd64.iso">
    <size>5924048896</size>
    <hash type="md5">a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4</hash>
    <hash type="sha-256">e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855</hash>
    <url priority="1">https://mirror.aarnet.edu.au/pub/ubuntu/releases/24.04/ubuntu-24.04-desktop-amd64.iso</url>
    <url priority="2">https://releases.ubuntu.com/24.04/ubuntu-24.04-desktop-amd64.iso</url>
    <url priority="10">https://mirror.rackspace.com/ubuntu/releases/24.04/ubuntu-24.04-desktop-amd64.iso</url>
  </file>
  <file name="SHA256SUMS">
    <size>324</size>
    <hash type="sha-256">fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210</hash>
    <url priority="1">https://releases.ubuntu.com/24.04/SHA256SUMS</url>
  </file>
</metalink>"#;

        let files = parse_metalink(xml.as_bytes()).expect("parse");
        assert_eq!(files.len(), 2);

        let iso = &files[0];
        assert_eq!(iso.name, "ubuntu-24.04-desktop-amd64.iso");
        assert_eq!(iso.size, Some(5_924_048_896));
        assert_eq!(iso.urls.len(), 3);
        assert_eq!(iso.hashes.len(), 2);

        let sums = &files[1];
        assert_eq!(sums.name, "SHA256SUMS");
        assert_eq!(sums.urls.len(), 1);
    }

    #[test]
    fn into_download_requests_from_realistic_metalink() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<metalink xmlns="urn:ietf:params:xml:ns:metalink">
  <file name="example.iso">
    <size>14471447</size>
    <hash type="sha-256">abc123</hash>
    <url priority="1">https://primary.example.com/example.iso</url>
    <url priority="2">https://secondary.example.com/example.iso</url>
    <url priority="3">https://tertiary.example.com/example.iso</url>
  </file>
</metalink>"#;

        let files = parse_metalink(xml.as_bytes()).expect("parse");
        let requests = files[0].clone().into_download_requests("/tmp/example.iso");

        assert_eq!(requests.len(), 1);
        let req = &requests[0];
        assert_eq!(req.url, "https://primary.example.com/example.iso");
        assert_eq!(req.mirrors.len(), 2);
        assert_eq!(req.mirrors[0], "https://secondary.example.com/example.iso");
        assert_eq!(req.mirrors[1], "https://tertiary.example.com/example.iso");
        assert!(req.checksum.is_some());
    }
}
