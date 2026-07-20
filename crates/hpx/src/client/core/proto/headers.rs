use bytes::BytesMut;
use http::{
    HeaderMap, Method,
    header::{
        CONNECTION, CONTENT_LENGTH, HeaderName, HeaderValue, TE, TRANSFER_ENCODING, UPGRADE,
        ValueIter,
    },
};

pub(super) fn connection_keep_alive(value: &HeaderValue) -> bool {
    connection_has(value, "keep-alive")
}

pub(super) fn connection_close(value: &HeaderValue) -> bool {
    connection_has(value, "close")
}

fn connection_has(value: &HeaderValue, needle: &str) -> bool {
    if let Ok(s) = value.to_str() {
        for val in s.split(',') {
            if val.trim().eq_ignore_ascii_case(needle) {
                return true;
            }
        }
    }
    false
}

// List of connection headers from RFC 9110 Section 7.6.1
//
// TE headers are allowed in HTTP/2 requests as long as the value is "trailers", so they're
// tested separately.
static CONNECTION_HEADERS: [HeaderName; 4] = [
    HeaderName::from_static("keep-alive"),
    HeaderName::from_static("proxy-connection"),
    TRANSFER_ENCODING,
    UPGRADE,
];

pub(crate) fn strip_connection_headers(headers: &mut HeaderMap, is_request: bool) {
    for header in &CONNECTION_HEADERS {
        if headers.remove(header).is_some() {
            warn!("Connection header illegal in HTTP/2: {}", header.as_str());
        }
    }

    if is_request {
        if headers
            .get(TE)
            .is_some_and(|te_header| te_header != "trailers")
        {
            warn!("TE headers not set to \"trailers\" are illegal in HTTP/2 requests");
            headers.remove(TE);
        }
    } else if headers.remove(TE).is_some() {
        warn!("TE headers illegal in HTTP/2 responses");
    }

    if let Some(header) = headers.remove(CONNECTION) {
        warn!(
            "Connection header illegal in HTTP/2: {}",
            CONNECTION.as_str()
        );
        let header_contents = header.to_str().unwrap();

        // A `Connection` header may have a comma-separated list of names of other headers that
        // are meant for only this specific connection.
        //
        // Iterate these names and remove them as headers. Connection-specific headers are
        // forbidden in HTTP2, as that information has been moved into frame types of the h2
        // protocol.
        for name in header_contents.split(',') {
            let name = name.trim();
            headers.remove(name);
        }
    }
}

pub(super) fn content_length_parse_all(headers: &HeaderMap) -> Option<u64> {
    content_length_parse_all_values(headers.get_all(CONTENT_LENGTH).into_iter())
}

pub(super) fn content_length_parse_all_values(values: ValueIter<'_, HeaderValue>) -> Option<u64> {
    // If multiple Content-Length headers were sent, everything can still
    // be alright if they all contain the same value, and all parse
    // correctly. If not, then it's an error.

    let mut content_length: Option<u64> = None;
    for h in values {
        if let Ok(line) = h.to_str() {
            for v in line.split(',') {
                let n = from_digits(v.trim().as_bytes())?;
                if content_length.is_none() {
                    content_length = Some(n)
                } else if content_length != Some(n) {
                    return None;
                }
            }
        } else {
            return None;
        }
    }

    content_length
}

fn from_digits(bytes: &[u8]) -> Option<u64> {
    // cannot use FromStr for u64, since it allows a signed prefix
    let mut result = 0u64;
    const RADIX: u64 = 10;

    if bytes.is_empty() {
        return None;
    }

    for &b in bytes {
        // can't use char::to_digit, since we haven't verified these bytes
        // are utf-8.
        match b {
            b'0'..=b'9' => {
                result = result.checked_mul(RADIX)?;
                result = result.checked_add((b - b'0') as u64)?;
            }
            _ => {
                // not a DIGIT, get outta here!
                return None;
            }
        }
    }

    Some(result)
}

pub(crate) fn method_has_defined_payload_semantics(method: &Method) -> bool {
    !matches!(
        *method,
        Method::GET | Method::HEAD | Method::DELETE | Method::CONNECT | Method::OPTIONS
    )
}

pub(crate) fn set_content_length_if_missing(headers: &mut HeaderMap, len: u64) {
    headers
        .entry(CONTENT_LENGTH)
        .or_insert_with(|| HeaderValue::from(len));
}

pub(super) fn transfer_encoding_is_chunked(headers: &HeaderMap) -> bool {
    is_chunked(headers.get_all(http::header::TRANSFER_ENCODING).into_iter())
}

/// Check if "chunked" appears more than once in Transfer-Encoding values.
///
/// RFC 9112 §3.3: A sender MUST NOT apply the chunked transfer coding more
/// than once to a message body.
pub(super) fn has_duplicate_chunked_transfer_encoding(headers: &HeaderMap) -> bool {
    let mut seen_chunked = false;
    for value in headers.get_all(http::header::TRANSFER_ENCODING) {
        if let Ok(s) = value.to_str() {
            for encoding in s.split(',') {
                if encoding.trim().eq_ignore_ascii_case("chunked") {
                    if seen_chunked {
                        return true;
                    }
                    seen_chunked = true;
                }
            }
        }
    }
    false
}

pub(super) fn is_chunked(mut encodings: ValueIter<'_, HeaderValue>) -> bool {
    // chunked must always be the last encoding, according to spec
    if let Some(line) = encodings.next_back() {
        return is_chunked_(line);
    }

    false
}

pub(super) fn is_chunked_(value: &HeaderValue) -> bool {
    // chunked must always be the last encoding, according to spec
    if let Ok(s) = value.to_str()
        && let Some(encoding) = s.rsplit(',').next()
    {
        return encoding.trim().eq_ignore_ascii_case("chunked");
    }

    false
}

pub(super) fn add_chunked(mut entry: http::header::OccupiedEntry<'_, HeaderValue>) {
    const CHUNKED: &str = "chunked";

    if let Some(line) = entry.iter_mut().next_back() {
        // + 2 for ", "
        let new_cap = line.as_bytes().len() + CHUNKED.len() + 2;
        let mut buf = BytesMut::with_capacity(new_cap);
        buf.extend_from_slice(line.as_bytes());
        buf.extend_from_slice(b", ");
        buf.extend_from_slice(CHUNKED.as_bytes());

        *line = HeaderValue::from_maybe_shared(buf.freeze())
            .expect("original header value plus ascii is valid");
        return;
    }

    entry.insert(HeaderValue::from_static(CHUNKED));
}
