//! Fingerprint diff utility.
//!
//! Compare two `BrowserFingerprint` instances and report differences.
//! Useful for understanding what changed between browser versions.

use super::{BrowserFingerprint, CipherSuite, Curve, SignatureAlgorithm};

/// Represents a single difference between two fingerprints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FingerprintDiff {
    /// Browser name differs.
    NameChanged {
        old: &'static str,
        new: &'static str,
    },
    /// Browser version differs.
    VersionChanged {
        old: &'static str,
        new: &'static str,
    },
    /// TLS curves list differs.
    CurvesChanged { old: Vec<Curve>, new: Vec<Curve> },
    /// TLS cipher suites differ.
    CipherSuitesChanged {
        old: Vec<CipherSuite>,
        new: Vec<CipherSuite>,
    },
    /// TLS signature algorithms differ.
    SignatureAlgorithmsChanged {
        old: Vec<SignatureAlgorithm>,
        new: Vec<SignatureAlgorithm>,
    },
    /// Extension permutation setting changed.
    PermuteExtensionsChanged { old: bool, new: bool },
    /// ECH mode changed.
    EchModeChanged {
        old: super::EchMode,
        new: super::EchMode,
    },
    /// PSK setting changed.
    PreSharedKeyChanged { old: bool, new: bool },
    /// ALPS new codepoint setting changed.
    AlpsNewCodepointChanged { old: bool, new: bool },
    /// HTTP/2 initial window size changed.
    H2InitialWindowSizeChanged { old: u32, new: u32 },
    /// HTTP/2 max concurrent streams changed.
    H2MaxConcurrentStreamsChanged { old: Option<u32>, new: Option<u32> },
    /// HTTP/2 enable_push changed.
    H2EnablePushChanged {
        old: Option<bool>,
        new: Option<bool>,
    },
    /// HTTP headers differ.
    HeadersChanged,
}

impl std::fmt::Display for FingerprintDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FingerprintDiff::NameChanged { old, new } => {
                write!(f, "Name: {old} -> {new}")
            }
            FingerprintDiff::VersionChanged { old, new } => {
                write!(f, "Version: {old} -> {new}")
            }
            FingerprintDiff::CurvesChanged { old, new } => {
                write!(
                    f,
                    "Curves: {} -> {}",
                    format_curves(old),
                    format_curves(new)
                )
            }
            FingerprintDiff::CipherSuitesChanged { old, new } => {
                write!(
                    f,
                    "Cipher suites: {} suites -> {} suites",
                    old.len(),
                    new.len()
                )
            }
            FingerprintDiff::SignatureAlgorithmsChanged { old, new } => {
                write!(f, "Signature algorithms: {} -> {}", old.len(), new.len())
            }
            FingerprintDiff::PermuteExtensionsChanged { old, new } => {
                write!(f, "Permute extensions: {old} -> {new}")
            }
            FingerprintDiff::EchModeChanged { old: _, new: _ } => {
                write!(f, "ECH mode changed")
            }
            FingerprintDiff::PreSharedKeyChanged { old, new } => {
                write!(f, "PSK: {old} -> {new}")
            }
            FingerprintDiff::AlpsNewCodepointChanged { old, new } => {
                write!(f, "ALPS new codepoint: {old} -> {new}")
            }
            FingerprintDiff::H2InitialWindowSizeChanged { old, new } => {
                write!(f, "H2 initial window size: {old} -> {new}")
            }
            FingerprintDiff::H2MaxConcurrentStreamsChanged { old, new } => {
                write!(f, "H2 max concurrent streams: {old:?} -> {new:?}")
            }
            FingerprintDiff::H2EnablePushChanged { old, new } => {
                write!(f, "H2 enable push: {old:?} -> {new:?}")
            }
            FingerprintDiff::HeadersChanged => {
                write!(f, "HTTP headers differ")
            }
        }
    }
}

fn format_curves(curves: &[Curve]) -> String {
    curves
        .iter()
        .map(|c| c.openssl_name())
        .collect::<Vec<_>>()
        .join(":")
}

