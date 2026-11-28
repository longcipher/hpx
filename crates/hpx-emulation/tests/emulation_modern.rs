//! Regression coverage for the Chrome 150+ emulation profiles.
//!
//! Chrome 150 starts advertising post-quantum ML-DSA SignatureSchemes and
//! Chrome 152 adds signature-algorithm GREASE. Those codepoints cannot be
//! expressed through OpenSSL's named `sigalgs_list`, so the modern profiles
//! must carry a raw codepoint list. These tests pin that the profiles actually
//! differ on the wire, rather than silently falling back to the Chrome 132
//! TLS template that Chrome 144–149 all share.

#![cfg(feature = "emulation")]

use hpx::EmulationFactory as _;
use hpx_emulation::Emulation;

fn tls_options(kind: Emulation) -> hpx::Emulation {
    kind.emulation()
}

#[test]
fn modern_chrome_profiles_set_tls_options() {
    for kind in [
        Emulation::Chrome149,
        Emulation::Chrome150,
        Emulation::Chrome151,
        Emulation::Chrome152,
    ] {
        let mut emu = tls_options(kind);
        assert!(
            emu.tls_options_mut().is_some(),
            "every Chrome profile must configure TLS options"
        );
    }
}

#[test]
fn post_quantum_profiles_diverge_from_the_chrome_132_template() {
    let mut legacy = tls_options(Emulation::Chrome149);
    let mut pq = tls_options(Emulation::Chrome150);
    assert_ne!(
        legacy.tls_options_mut(),
        pq.tls_options_mut(),
        "Chrome 150 must not reuse the Chrome 132 TLS template"
    );

    let mut grease = tls_options(Emulation::Chrome152);
    assert_ne!(
        pq.tls_options_mut(),
        grease.tls_options_mut(),
        "Chrome 152 must add signature-algorithm GREASE"
    );
}

#[test]
fn chrome_150_and_151_share_the_same_sigalg_tier() {
    let mut a = tls_options(Emulation::Chrome150);
    let mut b = tls_options(Emulation::Chrome151);
    assert_eq!(
        a.tls_options_mut(),
        b.tls_options_mut(),
        "Chrome 150 and 151 share the ML-DSA-only SignatureScheme tier"
    );
}

#[test]
fn modern_enum_variants_are_distinct() {
    assert_ne!(Emulation::Chrome150, Emulation::Chrome151);
    assert_ne!(Emulation::Chrome151, Emulation::Chrome152);
    assert_ne!(Emulation::Chrome149, Emulation::Chrome152);
}
