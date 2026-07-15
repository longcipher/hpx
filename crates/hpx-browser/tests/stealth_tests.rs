#![allow(missing_docs)]
use hpx_browser::{
    page::Page,
    stealth::{chrome_148_macos, chrome_148_windows, firefox_135_macos, iphone_15_pro_safari_18},
};

// ── Stealth Profile Validation ───────────────────────────────────────────

#[test]
fn chrome_148_windows_valid() {
    let p = chrome_148_windows();
    assert!(
        p.validate().is_ok(),
        "chrome_148_windows should be valid: {:?}",
        p.validate()
    );
    assert_eq!(p.browser_name, "Chrome");
}

#[test]
fn chrome_148_macos_valid() {
    let p = chrome_148_macos();
    assert!(
        p.validate().is_ok(),
        "chrome_148_macos should be valid: {:?}",
        p.validate()
    );
}

#[test]
fn firefox_135_macos_valid() {
    let p = firefox_135_macos();
    assert!(
        p.validate().is_ok(),
        "firefox_135_macos should be valid: {:?}",
        p.validate()
    );
    assert_eq!(p.browser_name, "Firefox");
}

#[test]
fn safari_iphone_valid() {
    let p = iphone_15_pro_safari_18();
    assert!(
        p.validate().is_ok(),
        "iphone_15_pro_safari_18 should be valid: {:?}",
        p.validate()
    );
    assert_eq!(p.browser_name, "Safari");
}

// ── Navigator Properties (V8 Required) ───────────────────────────────────

#[cfg(feature = "v8")]
async fn stealth_page() -> Page {
    let mut page = Page::from_html("<html><body></body></html>", true)
        .await
        .unwrap();
    page.set_profile(chrome_148_windows());
    page
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn navigator_webdriver_is_false() {
    let mut page = stealth_page().await;
    let result = page.evaluate("navigator.webdriver").unwrap();
    assert_eq!(
        result, "false",
        "navigator.webdriver should be false in stealth mode"
    );
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn navigator_webdriver_descriptor_removed() {
    let mut page = stealth_page().await;
    let result = page.evaluate(
        "Object.getOwnPropertyDescriptor(navigator, 'webdriver') === undefined ? 'true' : 'false'",
    ).unwrap();
    assert_eq!(result, "true", "webdriver descriptor should be removed");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn navigator_user_agent_set() {
    let mut page = stealth_page().await;
    let ua = page.evaluate("navigator.userAgent").unwrap();
    assert!(ua.contains("Chrome"), "UA should contain Chrome: {ua}");
    assert!(ua.contains("Mozilla"), "UA should contain Mozilla: {ua}");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn navigator_platform_set() {
    let mut page = stealth_page().await;
    let platform = page.evaluate("navigator.platform").unwrap();
    assert!(!platform.is_empty(), "platform should not be empty");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn navigator_languages_array() {
    let mut page = stealth_page().await;
    let result = page.evaluate("Array.isArray(navigator.languages)").unwrap();
    assert_eq!(result, "true", "navigator.languages should be an array");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn screen_dimensions_positive() {
    let mut page = stealth_page().await;
    let width: i32 = page.evaluate("screen.width").unwrap().parse().unwrap();
    let height: i32 = page.evaluate("screen.height").unwrap().parse().unwrap();
    assert!(width > 0, "screen width should be positive: {width}");
    assert!(height > 0, "screen height should be positive: {height}");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn chrome_object_exists() {
    let mut page = stealth_page().await;
    let result = page.evaluate("typeof chrome").unwrap();
    assert_eq!(result, "object", "chrome object should exist");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn chrome_csi_returns_timing() {
    let mut page = stealth_page().await;
    let result = page.evaluate("typeof chrome.csi().onloadT").unwrap();
    assert_eq!(result, "number", "chrome.csi().onloadT should be a number");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn plugins_count_nonzero() {
    let mut page = stealth_page().await;
    let count: i32 = page
        .evaluate("navigator.plugins.length")
        .unwrap()
        .parse()
        .unwrap();
    assert!(count > 0, "plugins count should be > 0: {count}");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn performance_memory_exists() {
    let mut page = stealth_page().await;
    let result = page
        .evaluate("typeof performance.memory.jsHeapSizeLimit")
        .unwrap();
    assert_eq!(
        result, "number",
        "performance.memory.jsHeapSizeLimit should exist"
    );
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn user_agent_data_brands() {
    let mut page = stealth_page().await;
    let count: i32 = page
        .evaluate("navigator.userAgentData.brands.length")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(count, 3, "should have 3 brands (grease)");
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn chromium_brand_exists() {
    let mut page = stealth_page().await;
    let result = page
        .evaluate("navigator.userAgentData.brands.some(function(b){return b.brand==='Chromium'})")
        .unwrap();
    assert_eq!(result, "true", "Chromium brand should exist");
}

// ── Consistency Checks ───────────────────────────────────────────────────

#[cfg(feature = "v8")]
#[tokio::test]
async fn user_agent_consistent_with_platform() {
    let mut page = stealth_page().await;
    let ua = page.evaluate("navigator.userAgent").unwrap();
    let platform = page.evaluate("navigator.platform").unwrap();

    // Chrome on Windows should have Win32 platform
    if ua.contains("Windows") {
        assert_eq!(platform, "Win32", "Windows UA should have Win32 platform");
    }
}

#[cfg(feature = "v8")]
#[tokio::test]
async fn multiple_evaluations_consistent() {
    let mut page = stealth_page().await;
    let ua1 = page.evaluate("navigator.userAgent").unwrap();
    let ua2 = page.evaluate("navigator.userAgent").unwrap();
    assert_eq!(ua1, ua2, "UA should be consistent across evaluations");
}

// ── Page-Level Stealth Flag ──────────────────────────────────────────────

#[tokio::test]
async fn stealth_flag_set_on_page() {
    let page = Page::from_html("<html><body></body></html>", true)
        .await
        .unwrap();
    assert!(page.stealth(), "stealth flag should be true");
}

#[tokio::test]
async fn non_stealth_flag_not_set() {
    let page = Page::from_html("<html><body></body></html>", false)
        .await
        .unwrap();
    assert!(!page.stealth(), "stealth flag should be false");
}

// ── Behavioral Profile ───────────────────────────────────────────────────

#[test]
fn mouse_trajectory_reasonable_length() {
    use hpx_browser::stealth::{BehaviorProfile, mouse_trajectory};
    let bp = BehaviorProfile::default();
    let points = mouse_trajectory((100.0, 100.0), (500.0, 500.0), 50.0, &bp);
    assert!(points.len() > 5, "trajectory should have enough points");
}

#[test]
fn keystroke_timings_positive() {
    use hpx_browser::stealth::{BehaviorProfile, keystroke_timings};
    let bp = BehaviorProfile::default();
    let timings = keystroke_timings("hello world", &bp);
    assert!(!timings.is_empty(), "should produce timings");
    for t in &timings {
        assert!(t.dwell_ms > 0.0, "dwell_ms should be positive");
        assert!(t.flight_ms >= 0.0, "flight_ms should be non-negative");
    }
}
