//! Proxy authentication.
//!
//! A proxy that requires authentication answers with `407 Proxy
//! Authentication Required` and one or more `Proxy-Authenticate` challenges.
//! This module parses those challenges and computes the matching
//! `Proxy-Authorization` responses for the schemes hpx supports:
//!
//! * `Basic` (RFC 7617) — sent preemptively when the credentials are known.
//! * `Digest` (RFC 7616) — answers the challenge with a computed response.
//! * `NTLM` / `Negotiate` — a multi-leg handshake that ends with an NTLMv2
//!   response.
//!
//! The `Basic` scheme needs no challenge, so the responder produces the
//! preemptive header for it directly.

use std::collections::HashMap;

use base64_simd::STANDARD as BASE64;
use hmac::{Hmac, KeyInit, Mac};
use md5::{Digest as _, Md5};
use sha2::Sha256;

/// The authentication scheme used with a proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProxyAuthMethod {
    /// HTTP Basic (RFC 7617).
    ///
    /// The credentials are sent preemptively in the `Proxy-Authorization`
    /// header; no challenge is needed.
    Basic,
    /// HTTP Digest (RFC 7616).
    ///
    /// The proxy sends a `Digest` challenge with a nonce; the client answers
    /// with a hash of the credentials, the nonce and the request URI.
    Digest,
    /// NTLM, carried in the `NTLM` or `Negotiate` HTTP scheme.
    ///
    /// The exchange takes up to three legs: a negotiate message, the
    /// server challenge, and the NTLMv2 response.
    Ntlm,
}

/// Credentials used to authenticate with a proxy.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ProxyCredentials {
    /// The scheme to use.
    pub method: ProxyAuthMethod,
    /// The user name.
    pub username: String,
    /// The password.
    pub password: String,
}

impl ProxyCredentials {
    /// Creates a new set of credentials.
    pub(crate) fn new(
        method: ProxyAuthMethod,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            method,
            username: username.into(),
            password: password.into(),
        }
    }
}

/// A parsed `Proxy-Authenticate` challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Challenge {
    /// The scheme token, upper-cased (for example `DIGEST`).
    pub scheme: String,
    /// The scheme parameters, keyed by their lower-cased names.
    ///
    /// For `Digest` this holds `realm`, `nonce`, `qop`, `algorithm` and
    /// `opaque`; for `Negotiate` it holds `token`, the base64 challenge
    /// message.
    pub params: HashMap<String, String>,
}

impl Challenge {
    /// Returns the value of a parameter, matched case-insensitively.
    pub(crate) fn param(&self, name: &str) -> Option<&str> {
        self.params
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Returns the raw challenge token, if the scheme carried one.
    pub(crate) fn token(&self) -> Option<&str> {
        self.param("token")
    }
}

/// Parses the `Proxy-Authenticate` header values into challenges.
///
/// A header may carry several challenges, and a `Digest` challenge carries
/// comma-separated parameters, so the two are told apart by the presence of
/// an `=` sign: a segment without one starts a new challenge.
pub(crate) fn parse_challenges(values: &[&http::HeaderValue]) -> Vec<Challenge> {
    let mut challenges = Vec::new();
    for value in values {
        if let Ok(text) = value.to_str() {
            challenges.extend(parse_challenge_list(text));
        }
    }
    challenges
}

/// Parses a single `Proxy-Authenticate` header value.
fn parse_challenge_list(text: &str) -> Vec<Challenge> {
    let mut challenges: Vec<Challenge> = Vec::new();

    for segment in split_outside_quotes(text, ',') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }

        let mut tokens = segment.split_whitespace();

        // The first token is either a new scheme or a parameter that
        // continues the challenge that is being built.
        let Some(first) = tokens.next() else {
            continue;
        };

        if let Some((name, value)) = split_parameter(first) {
            if let Some(current) = challenges.last_mut() {
                current.params.insert(name.to_ascii_lowercase(), value);
            }
            continue;
        }

        let scheme = first.to_ascii_uppercase();
        challenges.push(Challenge {
            scheme: scheme.clone(),
            params: HashMap::new(),
        });

        // `Negotiate` and `NTLM` carry the challenge message as a bare
        // token; the other schemes carry parameters.
        if matches!(scheme.as_str(), "NEGOTIATE" | "NTLM") {
            if let Some(token) = tokens.next()
                && !token.is_empty()
                && let Some(current) = challenges.last_mut()
            {
                current.params.insert("token".to_owned(), token.to_owned());
            }
            continue;
        }

        for token in tokens {
            if let Some((name, value)) = split_parameter(token)
                && let Some(current) = challenges.last_mut()
            {
                current.params.insert(name.to_ascii_lowercase(), value);
            }
        }
    }

    challenges
}

/// Splits a `name=value` segment, honouring quoted values.
fn split_parameter(segment: &str) -> Option<(String, String)> {
    let (name, value) = segment.split_once('=')?;
    let name = name.trim();
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }

    let value = value.trim();
    let value = if let Some(inner) = value.strip_prefix('"') {
        let end = inner.find('"')?;
        inner[..end].to_owned()
    } else {
        value.split(',').next().unwrap_or(value).trim().to_owned()
    };

    Some((name.to_owned(), value))
}

