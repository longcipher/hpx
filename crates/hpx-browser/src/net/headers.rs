//! Ordered browser header construction for Chrome/Firefox/Safari.
//!
//! Anti-bot systems check both the presence and order of HTTP headers.
//! This module builds headers in the exact order each browser sends them.

use crate::stealth::{DeviceClass, StealthProfile};

/// Browser-aware nav header dispatch.
pub fn nav_headers(profile: &StealthProfile, accept_ch_upgraded: bool) -> Vec<(String, String)> {
    match profile.browser_name.as_str() {
        "Firefox" => firefox_headers(profile),
        "Safari" => safari_headers(profile),
        _ if accept_ch_upgraded => chrome_headers_with_accept_ch(profile),
        _ => chrome_headers(profile),
    }
}

/// URL-aware nav header dispatch with per-region accept-language.
pub fn nav_headers_for_url(
    profile: &StealthProfile,
    url: &str,
    accept_ch_upgraded: bool,
) -> Vec<(String, String)> {
    let mut hdrs = nav_headers(profile, accept_ch_upgraded);
    apply_region_accept_language(&mut hdrs, url, &profile.browser_name);
    hdrs
}

/// Replace `accept-language` in `hdrs` with the region-appropriate value.
pub fn apply_region_accept_language(hdrs: &mut [(String, String)], url: &str, browser_name: &str) {
    let Some(langs) = region_languages_for_url(url) else {
        return;
    };
    let value = match browser_name {
        "Firefox" => build_firefox_accept_language(&langs),
        "Safari" => build_safari_accept_language(&langs),
        _ => build_accept_language(&langs),
    };
    for (k, v) in hdrs.iter_mut() {
        if k.eq_ignore_ascii_case("accept-language") {
            *v = value;
            return;
        }
    }
}

/// Per-TLD regional language list.
pub fn region_languages_for_url(url: &str) -> Option<Vec<String>> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let host = host.trim_start_matches("www.");
    let tld = if host.ends_with(".co.jp") {
        ".co.jp"
    } else if host.ends_with(".com.br") {
        ".com.br"
    } else if host.ends_with(".com.mx") {
        ".com.mx"
    } else if host.ends_with(".com.tr") {
        ".com.tr"
    } else if host.ends_with(".com.cn") {
        ".com.cn"
    } else {
        let dot = host.rfind('.')?;
        &host[dot..]
    };
    let langs: &[&str] = match tld {
        ".fr" => &["fr-FR", "fr", "en-US", "en"],
        ".de" => &["de-DE", "de", "en-US", "en"],
        ".co.jp" | ".jp" => &["ja-JP", "ja", "en-US", "en"],
        ".it" => &["it-IT", "it", "en-US", "en"],
        ".es" => &["es-ES", "es", "en-US", "en"],
        ".nl" => &["nl-NL", "nl", "en-US", "en"],
        ".pl" => &["pl-PL", "pl", "en-US", "en"],
        ".se" => &["sv-SE", "sv", "en-US", "en"],
        ".no" => &["nb-NO", "no", "en-US", "en"],
        ".dk" => &["da-DK", "da", "en-US", "en"],
        ".fi" => &["fi-FI", "fi", "en-US", "en"],
        ".pt" => &["pt-PT", "pt", "en-US", "en"],
        ".com.br" => &["pt-BR", "pt", "en-US", "en"],
        ".com.mx" => &["es-MX", "es", "en-US", "en"],
        ".com.tr" | ".tr" => &["tr-TR", "tr", "en-US", "en"],
        ".com.cn" | ".cn" => &["zh-CN", "zh", "en-US", "en"],
        ".ru" => &["ru-RU", "ru", "en-US", "en"],
        ".kr" => &["ko-KR", "ko", "en-US", "en"],
        ".tw" => &["zh-TW", "zh", "en-US", "en"],
        ".vn" => &["vi-VN", "vi", "en-US", "en"],
        _ => return None,
    };
    Some(langs.iter().map(|s| (*s).to_string()).collect())
}

