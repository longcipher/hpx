//! Chrome emulation header sets.
//!
//! # Fidelity notes
//!
//! Header *values* are byte-exact per profile, but [`HeaderMap`] does not
//! preserve insertion order, so HTTP/1.1 wire header ordering is unspecified
//! here. HTTP/2 pseudo-header and SETTINGS frame ordering are handled
//! separately through `SettingsOrder`/`PseudoOrder` in the device macros.
//! Fingerprint checks that depend on H1 header sequence are therefore not
//! covered by these profiles yet.

use super::*;

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn header_initializer(
    sec_ch_ua: &'static str,
    ua: &'static str,
    emulation_os: EmulationOS,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    header_chrome_sec_ch_ua!(
        headers,
        sec_ch_ua,
        emulation_os.sec_ch_ua_platform(),
        emulation_os.is_mobile()
    );
    header_chrome_ua!(headers, ua);
    header_sec_fetch!(headers);
    header_chrome_accept!(headers);
    headers
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn header_initializer_with_zstd(
    sec_ch_ua: &'static str,
    ua: &'static str,
    emulation_os: EmulationOS,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    header_chrome_sec_ch_ua!(
        headers,
        sec_ch_ua,
        emulation_os.sec_ch_ua_platform(),
        emulation_os.is_mobile()
    );
    header_chrome_ua!(headers, ua);
    header_sec_fetch!(headers);
    header_chrome_accept!(zstd, headers);
    headers
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn header_initializer_with_zstd_priority(
    sec_ch_ua: &'static str,
    ua: &'static str,
    emulation_os: EmulationOS,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    header_chrome_sec_ch_ua!(
        headers,
        sec_ch_ua,
        emulation_os.sec_ch_ua_platform(),
        emulation_os.is_mobile()
    );
    header_chrome_ua!(headers, ua);
    header_sec_fetch!(headers);
    header_chrome_accept!(zstd, headers);
    headers.insert(
        HeaderName::from_static("priority"),
        HeaderValue::from_static("u=0, i"),
    );
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC_CH_UA: &str = "\"Chromium\";v=\"147\", \"Not.A/Brand\";v=\"99\"";
    const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36";

    fn assert_full_headers(headers: HeaderMap) {
        assert!(
            headers.contains_key(http::header::USER_AGENT),
            "missing User-Agent"
        );
        assert!(headers.contains_key(http::header::ACCEPT), "missing Accept");
    }

    #[test]
    fn all_chrome_header_initializers_populate_headers() {
        assert_full_headers(header_initializer(SEC_CH_UA, UA, EmulationOS::Windows));
        assert_full_headers(header_initializer_with_zstd(
            SEC_CH_UA,
            UA,
            EmulationOS::Windows,
        ));
        assert_full_headers(header_initializer_with_zstd_priority(
            SEC_CH_UA,
            UA,
            EmulationOS::Windows,
        ));
    }
}
