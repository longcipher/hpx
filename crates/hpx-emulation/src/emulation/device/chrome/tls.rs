use super::*;
use crate::fingerprint::{
    CertCompression, CipherSuite as Cs, Curve, EchMode, SignatureAlgorithm as SigAlg,
    TlsFingerprint as FpTls, TlsPreset,
};

macro_rules! tls_options {
    (@build $builder:expr) => {
        $builder.build().into()
    };

    (1) => {
        tls_options!(@build ChromeTlsConfig::builder())
    };
    (2) => {
        tls_options!(@build ChromeTlsConfig::builder().enable_ech_grease(true))
    };
    (3) => {
        tls_options!(@build ChromeTlsConfig::builder().permute_extensions(true))
    };
    (4) => {
        tls_options!(@build ChromeTlsConfig::builder()
            .permute_extensions(true)
            .enable_ech_grease(true))
    };
    (5) => {
        tls_options!(@build ChromeTlsConfig::builder()
            .permute_extensions(true)
            .enable_ech_grease(true)
            .pre_shared_key(true))
    };
    (6, $curves:expr) => {
        tls_options!(@build ChromeTlsConfig::builder()
            .permute_extensions(true)
            .enable_ech_grease(true)
            .pre_shared_key(true)
            .curves($curves))
    };
    (7, $curves:expr) => {
        tls_options!(@build ChromeTlsConfig::builder()
            .permute_extensions(true)
            .enable_ech_grease(true)
            .pre_shared_key(true)
            .curves($curves)
            .alps_use_new_codepoint(true))
    };
}

/// Standard Chrome cipher suites.
const CHROME_CIPHER_SUITES: &[Cs] = &[
    Cs::Tls13Aes128GcmSha256,
    Cs::Tls13Aes256GcmSha384,
    Cs::Tls13ChaCha20Poly1305Sha256,
    Cs::EcdheEcdsaWithAes128GcmSha256,
    Cs::EcdheRsaWithAes128GcmSha256,
    Cs::EcdheEcdsaWithAes256GcmSha384,
    Cs::EcdheRsaWithAes256GcmSha384,
    Cs::EcdheEcdsaWithChaCha20Poly1305Sha256,
    Cs::EcdheRsaWithChaCha20Poly1305Sha256,
    Cs::EcdheRsaWithAes128CbcSha,
    Cs::EcdheRsaWithAes256CbcSha,
    Cs::RsaWithAes128GcmSha256,
    Cs::RsaWithAes256GcmSha384,
    Cs::RsaWithAes128CbcSha,
    Cs::RsaWithAes256CbcSha,
];

/// Standard Chrome signature algorithms.
const CHROME_SIGALGS: &[SigAlg] = &[
    SigAlg::EcdsaSecp256r1Sha256,
    SigAlg::RsaPssRsaeSha256,
    SigAlg::RsaPkcs1Sha256,
    SigAlg::EcdsaSecp384r1Sha384,
    SigAlg::RsaPssRsaeSha384,
    SigAlg::RsaPkcs1Sha384,
    SigAlg::RsaPssRsaeSha512,
    SigAlg::RsaPkcs1Sha512,
];

/// Builds a structured `FpTls` from a `TlsPreset`.
pub fn tls_fingerprint_from_preset(preset: TlsPreset) -> FpTls {
    match preset {
        TlsPreset::ChromeBase => FpTls {
            curves: vec![Curve::X25519, Curve::Secp256r1, Curve::Secp384r1],
            cipher_suites: CHROME_CIPHER_SUITES.to_vec(),
            signature_algorithms: CHROME_SIGALGS.to_vec(),
            permute_extensions: false,
            ech_mode: EchMode::Disabled,
            pre_shared_key: false,
            cert_compression: vec![CertCompression::Brotli],
            alps_use_new_codepoint: false,
        },
        TlsPreset::ChromeEchGrease => FpTls {
            ech_mode: EchMode::Grease,
            ..tls_fingerprint_from_preset(TlsPreset::ChromeBase)
        },
        TlsPreset::ChromePermute => FpTls {
            permute_extensions: true,
            ..tls_fingerprint_from_preset(TlsPreset::ChromeBase)
        },
        TlsPreset::ChromePermuteEch => FpTls {
            permute_extensions: true,
            ech_mode: EchMode::Grease,
            ..tls_fingerprint_from_preset(TlsPreset::ChromeBase)
        },
        TlsPreset::ChromePermuteEchPsk => FpTls {
            permute_extensions: true,
            ech_mode: EchMode::Grease,
            pre_shared_key: true,
            ..tls_fingerprint_from_preset(TlsPreset::ChromeBase)
        },
        TlsPreset::ChromeKyber => FpTls {
            curves: vec![
                Curve::X25519Kyber768Draft00,
                Curve::X25519,
                Curve::Secp256r1,
                Curve::Secp384r1,
            ],
            permute_extensions: true,
            ech_mode: EchMode::Grease,
            pre_shared_key: true,
            ..tls_fingerprint_from_preset(TlsPreset::ChromeBase)
        },
        TlsPreset::ChromeMlkem768 => FpTls {
            curves: vec![
                Curve::X25519MLKEM768,
                Curve::X25519,
                Curve::Secp256r1,
                Curve::Secp384r1,
            ],
            permute_extensions: true,
            ech_mode: EchMode::Grease,
            pre_shared_key: true,
            alps_use_new_codepoint: true,
            ..tls_fingerprint_from_preset(TlsPreset::ChromeBase)
        },
        TlsPreset::FirefoxBase => FpTls {
            curves: vec![Curve::X25519, Curve::Secp256r1, Curve::Secp384r1],
            cipher_suites: CHROME_CIPHER_SUITES.to_vec(),
            signature_algorithms: CHROME_SIGALGS.to_vec(),
            permute_extensions: false,
            ech_mode: EchMode::Disabled,
            pre_shared_key: false,
            cert_compression: vec![CertCompression::Brotli],
            alps_use_new_codepoint: false,
        },
        TlsPreset::FirefoxEchGrease => FpTls {
            ech_mode: EchMode::Grease,
            ..tls_fingerprint_from_preset(TlsPreset::FirefoxBase)
        },
        TlsPreset::SafariBase => FpTls {
            curves: vec![Curve::X25519, Curve::Secp256r1, Curve::Secp384r1],
            cipher_suites: CHROME_CIPHER_SUITES.to_vec(),
            signature_algorithms: CHROME_SIGALGS.to_vec(),
            permute_extensions: false,
            ech_mode: EchMode::Disabled,
            pre_shared_key: false,
            cert_compression: vec![CertCompression::Brotli],
            alps_use_new_codepoint: false,
        },
        TlsPreset::OkHttpBase => FpTls {
            curves: vec![Curve::X25519, Curve::Secp256r1, Curve::Secp384r1],
            cipher_suites: CHROME_CIPHER_SUITES.to_vec(),
            signature_algorithms: CHROME_SIGALGS.to_vec(),
            permute_extensions: false,
            ech_mode: EchMode::Disabled,
            pre_shared_key: false,
            cert_compression: vec![],
            alps_use_new_codepoint: false,
        },
    }
}