/// Browser-aware reload nav header dispatch.
pub fn nav_headers_reload(
    profile: &StealthProfile,
    referer: &str,
    accept_ch_upgraded: bool,
) -> Vec<(String, String)> {
    match profile.browser_name.as_str() {
        "Firefox" => firefox_headers_reload(profile, referer),
        "Safari" => safari_headers_reload(profile, referer),
        _ => chrome_headers_reload(profile, referer, accept_ch_upgraded),
    }
}

/// Browser-aware fetch (XHR/`window.fetch`) header dispatch.
pub fn nav_headers_fetch(
    profile: &StealthProfile,
    target_url: &str,
    origin: Option<&str>,
) -> Vec<(String, String)> {
    let mut hdrs = match profile.browser_name.as_str() {
        "Firefox" => firefox_headers_fetch(profile, target_url, origin),
        "Safari" => safari_headers_fetch(profile, target_url, origin),
        _ => chrome_headers_fetch(profile, target_url, origin),
    };
    let key_url = origin.unwrap_or(target_url);
    apply_region_accept_language(&mut hdrs, key_url, &profile.browser_name);
    hdrs
}

// ============================================================================
// Chrome header builders
// ============================================================================

pub fn chrome_headers(profile: &StealthProfile) -> Vec<(String, String)> {
    chrome_headers_impl(profile, false)
}

pub fn chrome_headers_with_accept_ch(profile: &StealthProfile) -> Vec<(String, String)> {
    chrome_headers_impl(profile, true)
}

pub fn chrome_headers_reload(
    profile: &StealthProfile,
    referer: &str,
    accept_ch_upgraded: bool,
) -> Vec<(String, String)> {
    let mut hdrs: Vec<(String, String)> = chrome_headers_impl(profile, accept_ch_upgraded)
        .into_iter()
        .filter(|(k, _)| k != "sec-fetch-user")
        .map(|(k, v)| {
            if k == "sec-fetch-site" {
                (k, "same-origin".to_string())
            } else {
                (k, v)
            }
        })
        .collect();
    hdrs.push(("referer".to_string(), referer.to_string()));
    hdrs
}

pub fn chrome_headers_fetch(
    profile: &StealthProfile,
    target_url: &str,
    origin: Option<&str>,
) -> Vec<(String, String)> {
    let mut headers = Vec::with_capacity(12);

    headers.push(("user-agent".to_string(), profile.user_agent.clone()));
    headers.push(("accept".to_string(), "*/*".to_string()));

    let sec_ch_ua = build_sec_ch_ua(profile);
    headers.push(("sec-ch-ua".to_string(), sec_ch_ua));
    let is_mobile = matches!(
        profile.device_class,
        DeviceClass::MobileAndroid | DeviceClass::MobileIOS
    );
    headers.push((
        "sec-ch-ua-mobile".to_string(),
        if is_mobile { "?1" } else { "?0" }.to_string(),
    ));
    headers.push((
        "sec-ch-ua-platform".to_string(),
        format!("\"{}\"", profile.os_name),
    ));

    let site = match origin {
        Some(o) => compute_sec_fetch_site(target_url, o),
        None => "cross-site",
    };
    headers.push(("sec-fetch-site".to_string(), site.to_string()));
    headers.push(("sec-fetch-mode".to_string(), "cors".to_string()));
    headers.push(("sec-fetch-dest".to_string(), "empty".to_string()));

    headers.push((
        "accept-encoding".to_string(),
        "gzip, deflate, br, zstd".to_string(),
    ));
    headers.push((
        "accept-language".to_string(),
        build_accept_language(&profile.languages),
    ));
    headers.push(("priority".to_string(), "u=1, i".to_string()));

    if let Some(o) = origin {
        headers.push(("origin".to_string(), o.to_string()));
        headers.push((
            "referer".to_string(),
            format!("{}/", o.trim_end_matches('/')),
        ));
    }

    headers
}