/// Splits `text` on `separator`, ignoring separators inside quoted strings.
fn split_outside_quotes(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;

    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            c if c == separator && !in_quotes => {
                parts.push(&text[start..index]);
                start = index + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// A stateful responder for proxy authentication challenges.
///
/// The responder keeps the state that the challenge schemes need across
/// legs: the Digest nonce counter and the NTLM handshake position.
#[derive(Debug)]
pub(crate) struct Authenticator {
    credentials: ProxyCredentials,
    /// The number of requests already answered with the current nonce.
    digest_nc: u32,
    /// The client nonce used for the current Digest challenge.
    digest_cnonce: String,
    /// Whether the NTLM negotiate message has already been sent.
    ntlm_negotiate_sent: bool,
}

impl Authenticator {
    /// Creates a responder for the given credentials.
    pub(crate) fn new(credentials: ProxyCredentials) -> Self {
        Self {
            credentials,
            digest_nc: 0,
            digest_cnonce: String::new(),
            ntlm_negotiate_sent: false,
        }
    }

    /// Sets the client nonce used for Digest responses.
    ///
    /// The nonce is normally random, but a fixed value makes the response
    /// reproducible for the RFC 7616 reference vectors.
    #[cfg(test)]
    pub(crate) fn set_client_nonce(&mut self, nonce: &str) {
        self.digest_cnonce = nonce.to_owned();
    }

    /// Builds the preemptive `Proxy-Authorization` header value.
    ///
    /// Only `Basic` can be sent without a challenge. For the other schemes
    /// this returns `None`.
    pub(crate) fn preemptive(&self) -> Option<http::HeaderValue> {
        match self.credentials.method {
            ProxyAuthMethod::Basic => Some(basic_value(
                &self.credentials.username,
                &self.credentials.password,
            )),
            ProxyAuthMethod::Digest | ProxyAuthMethod::Ntlm => None,
        }
    }

    /// Computes the `Proxy-Authorization` value that answers `challenges`.
    ///
    /// Returns `None` when no challenge matches the configured scheme.
    pub(crate) fn respond(
        &mut self,
        challenges: &[Challenge],
        method: &str,
        uri: &str,
    ) -> Option<http::HeaderValue> {
        match self.credentials.method {
            ProxyAuthMethod::Basic => Some(basic_value(
                &self.credentials.username,
                &self.credentials.password,
            )),
            ProxyAuthMethod::Digest => {
                let challenge = challenges.iter().find(|c| c.scheme == "DIGEST")?;
                self.respond_digest(challenge, method, uri)
            }
            ProxyAuthMethod::Ntlm => {
                let challenge = challenges
                    .iter()
                    .find(|c| c.scheme == "NTLM" || c.scheme == "NEGOTIATE")?;
                self.respond_ntlm(challenge)
            }
        }
    }

    /// Computes a Digest response (RFC 7616).
    fn respond_digest(
        &mut self,
        challenge: &Challenge,
        method: &str,
        uri: &str,
    ) -> Option<http::HeaderValue> {
        let realm = challenge.param("realm").unwrap_or_default();
        let nonce = challenge.param("nonce")?;
        let opaque = challenge.param("opaque");
        let algorithm = challenge
            .param("algorithm")
            .unwrap_or("MD5")
            .to_ascii_uppercase();
        let qop = challenge.param("qop").unwrap_or_default();

        // The `qop` parameter may list several options; `auth` is the only
        // one hpx implements, and it is the only one every server offers.
        let qop = if qop.is_empty() {
            String::new()
        } else {
            let selected = qop
                .split(',')
                .map(str::trim)
                .find(|q| q.eq_ignore_ascii_case("auth"))?;
            selected.to_ascii_lowercase()
        };

        let use_sha256 = algorithm.starts_with("SHA-256");
        let sess = algorithm.ends_with("-SESS");

        if self.digest_cnonce.is_empty() {
            self.digest_cnonce = random_hex(16);
        }
        self.digest_nc = self.digest_nc.wrapping_add(1);
        let nc = format!("{:08x}", self.digest_nc);
        let cnonce = self.digest_cnonce.clone();

        let ha1 = if use_sha256 {
            let mut hasher = Sha256::new();
            hasher.update(self.credentials.username.as_bytes());
            hasher.update(b":");
            hasher.update(realm.as_bytes());
            hasher.update(b":");
            hasher.update(self.credentials.password.as_bytes());
            let mut digest = hasher.finalize().to_vec();
            if sess {
                let mut hasher = Sha256::new();
                hasher.update(digest);
                hasher.update(nonce.as_bytes());
                hasher.update(b":");
                hasher.update(cnonce.as_bytes());
                digest = hasher.finalize().to_vec();
            }
            digest
        } else {
            let mut hasher = Md5::new();
            hasher.update(self.credentials.username.as_bytes());
            hasher.update(b":");
            hasher.update(realm.as_bytes());
            hasher.update(b":");
            hasher.update(self.credentials.password.as_bytes());
            let mut digest = hasher.finalize().to_vec();
            if sess {
                let mut hasher = Md5::new();
                hasher.update(digest);
                hasher.update(nonce.as_bytes());
                hasher.update(b":");
                hasher.update(cnonce.as_bytes());
                digest = hasher.finalize().to_vec();
            }
            digest
        };

        let ha2 = if use_sha256 {
            let mut hasher = Sha256::new();
            hasher.update(method.as_bytes());
            hasher.update(b":");
            hasher.update(uri.as_bytes());
            hasher.finalize().to_vec()
        } else {
            let mut hasher = Md5::new();
            hasher.update(method.as_bytes());
            hasher.update(b":");
            hasher.update(uri.as_bytes());
            hasher.finalize().to_vec()
        };

        let response = if qop.is_empty() {
            // RFC 2069 compatibility mode.
            let mut hasher = Md5::new();
            hasher.update(hex(&ha1));
            hasher.update(b":");
            hasher.update(nonce.as_bytes());
            hasher.update(b":");
            hasher.update(hex(&ha2));
            hex(&hasher.finalize())
        } else {
            let mut hasher = Md5::new();
            hasher.update(hex(&ha1));
            hasher.update(b":");
            hasher.update(nonce.as_bytes());
            hasher.update(b":");
            hasher.update(nc.as_bytes());
            hasher.update(b":");
            hasher.update(cnonce.as_bytes());
            hasher.update(b":");
            hasher.update(qop.as_bytes());
            hasher.update(b":");
            hasher.update(hex(&ha2));
            hex(&hasher.finalize())
        };

        let mut value = format!(
            "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{}\"",
            self.credentials.username, realm, nonce, uri, response
        );
        if !qop.is_empty() {
            value.push_str(&format!(", qop={qop}, nc={nc}, cnonce=\"{cnonce}\""));
        }
        if let Some(opaque) = opaque {
            value.push_str(&format!(", opaque=\"{opaque}\""));
        }
        value.push_str(&format!(", algorithm={algorithm}"));

        http::HeaderValue::from_str(&value).ok()
    }

    /// Computes the next NTLM message of the handshake.
    fn respond_ntlm(&mut self, challenge: &Challenge) -> Option<http::HeaderValue> {
        if !self.ntlm_negotiate_sent {
            self.ntlm_negotiate_sent = true;
            let message = ntlm_negotiate_message();
            return http::HeaderValue::from_str(&format!(
                "NTLM {}",
                BASE64.encode_to_string(&message)
            ))
            .ok();
        }

        let token = challenge.token()?;
        let message = BASE64.decode_to_vec(token.as_bytes()).ok()?;
        let response = ntlm_authenticate_message(
            &self.credentials.username,
            &self.credentials.password,
            &message,
        )?;

        http::HeaderValue::from_str(&format!("Negotiate {}", BASE64.encode_to_string(&response)))
            .ok()
    }
}

/// Builds a preemptive `Basic` `Proxy-Authorization` value.
pub(crate) fn basic_value(username: &str, password: &str) -> http::HeaderValue {
    let encoded = BASE64.encode_to_string(format!("{username}:{password}").as_bytes());
    let mut value = http::HeaderValue::from_str(&format!("Basic {encoded}"))
        .unwrap_or_else(|_| http::HeaderValue::from_static("Basic"));
    value.set_sensitive(true);
    value
}

/// Returns the lower-case hexadecimal representation of `bytes`.
fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    out
}

/// Returns a random lower-case hexadecimal string of `bytes` random bytes.
fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::fill(&mut buf);
    hex(&buf)
}