/// Compares two `BrowserFingerprint` instances and returns a list of differences.
///
/// Returns an empty vector if the fingerprints are identical.
pub fn diff_fingerprints(
    old: &BrowserFingerprint,
    new: &BrowserFingerprint,
) -> Vec<FingerprintDiff> {
    let mut diffs = Vec::new();

    if old.name != new.name {
        diffs.push(FingerprintDiff::NameChanged {
            old: old.name,
            new: new.name,
        });
    }

    if old.version != new.version {
        diffs.push(FingerprintDiff::VersionChanged {
            old: old.version,
            new: new.version,
        });
    }

    // TLS diffs
    if old.tls.curves != new.tls.curves {
        diffs.push(FingerprintDiff::CurvesChanged {
            old: old.tls.curves.clone(),
            new: new.tls.curves.clone(),
        });
    }

    if old.tls.cipher_suites != new.tls.cipher_suites {
        diffs.push(FingerprintDiff::CipherSuitesChanged {
            old: old.tls.cipher_suites.clone(),
            new: new.tls.cipher_suites.clone(),
        });
    }

    if old.tls.signature_algorithms != new.tls.signature_algorithms {
        diffs.push(FingerprintDiff::SignatureAlgorithmsChanged {
            old: old.tls.signature_algorithms.clone(),
            new: new.tls.signature_algorithms.clone(),
        });
    }

    if old.tls.permute_extensions != new.tls.permute_extensions {
        diffs.push(FingerprintDiff::PermuteExtensionsChanged {
            old: old.tls.permute_extensions,
            new: new.tls.permute_extensions,
        });
    }

    if old.tls.ech_mode != new.tls.ech_mode {
        diffs.push(FingerprintDiff::EchModeChanged {
            old: old.tls.ech_mode,
            new: new.tls.ech_mode,
        });
    }

    if old.tls.pre_shared_key != new.tls.pre_shared_key {
        diffs.push(FingerprintDiff::PreSharedKeyChanged {
            old: old.tls.pre_shared_key,
            new: new.tls.pre_shared_key,
        });
    }

    if old.tls.alps_use_new_codepoint != new.tls.alps_use_new_codepoint {
        diffs.push(FingerprintDiff::AlpsNewCodepointChanged {
            old: old.tls.alps_use_new_codepoint,
            new: new.tls.alps_use_new_codepoint,
        });
    }

    // HTTP/2 diffs
    if old.http2.initial_window_size != new.http2.initial_window_size {
        diffs.push(FingerprintDiff::H2InitialWindowSizeChanged {
            old: old.http2.initial_window_size,
            new: new.http2.initial_window_size,
        });
    }

    if old.http2.max_concurrent_streams != new.http2.max_concurrent_streams {
        diffs.push(FingerprintDiff::H2MaxConcurrentStreamsChanged {
            old: old.http2.max_concurrent_streams,
            new: new.http2.max_concurrent_streams,
        });
    }

    if old.http2.enable_push != new.http2.enable_push {
        diffs.push(FingerprintDiff::H2EnablePushChanged {
            old: old.http2.enable_push,
            new: new.http2.enable_push,
        });
    }

    // Headers diff
    if old.headers != new.headers {
        diffs.push(FingerprintDiff::HeadersChanged);
    }

    diffs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint::{EchMode, Http2Fingerprint, TlsFingerprint};

    fn make_test_fp(version: &'static str) -> BrowserFingerprint {
        BrowserFingerprint::new(
            "chrome",
            version,
            TlsFingerprint::default(),
            Http2Fingerprint::default(),
            vec![("user-agent", "test")],
        )
    }

    #[test]
    fn test_identical_fingerprints() {
        let a = make_test_fp("133");
        let b = make_test_fp("133");
        assert!(diff_fingerprints(&a, &b).is_empty());
    }

    #[test]
    fn test_version_diff() {
        let a = make_test_fp("133");
        let b = make_test_fp("134");
        let diffs = diff_fingerprints(&a, &b);
        assert_eq!(diffs.len(), 1);
        assert!(matches!(
            diffs[0],
            FingerprintDiff::VersionChanged {
                old: "133",
                new: "134"
            }
        ));
    }

    #[test]
    fn test_curves_diff() {
        let a = make_test_fp("133");
        let mut b = make_test_fp("134");
        b.tls.curves = vec![Curve::X25519MLKEM768, Curve::X25519];
        let diffs = diff_fingerprints(&a, &b);
        assert!(
            diffs
                .iter()
                .any(|d| matches!(d, FingerprintDiff::CurvesChanged { .. }))
        );
    }

    #[test]
    fn test_ech_diff() {
        let a = make_test_fp("133");
        let mut b = make_test_fp("133");
        b.tls.ech_mode = EchMode::Grease;
        let diffs = diff_fingerprints(&a, &b);
        assert!(
            diffs
                .iter()
                .any(|d| matches!(d, FingerprintDiff::EchModeChanged { .. }))
        );
    }
}
