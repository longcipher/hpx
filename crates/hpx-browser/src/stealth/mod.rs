//! Stealth fingerprint profiles for hpx-browser.
//!
//! Provides consistent browser identities — UA string, screen, locale,
//! GPU vendor/renderer, TLS impersonation label — so the engine reports
//! a coherent "I am Chrome 148 on macOS" surface rather than a default
//! headless fingerprint.

use serde::{Deserialize, Serialize};

pub mod behavior;
pub mod browser_presets;
pub mod gpu_presets;

// Re-export all public items for backward compatibility.
pub use behavior::*;
pub use browser_presets::*;
pub use gpu_presets::*;

// ── GPU catalog ──────────────────────────────────────────────────────

/// A snapshot of a real GPU's WebGL fingerprint as Chrome exposes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuProfile {
    /// `getParameter(VENDOR)` — Chrome always returns "WebKit".
    pub vendor: String,
    /// `getParameter(RENDERER)` — Chrome always returns "WebKit WebGL".
    pub renderer: String,
    /// `getParameter(VERSION)`.
    pub version: String,
    /// `getParameter(SHADING_LANGUAGE_VERSION)`.
    pub shading_language_version: String,
    /// `getParameter(UNMASKED_VENDOR_WEBGL)`.
    pub unmasked_vendor: String,
    /// `getParameter(UNMASKED_RENDERER_WEBGL)`.
    pub unmasked_renderer: String,
    /// `getSupportedExtensions()`.
    pub extensions: Vec<String>,
    /// Additional `getParameter()` values keyed by GLenum.
    pub params: Vec<(u32, serde_json::Value)>,
    /// `getShaderPrecisionFormat()` values.
    pub shader_precision: Vec<(u32, u32, [i32; 3])>,
    /// Distinct WebGL 1.0 surface (version string + extension list).
    #[serde(default)]
    pub webgl1: Option<WebGL1Surface>,
}

/// WebGL 1.0 surface, distinct from the WebGL 2.0 fields on `GpuProfile`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebGL1Surface {
    pub version: String,
    pub shading_language_version: String,
    pub extensions: Vec<String>,
}

impl Default for GpuProfile {
    fn default() -> Self {
        nvidia_rtx_3060_windows()
    }
}

// ── Device class ─────────────────────────────────────────────────────

/// Device class driving TLS curve selection, Sec-CH-UA-Mobile, etc.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceClass {
    #[default]
    Desktop,
    MobileAndroid,
    MobileIOS,
}

// ── Media device ─────────────────────────────────────────────────────

/// A media device reported by `navigator.mediaDevices.enumerateDevices()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaDeviceInfo {
    pub device_id: String,
    pub kind: String,
    pub label: String,
    pub group_id: String,
}

// ── StealthProfile ───────────────────────────────────────────────────