pub const CURVES_1: &str = join!(":", "X25519", "P-256", "P-384");
pub const CURVES_2: &str = join!(":", "X25519Kyber768Draft00", "X25519", "P-256", "P-384");
pub const CURVES_3: &str = join!(":", "X25519MLKEM768", "X25519", "P-256", "P-384");

pub const CIPHER_LIST: &str = join!(
    ":",
    "TLS_AES_128_GCM_SHA256",
    "TLS_AES_256_GCM_SHA384",
    "TLS_CHACHA20_POLY1305_SHA256",
    "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256",
    "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256",
    "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384",
    "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384",
    "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256",
    "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256",
    "TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA",
    "TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA",
    "TLS_RSA_WITH_AES_128_GCM_SHA256",
    "TLS_RSA_WITH_AES_256_GCM_SHA384",
    "TLS_RSA_WITH_AES_128_CBC_SHA",
    "TLS_RSA_WITH_AES_256_CBC_SHA"
);

pub const SIGALGS_LIST: &str = join!(
    ":",
    "ecdsa_secp256r1_sha256",
    "rsa_pss_rsae_sha256",
    "rsa_pkcs1_sha256",
    "ecdsa_secp384r1_sha384",
    "rsa_pss_rsae_sha384",
    "rsa_pkcs1_sha384",
    "rsa_pss_rsae_sha512",
    "rsa_pkcs1_sha512"
);

pub const CERT_COMPRESSION_ALGORITHM: &[CertificateCompressionAlgorithm] =
    &[CertificateCompressionAlgorithm::BROTLI];

#[derive(TypedBuilder)]
pub struct ChromeTlsConfig {
    #[builder(default = CURVES_1)]
    curves: &'static str,

    #[builder(default = SIGALGS_LIST)]
    sigalgs_list: &'static str,

    #[builder(default = CIPHER_LIST)]
    cipher_list: &'static str,

    #[builder(default = AlpsProtocol::HTTP2, setter(into))]
    alps_protos: AlpsProtocol,

    #[builder(default = false)]
    alps_use_new_codepoint: bool,

    #[builder(default = false, setter(into))]
    enable_ech_grease: bool,

    #[builder(default = false, setter(into))]
    permute_extensions: bool,

    #[builder(default = false, setter(into))]
    pre_shared_key: bool,
}

impl From<ChromeTlsConfig> for TlsOptions {
    fn from(val: ChromeTlsConfig) -> Self {
        TlsOptions::builder()
            .grease_enabled(true)
            .enable_ocsp_stapling(true)
            .enable_signed_cert_timestamps(true)
            .curves_list(val.curves)
            .sigalgs_list(val.sigalgs_list)
            .cipher_list(val.cipher_list)
            .min_tls_version(TlsVersion::TLS_1_2)
            .max_tls_version(TlsVersion::TLS_1_3)
            .permute_extensions(val.permute_extensions)
            .pre_shared_key(val.pre_shared_key)
            .enable_ech_grease(val.enable_ech_grease)
            .alps_protocols([val.alps_protos])
            .alps_use_new_codepoint(val.alps_use_new_codepoint)
            .aes_hw_override(true)
            .certificate_compression_algorithms(CERT_COMPRESSION_ALGORITHM)
            .build()
    }
}