fn chrome_headers_impl(
    profile: &StealthProfile,
    include_high_entropy: bool,
) -> Vec<(String, String)> {
    let mut headers = Vec::with_capacity(if include_high_entropy { 20 } else { 13 });

    // 1. sec-ch-ua
    let sec_ch_ua = build_sec_ch_ua(profile);
    headers.push(("sec-ch-ua".to_string(), sec_ch_ua));
    let is_mobile = matches!(
        profile.device_class,
        DeviceClass::MobileAndroid | DeviceClass::MobileIOS
    );
    // 2. sec-ch-ua-mobile
    headers.push((
        "sec-ch-ua-mobile".to_string(),
        if is_mobile { "?1" } else { "?0" }.to_string(),
    ));
    // 3. sec-ch-ua-platform
    headers.push((
        "sec-ch-ua-platform".to_string(),
        format!("\"{}\"", profile.os_name),
    ));

    if include_high_entropy {
        headers.push((
            "sec-ch-ua-arch".to_string(),
            format!("\"{}\"", profile.cpu_architecture),
        ));
        headers.push((
            "sec-ch-ua-bitness".to_string(),
            format!("\"{}\"", profile.cpu_bitness),
        ));
        headers.push((
            "sec-ch-ua-full-version-list".to_string(),
            build_sec_ch_ua_full_version_list(profile),
        ));
        headers.push((
            "sec-ch-ua-full-version".to_string(),
            format!("\"{}\"", profile.browser_version),
        ));
        headers.push((
            "sec-ch-ua-model".to_string(),
            format!("\"{}\"", profile.ua_model),
        ));
        headers.push((
            "sec-ch-ua-platform-version".to_string(),
            format!(
                "\"{}\"",
                chrome_platform_version(&profile.os_name, &profile.os_version)
            ),
        ));
        headers.push((
            "sec-ch-ua-wow64".to_string(),
            if profile.ua_wow64 { "?1" } else { "?0" }.to_string(),
        ));
        headers.push((
            "sec-ch-ua-form-factors".to_string(),
            if is_mobile {
                "\"Mobile\""
            } else {
                "\"Desktop\""
            }
            .to_string(),
        ));
        headers.push((
            "sec-ch-device-memory".to_string(),
            format!(
                "{}",
                quantize_device_memory(f64::from(profile.device_memory))
            ),
        ));
    }

    // 4. upgrade-insecure-requests
    headers.push(("upgrade-insecure-requests".to_string(), "1".to_string()));
    // 5. user-agent
    headers.push(("user-agent".to_string(), profile.user_agent.clone()));
    // 6. accept
    headers.push(("accept".to_string(),
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7".to_string(),
    ));
    // 7. sec-fetch headers
    headers.push(("sec-fetch-site".to_string(), "none".to_string()));
    headers.push(("sec-fetch-mode".to_string(), "navigate".to_string()));
    headers.push(("sec-fetch-user".to_string(), "?1".to_string()));
    headers.push(("sec-fetch-dest".to_string(), "document".to_string()));
    // 8. accept-encoding
    headers.push((
        "accept-encoding".to_string(),
        "gzip, deflate, br, zstd".to_string(),
    ));
    // 9. accept-language
    headers.push((
        "accept-language".to_string(),
        build_accept_language(&profile.languages),
    ));
    // 10. priority
    headers.push(("priority".to_string(), "u=0, i".to_string()));

    headers
}

// ============================================================================
// Firefox header builders
// ============================================================================

pub fn firefox_headers(profile: &StealthProfile) -> Vec<(String, String)> {
    firefox_headers_impl(profile, "none", true)
}