// ===== NTLM =====

/// The NTLM negotiate flags hpx offers.
const NTLM_FLAGS: u32 = 0x0000_0001 // NEGOTIATE_UNICODE
    | 0x0000_0004 // REQUEST_TARGET
    | 0x0000_0200 // NEGOTIATE_NTLM
    | 0x0000_8000 // NEGOTIATE_ALWAYS_SIGN
    | 0x0008_0000 // NEGOTIATE_EXTENDED_SESSIONSECURITY
    | 0x0200_0000 // NEGOTIATE_VERSION
    | 0x0080_0000 // NEGOTIATE_TARGET_INFO
    | 0x2000_0000 // NEGOTIATE_128
    | 0x4000_0000 // NEGOTIATE_KEY_EXCH
    | 0x8000_0000; // NEGOTIATE_56

/// Builds the NTLM type-1 (negotiate) message.
fn ntlm_negotiate_message() -> Vec<u8> {
    let mut message = Vec::with_capacity(40);

    // Signature: "NTLMSSP\0".
    message.extend_from_slice(b"NTLMSSP\0");
    // Message type: 1 (negotiate).
    message.extend_from_slice(&1u32.to_le_bytes());
    // Negotiate flags.
    message.extend_from_slice(&NTLM_FLAGS.to_le_bytes());
    // Domain name fields: length, allocated length, offset.
    message.extend_from_slice(&0u16.to_le_bytes());
    message.extend_from_slice(&0u16.to_le_bytes());
    message.extend_from_slice(&40u32.to_le_bytes());
    // Workstation fields.
    message.extend_from_slice(&0u16.to_le_bytes());
    message.extend_from_slice(&0u16.to_le_bytes());
    message.extend_from_slice(&40u32.to_le_bytes());
    // Version: major 10, minor 0, build 0, reserved 0, NTLM revision 15.
    message.extend_from_slice(&[10, 0, 0, 0, 0, 0, 0, 15]);

    message
}

/// The parsed parts of an NTLM type-2 (challenge) message.
struct NtlmChallenge {
    /// The 8-byte server challenge.
    server_challenge: [u8; 8],
    /// The target name, if the message carried one.
    target_name: Vec<u8>,
    /// The target info (AV pairs), if the message carried any.
    target_info: Vec<u8>,
}

