#![allow(missing_docs)]
#![allow(
    clippy::allow_attributes,
    clippy::assert_is_empty,
    clippy::borrow_as_ptr,
    clippy::cast_lossless,
    clippy::collection_is_never_read,
    clippy::collapsible_if,
    clippy::duration_suboptimal_units,
    clippy::explicit_iter_loop,
    clippy::expect_fun_call,
    clippy::expect_used,
    clippy::ignore_without_reason,
    clippy::ignored_unit_patterns,
    clippy::items_after_test_module,
    clippy::iter_kv_map,
    clippy::iter_with_drain,
    clippy::let_underscore_future,
    clippy::manual_div_ceil,
    clippy::manual_string_new,
    clippy::map_unwrap_or,
    clippy::needless_borrows_for_generic_args,
    clippy::needless_continue,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_raw_string_hashes,
    clippy::nonminimal_bool,
    clippy::panic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::redundant_async_block,
    clippy::redundant_clone,
    clippy::redundant_pattern_matching,
    clippy::struct_field_names,
    clippy::type_complexity,
    clippy::unchecked_time_subtraction,
    clippy::unreadable_literal,
    clippy::unnecessary_cast,
    clippy::unnecessary_trailing_comma,
    clippy::unnested_or_patterns,
    clippy::useless_borrows_in_formatting,
    clippy::useless_vec,
    clippy::use_self,
    clippy::unwrap_used,
    unfulfilled_lint_expectations
)]
use std::time::Duration;

use hpx::{
    Client,
    tls::{CertStore, TlsInfo},
};

/// Certificate Store Example
///
/// In most cases, you don't need to manually configure certificate stores. hpx automatically
/// uses appropriate default certificates:
/// - With `webpki-roots` feature enabled: Uses Mozilla's maintained root certificate collection
/// - Without this feature: Uses system default certificate store paths
///
/// Manual certificate store configuration is only needed in the following special cases:
///
/// ## Scenarios requiring custom certificate store:
///
/// ### 1. Self-signed Certificates
/// - Connect to internal services using self-signed certificates
/// - Test servers in development environments
///
/// ### 2. Enterprise Internal CA
/// - Add root certificates from enterprise internal certificate authorities
/// - Access HTTPS services on corporate intranets
///
/// ### 3. Certificate Updates and Management
/// - Dynamically update certificates in the certificate store
/// - Remove revoked or expired certificates
///
/// ### 4. Compliance Requirements
/// - Special compliance requirements for certain industries or regions
/// - Need to use specific certificate collections
///
/// ### 5. Performance Optimization
/// - Reduce certificate store size to improve TLS handshake performance
/// - Include only necessary root certificates
#[tokio::main]
async fn main() -> hpx::Result<()> {
    // Create a client with a custom certificate store using webpki-roots
    let client = Client::builder()
        .cert_store(CertStore::from_der_certs(
            webpki_root_certs::TLS_SERVER_ROOT_CERTS,
        )?)
        .build()?;

    // Use the API you're already familiar with
    client.get("https://www.google.com").send().await?;

    // Self-signed certificate Client
    // Skip certificate verification for self-signed certificates
    let client = Client::builder()
        .tls_info(true)
        .cert_verification(false)
        .build()?;

    // Use the API you're already familiar with
    let resp = client.get("https://self-signed.badssl.com/").send().await?;
    if let Some(tls_info) = resp.extensions().get::<TlsInfo>()
        && let Some(peer_cert_der) = tls_info.peer_certificate()
    {
        // Create self-signed certificate Store
        let self_signed_store = CertStore::from_der_certs(&[peer_cert_der])?;

        // Create a client with self-signed certificate store
        let client = Client::builder()
            .cert_store(self_signed_store)
            .connect_timeout(Duration::from_secs(10))
            .build()?;

        // Use the API you're already familiar with
        let resp = client.get("https://self-signed.badssl.com/").send().await?;
        println!("{}", resp.text().await?);
    }

    Ok(())
}
