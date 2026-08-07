use super::*;

#[inline]
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
    // Opera is Chromium-based and sends sec-fetch-* headers like Chrome.
    header_sec_fetch!(headers);

    headers.insert(ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9"));
    #[cfg(feature = "emulation-compression")]
    headers.insert(
        ACCEPT_ENCODING,
        HeaderValue::from_static("gzip, deflate, br, zstd"),
    );
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
    headers.insert(
        HeaderName::from_static("priority"),
        HeaderValue::from_static("u=0, i"),
    );
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC_CH_UA: &str = "\"Chromium\";v=\"131\", \"Not.A/Brand\";v=\"99\"";
    const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 OPR/131.0.0.0";

    #[test]
    fn opera_header_initializer_populates_headers() {
        let headers = header_initializer_with_zstd_priority(SEC_CH_UA, UA, EmulationOS::Linux);
        assert!(
            headers.contains_key(http::header::USER_AGENT),
            "missing User-Agent"
        );
        assert!(headers.contains_key(http::header::ACCEPT), "missing Accept");
    }
}
