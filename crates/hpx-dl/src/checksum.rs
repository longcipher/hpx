//! Checksum verification for downloaded content.

use std::path::Path;

use crate::{
    error::DownloadError,
    types::{ChecksumSpec, HashAlgorithm},
};

/// Compute the checksum of a file using the given algorithm.
///
/// Reads the file in 8 KiB chunks and feeds each chunk to the appropriate hasher.
pub async fn compute_checksum(
    file: &Path,
    algorithm: HashAlgorithm,
) -> Result<String, DownloadError> {
    use digest::Digest;
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(file).await?;
    let mut buf = [0u8; 8192];

    match algorithm {
        HashAlgorithm::Md5 => {
            let mut hasher = md5::Md5::new();
            loop {
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hex::encode(hasher.finalize()))
        }
        HashAlgorithm::Sha1 => {
            let mut hasher = sha1::Sha1::new();
            loop {
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hex::encode(hasher.finalize()))
        }
        HashAlgorithm::Sha256 => {
            let mut hasher = sha2::Sha256::new();
            loop {
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Ok(hex::encode(hasher.finalize()))
        }
    }
}

/// Verify the checksum of a file against the expected value in a [`ChecksumSpec`].
///
/// Returns `Ok(())` if the computed hash matches (case-insensitive),
/// or `Err(DownloadError::ChecksumMismatch)` otherwise.
pub async fn verify_checksum(spec: &ChecksumSpec, file: &Path) -> Result<(), DownloadError> {
    let computed = compute_checksum(file, spec.algorithm).await?;
    if computed.eq_ignore_ascii_case(&spec.expected) {
        tracing::debug!(
            algorithm = ?spec.algorithm,
            hash = %computed,
            "checksum verified"
        );
        Ok(())
    } else {
        tracing::warn!(
            algorithm = ?spec.algorithm,
            expected = %spec.expected,
            actual = %computed,
            "checksum mismatch"
        );
        Err(DownloadError::ChecksumMismatch {
            expected: spec.expected.clone(),
            actual: computed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: write content to a temp file and return its path.
    fn write_temp(content: &[u8]) -> tempfile::TempPath {
        let mut f = tempfile::NamedTempFile::new().expect("create temp file");
        std::io::Write::write_all(&mut f, content).expect("write temp file");
        f.into_temp_path()
    }

    // ── MD5 ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn compute_md5_empty_file() {
        let path = write_temp(b"");
        let hash = compute_checksum(&path, HashAlgorithm::Md5)
            .await
            .expect("md5");
        // MD5("") = d41d8cd98f00b204e9800998ecf8427e
        assert_eq!(hash, "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[tokio::test]
    async fn compute_md5_known_content() {
        let path = write_temp(b"hello world");
        let hash = compute_checksum(&path, HashAlgorithm::Md5)
            .await
            .expect("md5");
        // MD5("hello world") = 5eb63bbbe01eeed093cb22bb8f5acdc3
        assert_eq!(hash, "5eb63bbbe01eeed093cb22bb8f5acdc3");
    }

    #[tokio::test]
    async fn verify_md5_match() {
        let path = write_temp(b"hello world");
        let spec = ChecksumSpec {
            algorithm: HashAlgorithm::Md5,
            expected: "5eb63bbbe01eeed093cb22bb8f5acdc3".to_string(),
        };
        verify_checksum(&spec, &path).await.expect("should match");
    }

    #[tokio::test]
    async fn verify_md5_mismatch() {
        let path = write_temp(b"hello world");
        let spec = ChecksumSpec {
            algorithm: HashAlgorithm::Md5,
            expected: "00000000000000000000000000000000".to_string(),
        };
        let err = verify_checksum(&spec, &path).await.unwrap_err();
        assert!(matches!(err, DownloadError::ChecksumMismatch { .. }));
    }

    // ── SHA-1 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn compute_sha1_empty_file() {
        let path = write_temp(b"");
        let hash = compute_checksum(&path, HashAlgorithm::Sha1)
            .await
            .expect("sha1");
        // SHA-1("") = da39a3ee5e6b4b0d3255bfef95601890afd80709
        assert_eq!(hash, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[tokio::test]
    async fn compute_sha1_known_content() {
        let path = write_temp(b"hello world");
        let hash = compute_checksum(&path, HashAlgorithm::Sha1)
            .await
            .expect("sha1");
        // SHA-1("hello world") = 2aae6c35c94fcfb415dbe95f408b9ce91ee846ed
        assert_eq!(hash, "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed");
    }

    #[tokio::test]
    async fn verify_sha1_match() {
        let path = write_temp(b"hello world");
        let spec = ChecksumSpec {
            algorithm: HashAlgorithm::Sha1,
            expected: "2aae6c35c94fcfb415dbe95f408b9ce91ee846ed".to_string(),
        };
        verify_checksum(&spec, &path).await.expect("should match");
    }

    #[tokio::test]
    async fn verify_sha1_mismatch() {
        let path = write_temp(b"hello world");
        let spec = ChecksumSpec {
            algorithm: HashAlgorithm::Sha1,
            expected: "ffffffffffffffffffffffffffffffffffffffff".to_string(),
        };
        let err = verify_checksum(&spec, &path).await.unwrap_err();
        assert!(matches!(err, DownloadError::ChecksumMismatch { .. }));
    }

    // ── SHA-256 ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn compute_sha256_empty_file() {
        let path = write_temp(b"");
        let hash = compute_checksum(&path, HashAlgorithm::Sha256)
            .await
            .expect("sha256");
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[tokio::test]
    async fn compute_sha256_known_content() {
        let path = write_temp(b"hello world");
        let hash = compute_checksum(&path, HashAlgorithm::Sha256)
            .await
            .expect("sha256");
        // SHA-256("hello world") = b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[tokio::test]
    async fn verify_sha256_match() {
        let path = write_temp(b"hello world");
        let spec = ChecksumSpec {
            algorithm: HashAlgorithm::Sha256,
            expected: "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
                .to_string(),
        };
        verify_checksum(&spec, &path).await.expect("should match");
    }

    #[tokio::test]
    async fn verify_sha256_mismatch() {
        let path = write_temp(b"hello world");
        let spec = ChecksumSpec {
            algorithm: HashAlgorithm::Sha256,
            expected: "0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
        };
        let err = verify_checksum(&spec, &path).await.unwrap_err();
        assert!(matches!(err, DownloadError::ChecksumMismatch { .. }));
    }

    // ── Case-insensitive comparison ────────────────────────────────────

    #[tokio::test]
    async fn verify_case_insensitive() {
        let path = write_temp(b"hello world");
        let spec = ChecksumSpec {
            algorithm: HashAlgorithm::Sha256,
            expected: "B94D27B9934D3E08A52E52D7DA7DABFAC484EFE37A5380EE9088F7ACE2EFCDE9"
                .to_string(),
        };
        verify_checksum(&spec, &path)
            .await
            .expect("should match (case-insensitive)");
    }

    // ── Chunked reading ────────────────────────────────────────────────

    #[tokio::test]
    async fn compute_chunked_file() {
        // Create a file larger than the 8 KiB buffer to exercise chunked reading.
        let content = vec![0xAB_u8; 16_384]; // 16 KiB
        let path = write_temp(&content);

        // Compute expected with the same algorithm to ensure consistency.
        let hash = compute_checksum(&path, HashAlgorithm::Sha256)
            .await
            .expect("sha256");
        let spec = ChecksumSpec {
            algorithm: HashAlgorithm::Sha256,
            expected: hash.clone(),
        };
        verify_checksum(&spec, &path)
            .await
            .expect("roundtrip should match");
    }

    // ── Error on missing file ──────────────────────────────────────────

    #[tokio::test]
    async fn compute_missing_file_returns_io_error() {
        let err = compute_checksum(
            std::path::Path::new("/nonexistent/file"),
            HashAlgorithm::Sha256,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::Io(_)));
    }
}
