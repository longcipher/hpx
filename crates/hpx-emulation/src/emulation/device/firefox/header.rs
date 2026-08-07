use super::*;

pub fn header_initializer(ua: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    header_firefox_ua!(headers, ua);
    header_firefox_accept!(headers);
    header_sec_fetch!(headers);
    headers
}

pub fn header_initializer_with_zstd(ua: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    header_firefox_ua!(headers, ua);
    header_firefox_accept!(zstd, headers);
    header_sec_fetch!(headers);
    headers.insert(
        HeaderName::from_static("priority"),
        HeaderValue::from_static("u=0, i"),
    );
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    const UA: &str =
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:151.0) Gecko/20100101 Firefox/151.0";

    #[test]
    fn all_firefox_header_initializers_populate_headers() {
        for headers in [header_initializer(UA), header_initializer_with_zstd(UA)] {
            assert!(
                headers.contains_key(http::header::USER_AGENT),
                "missing User-Agent"
            );
            assert!(headers.contains_key(http::header::ACCEPT), "missing Accept");
        }
    }
}