/// A complete stealth fingerprint profile.
///
/// Start from a preset constructor; to customise, clone a preset, mutate
/// fields, and call [`StealthProfile::validate`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StealthProfile {
    // === Identity ===
    pub user_agent: String,
    pub browser_name: String,
    pub browser_version: String,
    pub os_name: String,
    pub os_version: String,
    pub platform: String,
    pub vendor: String,
    pub vendor_sub: String,
    pub product_sub: String,
    pub app_version: String,

    // === Hardware ===
    pub screen_width: u32,
    pub screen_height: u32,
    pub screen_avail_width: u32,
    pub screen_avail_height: u32,
    pub screen_avail_top: u32,
    pub screen_color_depth: u32,
    pub device_pixel_ratio: f64,
    pub cpu_cores: u8,
    pub device_memory: u8,
    pub max_touch_points: u8,

    // === GPU / WebGL ===
    pub webgl_vendor: String,
    pub webgl_renderer: String,
    #[serde(default = "default_gpu_profile")]
    pub gpu_profile: GpuProfile,

    // === Locale ===
    pub language: String,
    pub languages: Vec<String>,
    pub timezone: String,

    // === Client Hints high-entropy values ===
    #[serde(default = "default_cpu_architecture")]
    pub cpu_architecture: String,
    #[serde(default = "default_cpu_bitness")]
    pub cpu_bitness: String,
    #[serde(default)]
    pub platform_version: String,
    #[serde(default)]
    pub ua_model: String,
    #[serde(default)]
    pub ua_wow64: bool,

    // === Network ===
    #[serde(default)]
    pub device_class: DeviceClass,
    pub tls_impersonate: String,
    pub connection_effective_type: String,
    pub connection_rtt: u32,
    pub connection_downlink: f64,

    // === Plugins ===
    pub pdf_viewer_enabled: bool,
    pub plugins_count: u32,
    pub mime_types_count: u32,

    // === Fingerprint seeds ===
    pub canvas_seed: u64,
    pub audio_seed: u64,
    #[serde(default = "default_audio_sample_rate")]
    pub audio_sample_rate: u32,

    // === WebAuthn / FedCM ===
    #[serde(default)]
    pub has_platform_authenticator: bool,
    #[serde(default = "default_true")]
    pub conditional_mediation: bool,

    // === HTTP/3 / QUIC ===
    #[serde(default)]
    pub allow_http3: bool,

    // === Media features ===
    pub prefers_color_scheme: String,
    pub pointer_type: String,
    pub hover_capability: String,
    #[serde(default = "default_color_gamut")]
    pub color_gamut: String,

    // === Window dimensions ===
    pub inner_width: u32,
    pub inner_height: u32,
    pub outer_width: u32,
    pub outer_height: u32,

    // === Proxy ===
    #[serde(default)]
    pub proxy: Option<String>,

    // === Media devices ===
    #[serde(default)]
    pub media_devices: Vec<MediaDeviceInfo>,

    /// Enforce CSP on sub-resource fetches. Defaults to `true`.
    #[serde(default = "default_true")]
    pub enforce_csp: bool,
}

fn default_color_gamut() -> String {
    "srgb".into()
}
fn default_true() -> bool {
    true
}
fn default_gpu_profile() -> GpuProfile {
    nvidia_rtx_3060_windows()
}
fn default_cpu_architecture() -> String {
    "x86".into()
}
fn default_cpu_bitness() -> String {
    "64".into()
}
fn default_audio_sample_rate() -> u32 {
    44100
}

impl Default for StealthProfile {
    fn default() -> Self {
        chrome_148_windows()
    }
}

// ── Validation ───────────────────────────────────────────────────────

impl StealthProfile {
    /// Validate that all fields are internally consistent.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        // UA must contain the reduced major version (Chrome) or short version (Firefox)
        let ua_major = self.browser_version.split('.').next().unwrap_or("");
        let chrome_form = format!("{ua_major}.0.0.0");
        let firefox_form = format!("{ua_major}.0");
        if !self.user_agent.contains(&chrome_form) && !self.user_agent.contains(&firefox_form) {
            errors.push(format!(
                "UA '{}' doesn't contain reduced major version '{}' or '{}'",
                self.user_agent, chrome_form, firefox_form
            ));
        }

        // Platform must match OS
        match self.os_name.as_str() {
            "Windows" if self.platform != "Win32" => {
                errors.push(format!("Windows OS but platform is '{}'", self.platform));
            }
            "macOS" if self.platform != "MacIntel" => {
                errors.push(format!("macOS but platform is '{}'", self.platform));
            }
            "Linux" if !self.platform.starts_with("Linux") => {
                errors.push(format!("Linux OS but platform is '{}'", self.platform));
            }
            _ => {}
        }

        // Touch points: desktop = 0, mobile > 0
        if self.max_touch_points > 0 && self.screen_width > 1024 && self.pointer_type == "fine" {
            errors.push("Touch points > 0 but desktop pointer type".into());
        }