pub fn firefox_headers_reload(profile: &StealthProfile, referer: &str) -> Vec<(String, String)> {
    let mut hdrs = firefox_headers_impl(profile, "same-origin", false);
    hdrs.push(("referer".to_string(), referer.to_string()));
    hdrs
}

pub fn firefox_headers_fetch(
    profile: &StealthProfile,
    target_url: &str,
    origin: Option<&str>,
) -> Vec<(String, String)> {
    let mut headers = Vec::with_capacity(10);
    headers.push(("user-agent".to_string(), profile.user_agent.clone()));
    headers.push(("accept".to_string(), "*/*".to_string()));
    headers.push((
        "accept-language".to_string(),
        build_firefox_accept_language(&profile.languages),
    ));
    headers.push((
        "accept-encoding".to_string(),
        "gzip, deflate, br, zstd".to_string(),
    ));

    let site = match origin {
        Some(o) => compute_sec_fetch_site(target_url, o),
        None => "cross-site",
    };
    headers.push(("sec-fetch-dest".to_string(), "empty".to_string()));
    headers.push(("sec-fetch-mode".to_string(), "cors".to_string()));
    headers.push(("sec-fetch-site".to_string(), site.to_string()));

    if let Some(o) = origin {
        headers.push(("origin".to_string(), o.to_string()));
        headers.push((
            "referer".to_string(),
            format!("{}/", o.trim_end_matches('/')),
        ));
    }

    headers
}

fn firefox_headers_impl(
    profile: &StealthProfile,
    sec_fetch_site: &str,
    include_sec_fetch_user: bool,
) -> Vec<(String, String)> {
    let mut headers = Vec::with_capacity(9);

    headers.push(("user-agent".to_string(), profile.user_agent.clone()));
    headers.push((
        "accept".to_string(),
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".to_string(),
    ));
    headers.push((
        "accept-language".to_string(),
        build_firefox_accept_language(&profile.languages),
    ));
    headers.push((
        "accept-encoding".to_string(),
        "gzip, deflate, br, zstd".to_string(),
    ));
    headers.push(("upgrade-insecure-requests".to_string(), "1".to_string()));
    headers.push(("sec-fetch-dest".to_string(), "document".to_string()));
    headers.push(("sec-fetch-mode".to_string(), "navigate".to_string()));
    headers.push(("sec-fetch-site".to_string(), sec_fetch_site.to_string()));
    if include_sec_fetch_user {
        headers.push(("sec-fetch-user".to_string(), "?1".to_string()));
    }

    headers
}

// ============================================================================
// Safari header builders
// ============================================================================

pub fn safari_headers(profile: &StealthProfile) -> Vec<(String, String)> {
    safari_headers_impl(profile, None)
}

pub fn safari_headers_reload(profile: &StealthProfile, referer: &str) -> Vec<(String, String)> {
    safari_headers_impl(profile, Some(referer))
}

pub fn safari_headers_fetch(
    profile: &StealthProfile,
    target_url: &str,
    origin: Option<&str>,
) -> Vec<(String, String)> {
    let mut headers = Vec::with_capacity(7);
    headers.push(("accept".to_string(), "*/*".to_string()));
    headers.push((
        "accept-language".to_string(),
        build_safari_accept_language(&profile.languages),
    ));
    headers.push((
        "accept-encoding".to_string(),
        "gzip, deflate, br".to_string(),
    ));
    headers.push(("user-agent".to_string(), profile.user_agent.clone()));
    if let Some(o) = origin {
        headers.push(("origin".to_string(), o.to_string()));
        headers.push((
            "referer".to_string(),
            format!("{}/", o.trim_end_matches('/')),
        ));
    }
    let _ = target_url;
    headers
}

