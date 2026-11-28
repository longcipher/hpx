//! Chromium's `trust_anchors` ClientHello extension (codepoint `0xCA34`).
//!
//! `boring` 5.6.0 exposes `SSL_CTX_set1_requested_trust_anchors` in its
//! generated FFI but not in its safe wrapper, so this is the one place where
//! the workspace calls into BoringSSL directly.

use boring::ssl::SslConnectorBuilder;

use crate::Error;

/// Install the requested trust-anchor list on a client connection builder.
///
/// `Some(&[])` emits the empty extension that Chrome 152 sends by default;
/// `None` leaves the extension out entirely.
#[expect(
    unsafe_code,
    reason = "BoringSSL only exposes the trust-anchor setter as a C function"
)]
pub(super) fn set_requested_trust_anchors(
    builder: &mut SslConnectorBuilder,
    ids: Option<&[u8]>,
) -> Result<(), Error> {
    let Some(ids) = ids else {
        return Ok(());
    };

    // BoringSSL validates the length-prefixed ASN.1 relative-OID list itself;

    // `Span(nullptr, 0)` is valid in BoringSSL and produces the empty list.
    let ptr = if ids.is_empty() {
        std::ptr::null()
    } else {
        ids.as_ptr()
    };

    // SAFETY: `builder.as_ptr()` returns a live `*mut SSL_CTX` that remains
    // valid for the duration of this call. `ptr` is either null (with length
    // zero) or points to `ids.len()` readable bytes that outlive the call.
    // BoringSSL copies the list before returning, so no pointer escapes.
    let rc = unsafe {
        boring_sys::SSL_CTX_set1_requested_trust_anchors(builder.as_ptr(), ptr, ids.len())
    };

    if rc == 1 {
        Ok(())
    } else {
        Err(Error::tls(TrustAnchorError::Rejected))
    }
}

#[derive(Debug)]
enum TrustAnchorError {
    Rejected,
}

impl std::fmt::Display for TrustAnchorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected => f.write_str("BoringSSL rejected the trust anchor list"),
        }
    }
}

impl std::error::Error for TrustAnchorError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_oid_list() {
        let mut builder =
            boring::ssl::SslConnector::builder(boring::ssl::SslMethod::tls()).expect("builder");
        let err = set_requested_trust_anchors(&mut builder, Some(&[0xff, 0x01]))
            .expect_err("malformed ASN.1 relative OID must be rejected");
        assert!(err.to_string().contains("rejected"));
    }

    #[test]
    fn accepts_empty_and_well_formed_lists() {
        let mut builder =
            boring::ssl::SslConnector::builder(boring::ssl::SslMethod::tls()).expect("builder");
        set_requested_trust_anchors(&mut builder, Some(&[])).expect("empty list");
        // One-byte length prefix followed by a single ASN.1 subidentifier.
        set_requested_trust_anchors(&mut builder, Some(&[0x01, 0x2a])).expect("valid OID");
        set_requested_trust_anchors(&mut builder, None).expect("None is a no-op");
    }
}