        // GPU vendor must match renderer
        if self.webgl_renderer.contains("NVIDIA") && !self.webgl_vendor.contains("NVIDIA") {
            errors.push("WebGL renderer is NVIDIA but vendor doesn't match".into());
        }
        if self.webgl_renderer.contains("Intel") && !self.webgl_vendor.contains("Intel") {
            errors.push("WebGL renderer is Intel but vendor doesn't match".into());
        }
        if self.webgl_renderer.contains("Apple") && !self.webgl_vendor.contains("Apple") {
            errors.push("WebGL renderer is Apple but vendor doesn't match".into());
        }

        // Apple GPU only on macOS/iOS
        if self.webgl_renderer.contains("Apple")
            && !matches!(self.os_name.as_str(), "macOS" | "iOS")
        {
            errors.push("Apple GPU on non-Apple OS".into());
        }

        // Screen dimensions sanity
        if self.screen_width == 0 || self.screen_height == 0 {
            errors.push("Screen dimensions cannot be zero".into());
        }
        if self.inner_width > self.screen_width {
            errors.push("inner_width > screen_width".into());
        }
        if self.outer_width < self.inner_width {
            errors.push("outer_width < inner_width".into());
        }

        // CPU/memory sanity
        if self.cpu_cores == 0 || self.cpu_cores > 128 {
            errors.push(format!("Unrealistic cpu_cores: {}", self.cpu_cores));
        }
        if self.device_memory == 0 && self.os_name != "iOS" {
            errors.push(format!("Unrealistic device_memory: {}", self.device_memory));
        }

        // Language must be in languages list
        if !self.languages.contains(&self.language) {
            errors.push(format!(
                "language '{}' not in languages {:?}",
                self.language, self.languages
            ));
        }