/// Parses an NTLM type-2 message.
fn parse_ntlm_challenge(message: &[u8]) -> Option<NtlmChallenge> {
    if message.len() < 32 || &message[0..8] != b"NTLMSSP\0" {
        return None;
    }
    let message_type = u32::from_le_bytes(message[8..12].try_into().ok()?);
    if message_type != 2 {
        return None;
    }

    let target_name_fields = &message[12..20];
    let flags = u32::from_le_bytes(message[20..24].try_into().ok()?);
    let mut server_challenge = [0u8; 8];
    server_challenge.copy_from_slice(&message[24..32]);

    let read_field = |field: &[u8]| -> Option<Vec<u8>> {
        let len = u16::from_le_bytes(field[0..2].try_into().ok()?) as usize;
        let offset = u32::from_le_bytes(field[4..8].try_into().ok()?) as usize;
        if len == 0 {
            return Some(Vec::new());
        }
        let end = offset.checked_add(len)?;
        message.get(offset..end).map(<[u8]>::to_vec)
    };

    let target_name = read_field(target_name_fields).unwrap_or_default();

    // The target info is described by the field descriptor at offset 40.
    let target_info = message.get(40..48).and_then(read_field).unwrap_or_default();

    // The target info is only meaningful when the server announced it.
    let target_info = if flags & NEGOTIATE_TARGET_INFO != 0 {
        target_info
    } else {
        Vec::new()
    };

    Some(NtlmChallenge {
        server_challenge,
        target_name,
        target_info,
    })
}

/// Builds the NTLM type-3 (authenticate) message for an NTLMv2 response.
fn ntlm_authenticate_message(
    username: &str,
    password: &str,
    challenge_message: &[u8],
) -> Option<Vec<u8>> {
    let challenge = parse_ntlm_challenge(challenge_message)?;

    // The domain is the target name from the challenge, or the part of the
    // user name before the backslash.
    let domain = if challenge.target_name.is_empty() {
        username
            .split_once('\\')
            .map_or_else(String::new, |(domain, _)| domain.to_owned())
    } else {
        String::from_utf8_lossy(&challenge.target_name).into_owned()
    };
    let user = username
        .split_once('\\')
        .map_or_else(|| username.to_owned(), |(_, user)| user.to_owned());

    let ntlm_hash = md4_utf16(password.as_bytes());

    // The client challenge is eight random bytes.
    let mut client_challenge = [0u8; 8];
    rand::fill(&mut client_challenge);

    // The blob: version, timestamp, client challenge and the target info.
    // The blob (MS-NLMP 2.2.2.7):
    //
    // ```text
    //  0  RespType         1
    //  1  HiRespType       1
    //  2  Reserved1        2
    //  4  Reserved2        4
    //  8  Timestamp        8
    // 16  ClientChallenge  8
    // 24  Reserved3        4
    // 28  Reserved4        4
    // 32  AvPairs          variable
    //     ... terminator  4
    // ```
    let mut blob = Vec::new();
    blob.extend_from_slice(&[0x01, 0x01, 0x00, 0x00]);
    blob.extend_from_slice(&[0u8; 4]);
    blob.extend_from_slice(&filetime());
    blob.extend_from_slice(&client_challenge);
    blob.extend_from_slice(&[0u8; 4]);
    blob.extend_from_slice(&[0u8; 4]);
    if !challenge.target_info.is_empty() {
        blob.extend_from_slice(&challenge.target_info);
    }
    blob.extend_from_slice(&[0u8; 4]);

    // NTProofStr = HMAC-MD5(NTLMHash, ServerChallenge || blob).
    let mut mac = <Hmac<Md5> as KeyInit>::new_from_slice(&ntlm_hash).ok()?;
    mac.update(&challenge.server_challenge);
    mac.update(&blob);
    let proof = mac.finalize().into_bytes().to_vec();

    // The NTLM response is the proof followed by the blob.
    let mut ntlm_response = proof;
    ntlm_response.extend_from_slice(&blob);

    // The LMv2 response is unused by NTLMv2 but the field must be present.
    let mut lm_response = vec![0u8; 24];
    let mut mac = <Hmac<Md5> as KeyInit>::new_from_slice(&ntlm_hash).ok()?;
    mac.update(&challenge.server_challenge);
    mac.update(&client_challenge);
    lm_response[..16].copy_from_slice(&mac.finalize().into_bytes());

    let domain_bytes = utf16le(domain.as_bytes());
    let user_bytes = utf16le(user.as_bytes());

    // The layout of the type-3 message (MS-NLMP 2.2.2.3):
    //
    // ```text
    //   0  signature        8
    //   8  message type     4
    //  12  LM response     24
    //  36  LM fields        8
    //  44  NTLM fields      8
    //  52  NTLM response    n
    //  52+n domain fields   8
    //  60+n user fields     8
    //  68+n host fields     8
    //  76+n session fields  8
    //  84+n domain and user name
    // ```
    //
    // Each field descriptor is `MaxLen(2) Len(2) Offset(4)`, and the
    // offset points at the data it describes.
    const SIGNATURE_LEN: u32 = 8;
    const TYPE_LEN: u32 = 4;
    const FIELD_LEN: u32 = 8;

    let lm_offset = SIGNATURE_LEN + TYPE_LEN;
    let ntlm_fields_offset = lm_offset + LM_RESPONSE_LEN + FIELD_LEN;
    let ntlm_offset = ntlm_fields_offset + FIELD_LEN;
    // The domain and the user name follow the four field descriptors that
    // follow the NTLM response.
    let payload_offset = ntlm_offset + ntlm_response.len() as u32 + 4 * FIELD_LEN;

    let mut message = Vec::new();
    message.extend_from_slice(b"NTLMSSP\0");
    message.extend_from_slice(&3u32.to_le_bytes());

    // LM response.
    message.extend_from_slice(&lm_response);
    message.extend_from_slice(&(LM_RESPONSE_LEN as u16).to_le_bytes());
    message.extend_from_slice(&(LM_RESPONSE_LEN as u16).to_le_bytes());
    message.extend_from_slice(&lm_offset.to_le_bytes());

    // NTLM response fields, followed by the response data.
    message.extend_from_slice(&(ntlm_response.len() as u16).to_le_bytes());
    message.extend_from_slice(&(ntlm_response.len() as u16).to_le_bytes());
    message.extend_from_slice(&ntlm_offset.to_le_bytes());
    message.extend_from_slice(&ntlm_response);

    // Domain, user and workstation fields. The domain and the user name are
    // written after the fixed part, and the fields describe where.
    let domain_offset = payload_offset;
    let user_offset = domain_offset + domain_bytes.len() as u32;
    let workstation_offset = user_offset + user_bytes.len() as u32;

    message.extend_from_slice(&(domain_bytes.len() as u16).to_le_bytes());
    message.extend_from_slice(&(domain_bytes.len() as u16).to_le_bytes());
    message.extend_from_slice(&domain_offset.to_le_bytes());

    message.extend_from_slice(&(user_bytes.len() as u16).to_le_bytes());
    message.extend_from_slice(&(user_bytes.len() as u16).to_le_bytes());
    message.extend_from_slice(&user_offset.to_le_bytes());

    // The workstation name is empty.
    message.extend_from_slice(&0u16.to_le_bytes());
    message.extend_from_slice(&0u16.to_le_bytes());
    message.extend_from_slice(&workstation_offset.to_le_bytes());

    // The session key is empty for NTLMv2 without key exchange.
    message.extend_from_slice(&0u16.to_le_bytes());
    message.extend_from_slice(&0u16.to_le_bytes());
    message.extend_from_slice(&workstation_offset.to_le_bytes());

    message.extend_from_slice(&domain_bytes);
    message.extend_from_slice(&user_bytes);

    Some(message)
}