fn safari_headers_impl(profile: &StealthProfile, referer: Option<&str>) -> Vec<(String, String)> {
    let mut headers = Vec::with_capacity(9);

    headers.push(("sec-fetch-dest".to_string(), "document".to_string()));
    headers.push(("user-agent".to_string(), profile.user_agent.clone()));
    headers.push((
        "accept".to_string(),
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".to_string(),
    ));
    let site = if referer.is_some() {
        "same-origin"
    } else {
        "none"
    };
    headers.push(("sec-fetch-site".to_string(), site.to_string()));
    headers.push(("sec-fetch-mode".to_string(), "navigate".to_string()));
    headers.push((
        "accept-language".to_string(),
        build_safari_accept_language(&profile.languages),
    ));
    headers.push(("priority".to_string(), "u=0, i".to_string()));
    headers.push((
        "accept-encoding".to_string(),
        "gzip, deflate, br".to_string(),
    ));
    if let Some(r) = referer {
        headers.push(("referer".to_string(), r.to_string()));
    }

    headers
}

// ============================================================================
// Accept-Language builders
// ============================================================================

fn build_accept_language(languages: &[String]) -> String {
    if languages.is_empty() {
        return "en-US,en;q=0.9".to_string();
    }
    let mut parts = Vec::with_capacity(languages.len());
    for (i, lang) in languages.iter().enumerate() {
        if i == 0 {
            parts.push(lang.clone());
        } else {
            let q = 1.0 - (i as f64 * 0.1);
            if q > 0.0 {
                parts.push(format!("{};q={:.1}", lang, q));
            }
        }
    }
    parts.join(",")
}

fn build_firefox_accept_language(languages: &[String]) -> String {
    if languages.is_empty() {
        return "en-US,en;q=0.5".to_string();
    }
    let mut parts = Vec::with_capacity(languages.len());
    for (i, lang) in languages.iter().enumerate() {
        if i == 0 {
            parts.push(lang.clone());
        } else {
            let q = 0.5 - ((i - 1) as f64 * 0.2);
            if q > 0.0 {
                parts.push(format!("{};q={:.1}", lang, q));
            }
        }
    }
    parts.join(",")
}

fn build_safari_accept_language(languages: &[String]) -> String {
    build_accept_language(languages)
}

// ============================================================================
// Client Hints helpers
// ============================================================================

fn build_sec_ch_ua(profile: &StealthProfile) -> String {
    let major_version = profile.browser_version.split('.').next().unwrap_or("147");
    format!(
        "\"Google Chrome\";v=\"{v}\", \"Not.A/Brand\";v=\"8\", \"Chromium\";v=\"{v}\"",
        v = major_version
    )
}

fn build_sec_ch_ua_full_version_list(profile: &StealthProfile) -> String {
    let v = &profile.browser_version;
    format!("\"Google Chrome\";v=\"{v}\", \"Not.A/Brand\";v=\"8.0.0.0\", \"Chromium\";v=\"{v}\"")
}

fn chrome_platform_version(os_name: &str, os_version: &str) -> String {
    let parts: Vec<&str> = os_version.split('.').collect();
    if parts.len() >= 3 {
        return os_version.to_string();
    }
    match os_name {
        "Windows" | "macOS" => match parts.len() {
            1 => format!("{}.0.0", parts[0]),
            2 => format!("{}.{}.0", parts[0], parts[1]),
            _ => os_version.to_string(),
        },
        "Linux" => String::new(),
        _ => os_version.to_string(),
    }
}

fn quantize_device_memory(gb: f64) -> f64 {
    const SPEC: [f64; 6] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
    if gb < SPEC[0] {
        return SPEC[0];
    }
    let mut out = SPEC[0];
    for &v in &SPEC {
        if v <= gb {
            out = v;
        }
    }
    out
}

// ============================================================================
// Helpers
// ============================================================================

fn compute_sec_fetch_site(target_url: &str, origin: &str) -> &'static str {
    let t = url::Url::parse(target_url).ok();
    let o = url::Url::parse(origin).ok();
    match (t, o) {
        (Some(tu), Some(ou)) => {
            if tu.host_str() == ou.host_str() {
                "same-origin"
            } else if same_site(&tu, &ou) {
                "same-site"
            } else {
                "cross-site"
            }
        }
        _ => "cross-site",
    }
}