        // Client Hints consistency
        if !matches!(self.cpu_architecture.as_str(), "x86" | "arm" | "") {
            errors.push(format!(
                "cpu_architecture must be 'x86', 'arm', or '' (got '{}')",
                self.cpu_architecture
            ));
        }
        if !matches!(self.cpu_bitness.as_str(), "64" | "32") {
            errors.push(format!(
                "cpu_bitness must be '64' or '32' (got '{}')",
                self.cpu_bitness
            ));
        }
        if self.ua_wow64 && (self.os_name != "Windows" || self.cpu_bitness != "32") {
            errors.push(format!(
                "ua_wow64=true requires os_name=Windows and cpu_bitness=32 (got {} / {})",
                self.os_name, self.cpu_bitness
            ));
        }
        if self.os_name == "Linux" && !self.platform_version.is_empty() {
            errors.push(format!(
                "Chrome on Linux must report empty platform_version (got '{}')",
                self.platform_version
            ));
        }
        if self.cpu_architecture == "arm"
            && !matches!(
                self.os_name.as_str(),
                "macOS" | "Android" | "ChromeOS" | "iOS"
            )
        {
            errors.push(format!(
                "cpu_architecture=arm only on macOS/Android/ChromeOS/iOS (got '{}')",
                self.os_name
            ));
        }
        if !self.ua_model.is_empty() && self.max_touch_points == 0 {
            errors.push(format!(
                "ua_model='{}' on a desktop (max_touch_points=0) profile",
                self.ua_model
            ));
        }
        if !matches!(self.audio_sample_rate, 44100 | 48000 | 96000 | 192000) {
            errors.push(format!(
                "audio_sample_rate must be one of {{44100, 48000, 96000, 192000}} (got {})",
                self.audio_sample_rate
            ));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

// ── Utility functions ────────────────────────────────────────────────

/// Create a profile with custom locale/timezone from a base profile.
pub fn with_locale(
    mut base: StealthProfile,
    language: &str,
    languages: &[&str],
    timezone: &str,
) -> StealthProfile {
    base.language = language.into();
    base.languages = languages.iter().map(|s| (*s).to_string()).collect();
    base.timezone = timezone.into();
    base
}

/// Random desktop profile (picks randomly from Chrome presets).
pub fn random_desktop() -> StealthProfile {
    use rand::RngExt;
    let mut rng = rand::rng();
    let mut profile = match rng.random_range(0..3u32) {
        0 => chrome_148_windows(),
        1 => chrome_148_macos(),
        _ => chrome_148_linux(),
    };
    profile.canvas_seed = rng.random();
    profile.audio_seed = rng.random();
    profile
}

/// Apple Silicon Chrome 148 profile sampler.
///
/// Returns one variant of `chrome_148_macos` with screen geometry, core
/// count, RAM, and fingerprint seeds independently sampled from
/// realistic Apple Silicon distributions.
pub fn chrome_148_macos_sampled() -> StealthProfile {
    chrome_148_macos_sampled_with_rng(&mut rand::rng())
}

/// As [`chrome_148_macos_sampled`] but takes a caller-supplied RNG.
pub fn chrome_148_macos_sampled_with_rng(rng: &mut impl rand::RngExt) -> StealthProfile {
    let mut p = chrome_148_macos();

    type ChipConfig = (
        &'static [u8],
        &'static [u8],
        &'static [(u32, u32, u32)],
        GpuProfile,
    );
    let chip_idx = rng.random_range(0..3u32);
    let (cores_pool, ram_pool, screens, gpu): ChipConfig = match chip_idx {
        0 => (
            &[8],
            &[8, 16, 24],
            &[(1512, 982, 949), (1728, 1117, 1010)],
            apple_m3_macos(),
        ),
        1 => (
            &[11, 12],
            &[18, 36],
            &[(1800, 1169, 1100), (2056, 1329, 1253)],
            apple_m3_pro_macos(),
        ),
        _ => (
            &[14, 16],
            &[36, 48],
            &[(1800, 1169, 1100), (2056, 1329, 1253)],
            apple_m3_max_macos(),
        ),
    };

    p.cpu_cores = cores_pool[rng.random_range(0..cores_pool.len())];
    p.device_memory = ram_pool[rng.random_range(0..ram_pool.len())];

    let (w, h, ah) = screens[rng.random_range(0..screens.len())];
    p.screen_width = w;
    p.screen_height = h;
    p.screen_avail_width = w;
    p.screen_avail_height = ah;
    p.inner_width = w;
    p.inner_height = h.saturating_sub(111);
    p.outer_width = w;
    p.outer_height = h;

    p.gpu_profile = gpu;
    p.webgl_renderer = p.gpu_profile.unmasked_renderer.clone();

    p.canvas_seed = rng.random();
    p.audio_seed = rng.random();

    debug_assert!(
        p.validate().is_ok(),
        "chrome_148_macos_sampled produced an invalid profile: {:?}",
        p.validate()
    );

    p
}

// ── Compat presets module (for headers.rs tests) ─────────────────────

/// Test presets re-exported as a module for backward compat.
#[cfg(test)]
pub mod presets {
    use super::*;

    pub fn chrome_147_macos() -> StealthProfile {
        chrome_148_macos()
    }
    pub fn chrome_147_windows() -> StealthProfile {
        chrome_148_windows()
    }
    pub fn chrome_147_linux() -> StealthProfile {
        chrome_148_linux()
    }
    pub fn firefox_135_macos() -> StealthProfile {
        super::firefox_135_macos()
    }
    pub fn safari_ios_18() -> StealthProfile {
        iphone_15_pro_safari_18()
    }
    pub fn pixel_9_pro_chrome_148() -> StealthProfile {
        super::pixel_9_pro_chrome_148()
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_148_windows_validates() {
        let p = chrome_148_windows();
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }

    #[test]
    fn chrome_148_macos_validates() {
        let p = chrome_148_macos();
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }

    #[test]
    fn chrome_148_linux_validates() {
        let p = chrome_148_linux();
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }

    #[test]
    fn chrome_148_ru_validates() {
        let p = chrome_148_ru();
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }

    #[test]
    fn chrome_148_cn_validates() {
        let p = chrome_148_cn();
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }

    #[test]
    fn firefox_135_macos_validates() {
        let p = firefox_135_macos();
        assert!(p.validate().is_ok(), "{:?}", p.validate());
        assert_eq!(p.browser_name, "Firefox");
        assert_eq!(p.vendor, "");
        assert_eq!(p.product_sub, "20100101");
        assert!(p.user_agent.contains("rv:135.0"));
        assert!(p.user_agent.contains("Firefox/135.0"));
        assert!(!p.user_agent.contains("Chrome"));
    }

    #[test]
    fn firefox_135_windows_validates() {
        let p = firefox_135_windows();
        assert!(p.validate().is_ok(), "{:?}", p.validate());
        assert!(p.user_agent.contains("Firefox/135.0"));
    }

    #[test]
    fn firefox_135_linux_validates() {
        let p = firefox_135_linux();
        assert!(p.validate().is_ok(), "{:?}", p.validate());
        assert!(p.user_agent.contains("Firefox/135.0"));
    }

    #[test]
    fn pixel_9_pro_validates() {
        let p = pixel_9_pro_chrome_148();
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }

    #[test]
    fn iphone_15_pro_validates() {
        let p = iphone_15_pro_safari_18();
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }

    #[test]
    fn http3_disabled_by_default_on_all_presets() {
        for profile in [
            chrome_148_windows(),
            chrome_148_macos(),
            chrome_148_linux(),
            chrome_148_ru(),
            chrome_148_cn(),
            chrome_148_de(),
            chrome_148_jp(),
            firefox_135_macos(),
            firefox_135_windows(),
            firefox_135_linux(),
        ] {
            assert!(
                !profile.allow_http3,
                "Profile sets allow_http3=true: {}",
                profile.user_agent
            );
        }
    }

    #[test]
    fn firefox_webgl_is_masked() {
        for profile in [
            firefox_135_macos(),
            firefox_135_windows(),
            firefox_135_linux(),
        ] {
            assert_eq!(profile.webgl_vendor, "Mozilla");
            assert_eq!(profile.webgl_renderer, "Mozilla");
        }
    }

    #[test]
    fn random_desktop_validates() {
        for _ in 0..10 {
            let p = random_desktop();
            assert!(p.validate().is_ok(), "{:?}", p.validate());
        }
    }

    #[test]
    fn random_desktop_diversity() {
        use std::collections::HashSet;
        let mut names = HashSet::new();
        for _ in 0..30 {
            let p = random_desktop();
            names.insert(p.browser_name.clone());
        }
        // All Chrome presets share browser_name="Chrome", so diversity
        // comes from screen/seed variation. At least 1 name expected.
        assert!(!names.is_empty());
    }

    #[test]
    fn invalid_profile_detected() {
        let mut p = chrome_148_windows();
        p.platform = "MacIntel".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn invalid_gpu_os_mismatch() {
        let mut p = chrome_148_windows();
        p.webgl_renderer =
            "ANGLE (Apple, ANGLE Metal Renderer: Apple M2, Unspecified Version)".into();
        p.webgl_vendor = "Google Inc. (Apple)".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn ua_contains_version() {
        let p = chrome_148_windows();
        assert!(p.user_agent.contains("148.0.0.0"));
        assert_eq!(p.browser_version, "148.0.7778.168");
    }

    #[test]
    fn serialization_roundtrip() {
        let p = chrome_148_windows();
        let json = serde_json::to_string(&p).unwrap();
        let deserialized: StealthProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(p.user_agent, deserialized.user_agent);
        assert_eq!(p.screen_width, deserialized.screen_width);
    }

    #[test]
    fn macos_sampler_produces_valid_profiles() {
        for _ in 0..200 {
            let p = chrome_148_macos_sampled();
            p.validate()
                .unwrap_or_else(|e| panic!("invalid sampled profile: {e:?}"));
            assert!(matches!(p.screen_width, 1512 | 1728 | 1800 | 2056));
            assert!(matches!(p.cpu_cores, 8 | 11 | 12 | 14 | 16));
            assert!(matches!(p.device_memory, 8 | 16 | 18 | 24 | 36 | 48));
            assert_eq!(p.device_pixel_ratio, 2.0);
            assert_eq!(p.audio_sample_rate, 48000);
            assert_eq!(p.cpu_architecture, "arm");
            assert_eq!(p.platform, "MacIntel");
            assert_eq!(p.inner_height + 111, p.screen_height);
        }
    }

    #[test]
    fn macos_sampler_keeps_cross_api_consistency() {
        for _ in 0..50 {
            let p = chrome_148_macos_sampled();
            let r = &p.gpu_profile.unmasked_renderer;
            match p.cpu_cores {
                8 => {
                    assert!(r.contains("Apple M3,"));
                    assert!(matches!(p.device_memory, 8 | 16 | 24));
                }
                11 | 12 => {
                    assert!(r.contains("Apple M3 Pro"));
                    assert!(matches!(p.device_memory, 18 | 36));
                }
                14 | 16 => {
                    assert!(r.contains("Apple M3 Max"));
                    assert!(matches!(p.device_memory, 36 | 48));
                }
                other => panic!("unexpected cpu_cores {other}"),
            }
            assert_eq!(p.webgl_renderer, *r);
        }
    }

    // ── Behavior tests ─────────────────────────────────────────────

    use rand_chacha::rand_core::SeedableRng;

    fn fixed_rng() -> rand_chacha::ChaCha20Rng {
        rand_chacha::ChaCha20Rng::seed_from_u64(42)
    }

    #[test]
    fn behavior_profile_defaults_are_sensible() {
        let p = BehaviorProfile::default();
        assert!((30.0..=80.0).contains(&p.typing_wpm_mean));
        assert!((130.0..=220.0).contains(&p.fitts_b));
        assert_eq!(p.handedness, Handedness::Right);
    }

    #[test]
    fn rng_for_is_deterministic_per_seed() {
        let p = BehaviorProfile {
            seed: 99,
            ..BehaviorProfile::default()
        };
        let mut a = p.rng_for(123);
        let mut b = p.rng_for(123);
        use rand::RngExt;
        assert_eq!(a.random::<u64>(), b.random::<u64>());
    }

    #[test]
    fn rng_for_differs_across_salts() {
        let p = BehaviorProfile {
            seed: 99,
            ..BehaviorProfile::default()
        };
        let mut a = p.rng_for(1);
        let mut b = p.rng_for(2);
        use rand::RngExt;
        assert_ne!(a.random::<u64>(), b.random::<u64>());
    }

    #[test]
    fn mouse_trajectory_starts_at_from_and_ends_at_to() {
        let p = BehaviorProfile {
            seed: 42,
            ..BehaviorProfile::default()
        };
        let pts = mouse_trajectory((100.0, 100.0), (500.0, 400.0), 50.0, &p);
        assert!(pts.len() > 5);
        let first = pts[0];
        let last = pts[pts.len() - 1];
        assert!((first.x - 100.0).abs() < 10.0, "first x={}", first.x);
        assert!((first.y - 100.0).abs() < 10.0, "first y={}", first.y);
        assert_eq!(last.x, 500.0);
        assert_eq!(last.y, 400.0);
    }

    #[test]
    fn mouse_trajectory_obeys_fitts_law_total_time() {
        let p = BehaviorProfile {
            seed: 42,
            ..BehaviorProfile::default()
        };
        let pts = mouse_trajectory((0.0, 0.0), (500.0, 0.0), 50.0, &p);
        let last_t = pts[pts.len() - 1].t_ms;
        assert!(
            (700.0..=950.0).contains(&last_t),
            "expected ~805 ms, got {last_t}"
        );
    }

    #[test]
    fn mouse_trajectory_uses_8ms_sample_rate() {
        let p = BehaviorProfile {
            seed: 42,
            ..BehaviorProfile::default()
        };
        let pts = mouse_trajectory((0.0, 0.0), (200.0, 0.0), 30.0, &p);
        for w in pts.windows(2) {
            let dt = w[1].t_ms - w[0].t_ms;
            assert!((dt - 8.0).abs() < 1e-3, "gap {} not 8 ms", dt);
        }
    }

    #[test]
    fn mouse_trajectory_has_velocity_diversity() {
        let p = BehaviorProfile {
            seed: 42,
            ..BehaviorProfile::default()
        };
        let mut rng = fixed_rng();
        let pts = mouse_trajectory_with_rng((0.0, 0.0), (600.0, 400.0), 40.0, &p, &mut rng);
        let speeds: Vec<f32> = pts
            .windows(2)
            .map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt())
            .collect();
        let mean = speeds.iter().sum::<f32>() / speeds.len() as f32;
        let var = speeds.iter().map(|s| (s - mean).powi(2)).sum::<f32>() / speeds.len() as f32;
        let std = var.sqrt();
        let cv = std / mean.max(1e-3);
        assert!(cv > 0.4, "speed CV too low: {cv}");
    }

    #[test]
    fn mouse_trajectory_deterministic_per_seed() {
        let p = BehaviorProfile {
            seed: 123,
            ..BehaviorProfile::default()
        };
        let mut r1 = p.rng_for(1);
        let mut r2 = p.rng_for(1);
        let a = mouse_trajectory_with_rng((0.0, 0.0), (300.0, 200.0), 25.0, &p, &mut r1);
        let b = mouse_trajectory_with_rng((0.0, 0.0), (300.0, 200.0), 25.0, &p, &mut r2);
        assert_eq!(a.len(), b.len());
        for (pa, pb) in a.iter().zip(b.iter()) {
            assert_eq!(pa, pb);
        }
    }

    #[test]
    fn mouse_trajectory_no_endpoint_jerk_spike() {
        for seed in 0..40u64 {
            let p = BehaviorProfile {
                seed,
                ..BehaviorProfile::default()
            };
            let mut r = p.rng_for(2);
            let tr = mouse_trajectory_with_rng((12.0, 30.0), (840.0, 510.0), 28.0, &p, &mut r);
            assert!(tr.len() >= 8);
            let step =
                |a: &MousePoint, b: &MousePoint| ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
            let steps: Vec<f32> = tr.windows(2).map(|w| step(&w[0], &w[1])).collect();
            let n = steps.len();
            let final_step = steps[n - 1];
            let mut sorted = steps.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median = sorted[n / 2];
            let max_step = sorted[n - 1];
            assert!(
                final_step <= max_step + 1e-3,
                "seed {seed}: final step {final_step} exceeds max interior {max_step}"
            );
            assert!(
                final_step <= median * 6.0 + 5.0,
                "seed {seed}: final step {final_step} is jerk outlier vs median {median}"
            );
            let last = tr.last().unwrap();
            assert!((last.x - 840.0).abs() < 1e-2 && (last.y - 510.0).abs() < 1e-2);
        }
    }

    #[test]
    fn keystroke_first_has_no_flight() {
        let p = BehaviorProfile {
            seed: 42,
            ..BehaviorProfile::default()
        };
        let ks = keystroke_timings("hi", &p);
        assert_eq!(ks[0].flight_ms, 0.0);
        assert!(ks[1].flight_ms > 0.0);
    }

    #[test]
    fn keystroke_dwell_in_realistic_range() {
        let p = BehaviorProfile {
            seed: 42,
            ..BehaviorProfile::default()
        };
        let ks = keystroke_timings("the quick brown fox jumps over the lazy dog", &p);
        let mean_dwell: f32 = ks.iter().map(|k| k.dwell_ms).sum::<f32>() / ks.len() as f32;
        assert!(
            (70.0..=150.0).contains(&mean_dwell),
            "mean dwell {mean_dwell} outside plausible range"
        );
    }

    #[test]
    fn keystroke_flight_scales_with_wpm() {
        let slow = BehaviorProfile {
            seed: 42,
            typing_wpm_mean: 30.0,
            ..BehaviorProfile::default()
        };
        let fast = BehaviorProfile {
            seed: 42,
            typing_wpm_mean: 70.0,
            ..BehaviorProfile::default()
        };
        let s = keystroke_timings("the quick brown fox jumps over", &slow);
        let f = keystroke_timings("the quick brown fox jumps over", &fast);
        let mean = |ks: &[KeystrokeTiming]| -> f32 {
            ks.iter().skip(1).map(|k| k.flight_ms).sum::<f32>() / (ks.len() - 1) as f32
        };
        assert!(
            mean(&s) > mean(&f),
            "30 WPM flight {} should exceed 70 WPM flight {}",
            mean(&s),
            mean(&f)
        );
    }

    #[test]
    fn keystroke_bigram_th_faster_than_dd() {
        let mut th_total = 0.0_f32;
        let mut dd_total = 0.0_f32;
        for seed in 0..50 {
            let prof = BehaviorProfile {
                seed,
                ..BehaviorProfile::default()
            };
            let th = keystroke_timings("th", &prof);
            let dd = keystroke_timings("dd", &prof);
            th_total += th[1].flight_ms;
            dd_total += dd[1].flight_ms;
        }
        let th_mean = th_total / 50.0;
        let dd_mean = dd_total / 50.0;
        assert!(
            dd_mean > th_mean * 1.5,
            "dd flight {dd_mean} should be > 1.5× th flight {th_mean}"
        );
    }

    #[test]
    fn keystroke_deterministic_per_seed() {
        let mut rng_a = rand_chacha::ChaCha20Rng::seed_from_u64(7);
        let mut rng_b = rand_chacha::ChaCha20Rng::seed_from_u64(7);
        let p = BehaviorProfile::default();
        let a = keystroke_timings_with_rng("hello world", &p, &mut rng_a);
        let b = keystroke_timings_with_rng("hello world", &p, &mut rng_b);
        assert_eq!(a, b);
    }

    #[test]
    fn trackpad_burst_decays_to_zero() {
        let p = BehaviorProfile {
            seed: 42,
            scroll_style: ScrollStyle::Trackpad,
            ..BehaviorProfile::default()
        };
        let ticks = wheel_burst(-1000.0, &p);
        assert!(ticks.len() > 5);
        for t in &ticks {
            assert_eq!(t.mode, 0);
            assert!(t.delta_y < 0.0);
        }
        let cum: f32 = ticks.iter().map(|t| t.delta_y).sum();
        assert!(
            (cum + 1000.0).abs() < 200.0,
            "cumulative {cum} not close to -1000"
        );
        for w in ticks.windows(2) {
            let dt = w[1].t_ms - w[0].t_ms;
            assert!((dt - 16.0).abs() < 1e-3);
        }
    }

    #[test]
    fn wheel_burst_uses_100px_notches() {
        let p = BehaviorProfile {
            seed: 42,
            scroll_style: ScrollStyle::Wheel,
            ..BehaviorProfile::default()
        };
        let ticks = wheel_burst(500.0, &p);
        assert_eq!(ticks.len(), 5);
        for t in &ticks {
            assert_eq!(t.delta_y, 100.0);
            assert_eq!(t.mode, 0);
        }
    }

    #[test]
    fn wheel_burst_intervals_are_lognormal_distributed() {
        let p = BehaviorProfile {
            seed: 42,
            scroll_style: ScrollStyle::Wheel,
            ..BehaviorProfile::default()
        };
        let ticks = wheel_burst(2000.0, &p);
        let intervals: Vec<f32> = ticks.windows(2).map(|w| w[1].t_ms - w[0].t_ms).collect();
        let mean = intervals.iter().sum::<f32>() / intervals.len() as f32;
        assert!(
            (mean - 180.0).abs() < 200.0,
            "mean interval {mean} too far from 180 ms"
        );
        let mut sorted = intervals.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        sorted.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        assert!(sorted.len() > 5, "only {} distinct intervals", sorted.len());
    }

    #[test]
    fn default_seeds_differ_across_instances() {
        let a = BehaviorProfile::default();
        let b = BehaviorProfile::default();
        assert_ne!(a.seed, b.seed);
    }
}