/// The length of the LM response field in the NTLM type-3 message.
const LM_RESPONSE_LEN: u32 = 24;

/// The `NEGOTIATE_TARGET_INFO` negotiate flag (MS-NLMP 2.2.2.5).
const NEGOTIATE_TARGET_INFO: u32 = 0x0080_0000;

/// The NTLM hash: MD4 over the UTF-16LE encoded password.
fn md4_utf16(password: &[u8]) -> Vec<u8> {
    let mut hasher = md4::Md4::new();
    hasher.update(utf16le(password));
    hasher.finalize().to_vec()
}

/// Encodes `bytes` as UTF-16LE.
fn utf16le(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    text.encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<u8>>()
}

/// The current time as a Windows FILETIME (100 ns ticks since 1601).
fn filetime() -> [u8; 8] {
    const EPOCH_DIFF: u128 = 116_444_736_000_000_000;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() / 100);
    let ticks = now.saturating_add(EPOCH_DIFF) as u64;
    ticks.to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(value: &str) -> http::HeaderValue {
        http::HeaderValue::from_str(value).unwrap()
    }

    #[test]
    fn parses_a_single_basic_challenge() {
        let challenges = parse_challenges(&[&header("Basic realm=\"proxy\"")]);
        assert_eq!(challenges.len(), 1);
        assert_eq!(challenges[0].scheme, "BASIC");
        assert_eq!(challenges[0].param("realm"), Some("proxy"));
    }

    #[test]
    fn parses_a_digest_challenge_with_parameters() {
        let value = header(
            "Digest realm=\"proxy\", nonce=\"abc123\", qop=\"auth\", algorithm=MD5, opaque=\"xyz\"",
        );
        let challenges = parse_challenges(&[&value]);
        assert_eq!(challenges.len(), 1);
        assert_eq!(challenges[0].scheme, "DIGEST");
        assert_eq!(challenges[0].param("realm"), Some("proxy"));
        assert_eq!(challenges[0].param("nonce"), Some("abc123"));
        assert_eq!(challenges[0].param("qop"), Some("auth"));
        assert_eq!(challenges[0].param("algorithm"), Some("MD5"));
        assert_eq!(challenges[0].param("opaque"), Some("xyz"));
    }

    #[test]
    fn parses_several_challenges_in_one_header() {
        let value = header("Basic realm=\"a\", Digest realm=\"b\", nonce=\"n\"");
        let challenges = parse_challenges(&[&value]);
        assert_eq!(challenges.len(), 2);
        assert_eq!(challenges[0].scheme, "BASIC");
        assert_eq!(challenges[0].param("realm"), Some("a"));
        assert_eq!(challenges[1].scheme, "DIGEST");
        assert_eq!(challenges[1].param("nonce"), Some("n"));
    }

    #[test]
    fn parses_several_challenge_headers() {
        let challenges = parse_challenges(&[&header("Basic realm=\"a\""), &header("NTLM")]);
        assert_eq!(challenges.len(), 2);
        assert_eq!(challenges[0].scheme, "BASIC");
        assert_eq!(challenges[1].scheme, "NTLM");
    }

    #[test]
    fn parses_a_negotiate_challenge_with_a_token() {
        let value = header("Negotiate TlRMTVNTUAABAAAAB4IIAAAAAAAAAAAAAAAAAAAAAAA=");
        let challenges = parse_challenges(&[&value]);
        assert_eq!(challenges.len(), 1);
        assert_eq!(challenges[0].scheme, "NEGOTIATE");
        assert_eq!(
            challenges[0].token(),
            Some("TlRMTVNTUAABAAAAB4IIAAAAAAAAAAAAAAAAAAAAAAA=")
        );
    }

    #[test]
    fn basic_credentials_are_sent_preemptively() {
        let auth = Authenticator::new(ProxyCredentials::new(
            ProxyAuthMethod::Basic,
            "Aladdin",
            "open sesame",
        ));
        let value = auth.preemptive().unwrap();
        assert_eq!(value, "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
    }

    #[test]
    fn digest_credentials_have_no_preemptive_value() {
        let auth = Authenticator::new(ProxyCredentials::new(
            ProxyAuthMethod::Digest,
            "Aladdin",
            "open sesame",
        ));
        assert!(auth.preemptive().is_none());
    }

    /// RFC 7616 §3.5 example: the response for the well-known nonce.
    #[test]
    fn digest_response_matches_the_reference_computation() {
        let mut auth = Authenticator::new(ProxyCredentials::new(
            ProxyAuthMethod::Digest,
            "Mufasa",
            "Circle Of Life",
        ));
        auth.set_client_nonce("0a4f113b");

        let value = header(
            "Digest realm=\"testrealm@host.com\", qop=\"auth,auth-int\", nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", opaque=\"5ccc069c403ebaf9f0171e9517f40e41\"",
        );
        let challenges = parse_challenges(&[&value]);
        let response = auth.respond(&challenges, "GET", "/dir/index.html").unwrap();
        let text = response.to_str().unwrap();

        assert!(text.starts_with("Digest "), "{text}");
        assert!(text.contains("username=\"Mufasa\""), "{text}");
        assert!(text.contains("realm=\"testrealm@host.com\""), "{text}");
        assert!(text.contains("qop=auth"), "{text}");
        assert!(text.contains("nc=00000001"), "{text}");
        assert!(text.contains("cnonce=\"0a4f113b\""), "{text}");
        assert!(
            text.contains("response=\"6629fae49393a05397450978507c4ef1\""),
            "{text}"
        );
    }

    /// The response must change when the nonce changes.
    #[test]
    fn digest_response_depends_on_the_nonce() {
        let mut auth = Authenticator::new(ProxyCredentials::new(
            ProxyAuthMethod::Digest,
            "Mufasa",
            "Circle Of Life",
        ));

        let first = header("Digest realm=\"r\", qop=\"auth\", nonce=\"aaaa\"");
        let second = header("Digest realm=\"r\", qop=\"auth\", nonce=\"bbbb\"");

        let one = auth
            .respond(&parse_challenges(&[&first]), "GET", "/")
            .unwrap();
        let two = auth
            .respond(&parse_challenges(&[&second]), "GET", "/")
            .unwrap();

        assert_ne!(one, two);
    }

    /// The nonce counter increases for every request with the same nonce.
    #[test]
    fn digest_nonce_counter_increases() {
        let mut auth = Authenticator::new(ProxyCredentials::new(
            ProxyAuthMethod::Digest,
            "user",
            "pass",
        ));
        let challenge = header("Digest realm=\"r\", qop=\"auth\", nonce=\"aaaa\"");

        let first = auth
            .respond(&parse_challenges(&[&challenge]), "GET", "/")
            .unwrap();
        let second = auth
            .respond(&parse_challenges(&[&challenge]), "GET", "/")
            .unwrap();

        assert!(first.to_str().unwrap().contains("nc=00000001"));
        assert!(second.to_str().unwrap().contains("nc=00000002"));
    }

    /// SHA-256 produces a different (and longer) response than MD5.
    #[test]
    fn digest_supports_sha256() {
        let mut auth = Authenticator::new(ProxyCredentials::new(
            ProxyAuthMethod::Digest,
            "user",
            "pass",
        ));
        let challenge =
            header("Digest realm=\"r\", qop=\"auth\", nonce=\"aaaa\", algorithm=SHA-256");

        let response = auth
            .respond(&parse_challenges(&[&challenge]), "GET", "/")
            .unwrap();
        let text = response.to_str().unwrap();

        assert!(text.contains("algorithm=SHA-256"), "{text}");
        assert!(text.contains(&"response=\"".to_owned()), "{text}");
        // A SHA-256 response is 64 hex characters long.
        let start = text.find("response=\"").unwrap() + "response=\"".len();
        assert_eq!(text[start..start + 64].len(), 64, "{text}");
    }

    /// The NTLM handshake starts with a negotiate message.
    #[test]
    fn ntlm_starts_with_a_negotiate_message() {
        let mut auth =
            Authenticator::new(ProxyCredentials::new(ProxyAuthMethod::Ntlm, "user", "pass"));

        let challenge = header("NTLM");
        let response = auth
            .respond(&parse_challenges(&[&challenge]), "CONNECT", "host:443")
            .unwrap();
        let text = response.to_str().unwrap();

        assert!(text.starts_with("NTLM "), "{text}");
        let message = BASE64.decode_to_vec(&text.as_bytes()[5..]).unwrap();
        assert_eq!(&message[0..8], b"NTLMSSP\0");
        assert_eq!(u32::from_le_bytes(message[8..12].try_into().unwrap()), 1);
    }

    /// The NTLM handshake answers the type-2 challenge with a type-3
    /// message.
    #[test]
    fn ntlm_answers_the_challenge_with_an_authenticate_message() {
        let mut auth =
            Authenticator::new(ProxyCredentials::new(ProxyAuthMethod::Ntlm, "user", "pass"));

        // Leg one: the negotiate message.
        let challenge = header("NTLM");
        auth.respond(&parse_challenges(&[&challenge]), "CONNECT", "host:443")
            .unwrap();

        // Leg two: the server challenge.
        let mut type2 = Vec::new();
        type2.extend_from_slice(b"NTLMSSP\0");
        type2.extend_from_slice(&2u32.to_le_bytes());
        type2.extend_from_slice(&0u16.to_le_bytes());
        type2.extend_from_slice(&0u16.to_le_bytes());
        type2.extend_from_slice(&48u32.to_le_bytes());
        type2.extend_from_slice(&NTLM_FLAGS.to_le_bytes());
        type2.extend_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        type2.extend_from_slice(&[0u8; 8]);
        type2.extend_from_slice(&0u16.to_le_bytes());
        type2.extend_from_slice(&0u16.to_le_bytes());
        type2.extend_from_slice(&48u32.to_le_bytes());

        let token = BASE64.encode_to_string(&type2);
        let challenge = header(&format!("Negotiate {token}"));
        let response = auth
            .respond(&parse_challenges(&[&challenge]), "CONNECT", "host:443")
            .unwrap();
        let text = response.to_str().unwrap();

        assert!(text.starts_with("Negotiate "), "{text}");
        let message = BASE64.decode_to_vec(&text.as_bytes()[10..]).unwrap();
        assert_eq!(&message[0..8], b"NTLMSSP\0");
        assert_eq!(u32::from_le_bytes(message[8..12].try_into().unwrap()), 3);
    }

    /// The NTLM hash is MD4 over the UTF-16LE password.
    #[test]
    fn ntlm_hash_matches_the_reference_value() {
        // MS-NLMP 4.2.4: the NTLM hash of "Password".
        assert_eq!(
            hex(&md4_utf16(b"Password")),
            "a4f49c406510bdcab6824ee7c30fd852"
        );
    }

    /// Builds an NTLM type-2 message with the given server challenge.
    fn ntlm_challenge_message(server_challenge: [u8; 8], target_name: &[u8]) -> Vec<u8> {
        ntlm_challenge_message_with_info(server_challenge, target_name, &[])
    }

    /// Builds an NTLM type-2 message with a target name and target info.
    fn ntlm_challenge_message_with_info(
        server_challenge: [u8; 8],
        target_name: &[u8],
        target_info: &[u8],
    ) -> Vec<u8> {
        let mut message = Vec::new();
        message.extend_from_slice(b"NTLMSSP\0");
        message.extend_from_slice(&2u32.to_le_bytes());
        // Target name fields: length, allocated length, offset.
        message.extend_from_slice(&(target_name.len() as u16).to_le_bytes());
        message.extend_from_slice(&(target_name.len() as u16).to_le_bytes());
        message.extend_from_slice(&48u32.to_le_bytes());
        // Negotiate flags: target info is present.
        message.extend_from_slice(&NEGOTIATE_TARGET_INFO.to_le_bytes());
        message.extend_from_slice(&server_challenge);
        message.extend_from_slice(&[0u8; 8]);
        // Target info fields: length, allocated length, offset. The target
        // info follows the target name.
        let target_info_offset = 48 + target_name.len() as u32;
        message.extend_from_slice(&(target_info.len() as u16).to_le_bytes());
        message.extend_from_slice(&(target_info.len() as u16).to_le_bytes());
        message.extend_from_slice(&target_info_offset.to_le_bytes());
        message.extend_from_slice(target_name);
        message.extend_from_slice(target_info);
        message
    }

    /// Builds an AV pair (MS-NLMP 2.2.2.7.1).
    fn av_pair(id: u16, value: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(value);
        out
    }

    /// The target info of the challenge is carried into the NTLMv2 blob.
    #[test]
    fn ntlm_target_info_is_carried_into_the_blob() {
        let server_challenge = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let mut av_pairs = av_pair(0x0002, b"WORKGROUP");
        av_pairs.extend_from_slice(&av_pair(0x0000, b""));

        let challenge = ntlm_challenge_message_with_info(server_challenge, b"DOMAIN", &av_pairs);
        let message =
            ntlm_authenticate_message("DOMAIN\\user", "Password", &challenge).expect("message");

        // The NTLM response is the proof followed by the blob.
        let ntlm_len = u16::from_le_bytes(message[44..46].try_into().unwrap());
        let ntlm_offset = u32::from_le_bytes(message[48..52].try_into().unwrap()) as usize;
        let blob = &message[ntlm_offset + 16..ntlm_offset + ntlm_len as usize];

        // The AV pairs follow the fixed part of the blob.
        assert!(
            blob.windows(av_pairs.len()).any(|w| w == av_pairs),
            "the target info must be carried into the blob"
        );
    }

    /// A challenge without target info produces a blob without AV pairs.
    #[test]
    fn ntlm_without_target_info_has_no_av_pairs() {
        let server_challenge = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let challenge = ntlm_challenge_message(server_challenge, b"DOMAIN");
        let message =
            ntlm_authenticate_message("DOMAIN\\user", "Password", &challenge).expect("message");

        let ntlm_len = u16::from_le_bytes(message[44..46].try_into().unwrap());
        let ntlm_offset = u32::from_le_bytes(message[48..52].try_into().unwrap()) as usize;
        let blob = &message[ntlm_offset + 16..ntlm_offset + ntlm_len as usize];

        // The fixed part of the blob is 32 bytes, plus the terminator.
        assert_eq!(blob.len(), 36, "no AV pairs are carried");
    }

    /// The timestamp in the blob is a Windows FILETIME.
    #[test]
    fn ntlm_timestamp_is_a_filetime() {
        let server_challenge = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let challenge = ntlm_challenge_message(server_challenge, b"DOMAIN");
        let message =
            ntlm_authenticate_message("DOMAIN\\user", "Password", &challenge).expect("message");

        let ntlm_len = u16::from_le_bytes(message[44..46].try_into().unwrap());
        let ntlm_offset = u32::from_le_bytes(message[48..52].try_into().unwrap()) as usize;
        let blob = &message[ntlm_offset + 16..ntlm_offset + ntlm_len as usize];

        // The timestamp is the third field of the blob.
        let timestamp = u64::from_le_bytes(blob[8..16].try_into().unwrap());
        // 100 ns ticks since 1601: roughly 1.3e17 for a date in the 2020s.
        assert!(
            (100_000_000_000_000_000..200_000_000_000_000_000).contains(&timestamp),
            "unexpected timestamp {timestamp}"
        );
    }

    /// A malformed challenge is rejected.
    #[test]
    fn ntlm_rejects_a_malformed_challenge() {
        // Too short.
        assert!(parse_ntlm_challenge(&[0u8; 8]).is_none());
        // Wrong signature.
        let mut message = ntlm_challenge_message([0u8; 8], b"");
        message[0] = b'X';
        assert!(parse_ntlm_challenge(&message).is_none());
        // Wrong message type.
        let mut message = ntlm_challenge_message([0u8; 8], b"");
        message[8] = 3;
        assert!(parse_ntlm_challenge(&message).is_none());
    }

    /// The NTLM type-3 message must carry consistent field offsets.
    #[test]
    fn ntlm_type3_message_has_consistent_offsets() {
        let server_challenge = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let challenge = ntlm_challenge_message(server_challenge, b"DOMAIN");

        let message =
            ntlm_authenticate_message("DOMAIN\\user", "Password", &challenge).expect("message");

        assert_eq!(&message[0..8], b"NTLMSSP\0");
        assert_eq!(u32::from_le_bytes(message[8..12].try_into().unwrap()), 3);

        // The LM response field: 24 bytes at offset 12.
        let lm_len = u16::from_le_bytes(message[36..38].try_into().unwrap());
        let lm_offset = u32::from_le_bytes(message[40..44].try_into().unwrap());
        assert_eq!(lm_len, 24);
        assert_eq!(lm_offset, 12, "the LM response is at offset 12");

        // The NTLM field descriptor, followed by the response data.
        let ntlm_len = u16::from_le_bytes(message[44..46].try_into().unwrap());
        let ntlm_offset = u32::from_le_bytes(message[48..52].try_into().unwrap());
        assert_eq!(ntlm_offset, 52, "the NTLM response is at offset 52");
        assert!(
            ntlm_len > 16,
            "the NTLM response holds the proof and the blob"
        );

        // The NTLM response is the 16-byte proof followed by the blob.
        let blob = &message[ntlm_offset as usize + 16..ntlm_offset as usize + ntlm_len as usize];
        assert_eq!(&blob[0..2], &[0x01, 0x01], "the blob version");
        assert_eq!(blob.len() % 4, 0, "the blob is a multiple of four bytes");

        // The domain and user name fields follow the NTLM response.
        let fields = 52 + ntlm_len as usize;
        let domain_len = u16::from_le_bytes(message[fields..fields + 2].try_into().unwrap());
        let domain_offset = u32::from_le_bytes(message[fields + 4..fields + 8].try_into().unwrap());
        let user_len = u16::from_le_bytes(message[fields + 8..fields + 10].try_into().unwrap());
        let user_offset = u32::from_le_bytes(message[fields + 12..fields + 16].try_into().unwrap());

        assert_eq!(domain_len, "DOMAIN".len() as u16 * 2, "UTF-16LE length");
        assert_eq!(user_len, "user".len() as u16 * 2, "UTF-16LE length");
        assert_eq!(
            user_offset,
            domain_offset + u32::from(domain_len),
            "the user name follows the domain"
        );

        // The payload really is at the recorded offsets.
        let domain =
            &message[domain_offset as usize..(domain_offset + u32::from(domain_len)) as usize];
        assert_eq!(domain, &utf16le(b"DOMAIN"));
        let user = &message[user_offset as usize..(user_offset + u32::from(user_len)) as usize];
        assert_eq!(user, &utf16le(b"user"));

        // The message ends with the domain and the user name.
        assert_eq!(
            message.len(),
            (user_offset + u32::from(user_len)) as usize,
            "no trailing bytes"
        );
    }

    /// Without a target name the domain comes from the user name.
    #[test]
    fn ntlm_type3_falls_back_to_the_user_name_domain() {
        let server_challenge = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let challenge = ntlm_challenge_message(server_challenge, b"");

        let message =
            ntlm_authenticate_message("WORKGROUP\\user", "Password", &challenge).expect("message");

        let ntlm_len = u16::from_le_bytes(message[44..46].try_into().unwrap());
        let fields = 52 + ntlm_len as usize;
        let domain_len = u16::from_le_bytes(message[fields..fields + 2].try_into().unwrap());
        let domain_offset = u32::from_le_bytes(message[fields + 4..fields + 8].try_into().unwrap());
        let domain =
            &message[domain_offset as usize..(domain_offset + u32::from(domain_len)) as usize];
        assert_eq!(domain, &utf16le(b"WORKGROUP"));
    }

    #[test]
    fn basic_value_is_sensitive() {
        let value = basic_value("user", "pass");
        assert!(value.is_sensitive());
    }
}