fn same_site(a: &url::Url, b: &url::Url) -> bool {
    fn tail2(u: &url::Url) -> Option<String> {
        let host = u.host_str()?;
        let mut parts: Vec<&str> = host.rsplit('.').collect();
        if parts.len() < 2 {
            return Some(host.to_string());
        }
        parts.truncate(2);
        parts.reverse();
        Some(parts.join("."))
    }
    tail2(a) == tail2(b)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stealth::presets::*;

    #[test]
    fn accept_language_single() {
        assert_eq!(build_accept_language(&["en-US".into()]), "en-US");
    }

    #[test]
    fn accept_language_multiple() {
        assert_eq!(
            build_accept_language(&["en-US".into(), "en".into()]),
            "en-US,en;q=0.9"
        );
    }

    #[test]
    fn accept_language_empty() {
        assert_eq!(build_accept_language(&[]), "en-US,en;q=0.9");
    }

    #[test]
    fn firefox_accept_language_uses_q_05() {
        assert_eq!(
            build_firefox_accept_language(&["en-US".into(), "en".into()]),
            "en-US,en;q=0.5"
        );
    }

    #[test]
    fn chrome_headers_first_visit_count() {
        let profile = chrome_147_windows();
        let headers = chrome_headers(&profile);
        assert_eq!(headers.len(), 13);
        let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
        for required in &["sec-ch-ua", "sec-ch-ua-mobile", "sec-ch-ua-platform"] {
            assert!(names.contains(required), "missing {required}");
        }
        for forbidden in &[
            "sec-ch-ua-arch",
            "sec-ch-ua-bitness",
            "sec-ch-ua-full-version-list",
            "sec-ch-ua-model",
            "sec-ch-ua-platform-version",
            "sec-ch-ua-wow64",
        ] {
            assert!(
                !names.contains(forbidden),
                "{forbidden} leaked on first visit"
            );
        }
    }

    #[test]
    fn chrome_headers_accept_ch_includes_high_entropy() {
        let profile = chrome_147_windows();
        let headers = chrome_headers_with_accept_ch(&profile);
        let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
        for required in &[
            "sec-ch-ua-arch",
            "sec-ch-ua-bitness",
            "sec-ch-ua-full-version-list",
            "sec-ch-ua-model",
            "sec-ch-ua-platform-version",
            "sec-ch-ua-wow64",
        ] {
            assert!(names.contains(required), "missing {required}");
        }
    }

    #[test]
    fn firefox_headers_have_no_sec_ch_ua() {
        let profile = firefox_135_macos();
        let headers = firefox_headers(&profile);
        for (k, _) in &headers {
            assert!(!k.starts_with("sec-ch-ua"), "Firefox must not send {k}");
        }
        assert!(!headers.iter().any(|(k, _)| k == "priority"));
    }

    #[test]
    fn firefox_headers_have_correct_accept() {
        let profile = firefox_135_macos();
        let headers = firefox_headers(&profile);
        let accept = headers.iter().find(|(k, _)| k == "accept").unwrap();
        assert_eq!(
            accept.1,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
        );
    }

    #[test]
    fn platform_version_triple_padded() {
        assert_eq!(chrome_platform_version("Windows", "10.0"), "10.0.0");
        assert_eq!(chrome_platform_version("Windows", "11"), "11.0.0");
        assert_eq!(chrome_platform_version("macOS", "15.2"), "15.2.0");
        assert_eq!(chrome_platform_version("Linux", "anything"), "");
    }

    #[test]
    fn device_memory_quantizes_to_w3_spec_set() {
        assert_eq!(quantize_device_memory(8.0), 8.0);
        assert_eq!(quantize_device_memory(4.0), 4.0);
        assert_eq!(quantize_device_memory(16.0), 8.0);
        assert_eq!(quantize_device_memory(6.0), 4.0);
        assert_eq!(quantize_device_memory(0.3), 0.25);
        assert_eq!(quantize_device_memory(0.0), 0.25);
    }

    #[test]
    fn region_languages_amazon_fr() {
        let langs = region_languages_for_url("https://www.amazon.fr/").unwrap();
        assert_eq!(langs, vec!["fr-FR", "fr", "en-US", "en"]);
    }

    #[test]
    fn region_languages_amazon_jp() {
        let langs = region_languages_for_url("https://www.amazon.co.jp/").unwrap();
        assert_eq!(langs, vec!["ja-JP", "ja", "en-US", "en"]);
    }

    #[test]
    fn region_languages_amazon_com_no_override() {
        assert!(region_languages_for_url("https://www.amazon.com/").is_none());
    }

    #[test]
    fn region_languages_amazon_co_uk_no_override() {
        assert!(region_languages_for_url("https://www.amazon.co.uk/").is_none());
    }

    #[test]
    fn nav_headers_for_url_overrides_amazon_fr() {
        let profile = chrome_147_macos();
        let hdrs = nav_headers_for_url(&profile, "https://www.amazon.fr/", false);
        let al = hdrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("accept-language"))
            .unwrap();
        assert_eq!(al.1, "fr-FR,fr;q=0.9,en-US;q=0.8,en;q=0.7");
    }

    #[test]
    fn nav_headers_for_url_overrides_amazon_de_firefox() {
        let profile = firefox_135_macos();
        let hdrs = nav_headers_for_url(&profile, "https://www.amazon.de/", false);
        let al = hdrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("accept-language"))
            .unwrap();
        assert_eq!(al.1, "de-DE,de;q=0.5,en-US;q=0.3,en;q=0.1");
    }

    #[test]
    fn pixel_mobile_emits_mobile_client_hints() {
        let profile = pixel_9_pro_chrome_148();
        let headers = chrome_headers_with_accept_ch(&profile);
        let h: std::collections::HashMap<_, _> = headers.iter().cloned().collect();
        assert_eq!(h.get("sec-ch-ua-mobile").map(String::as_str), Some("?1"));
        assert_eq!(
            h.get("sec-ch-ua-platform").map(String::as_str),
            Some("\"Android\"")
        );
        assert_eq!(
            h.get("sec-ch-ua-model").map(String::as_str),
            Some("\"Pixel 9 Pro\"")
        );
        assert_eq!(
            h.get("sec-ch-ua-form-factors").map(String::as_str),
            Some("\"Mobile\"")
        );
    }

    #[test]
    fn desktop_emits_desktop_client_hints() {
        let profile = chrome_147_macos();
        let headers = chrome_headers_with_accept_ch(&profile);
        let h: std::collections::HashMap<_, _> = headers.iter().cloned().collect();
        assert_eq!(h.get("sec-ch-ua-mobile").map(String::as_str), Some("?0"));
        assert_eq!(
            h.get("sec-ch-ua-form-factors").map(String::as_str),
            Some("\"Desktop\"")
        );
    }

    #[test]
    fn safari_headers_have_no_sec_ch_ua() {
        let profile = safari_ios_18();
        let headers = safari_headers(&profile);
        for (k, _) in &headers {
            assert!(!k.starts_with("sec-ch-ua"), "Safari must not send {k}");
        }
    }

    #[test]
    fn fetch_headers_mobile_flag_matches_nav() {
        let pixel = pixel_9_pro_chrome_148();
        let fh: std::collections::HashMap<_, _> = chrome_headers_fetch(
            &pixel,
            "https://example.com/x.js",
            Some("https://example.com"),
        )
        .into_iter()
        .collect();
        assert_eq!(fh.get("sec-ch-ua-mobile").map(String::as_str), Some("?1"));
    }
}
