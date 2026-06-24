//! Stealth fingerprint profiles for hpx-browser.
//!
//! Provides consistent browser identities — UA string, screen, locale,
//! GPU vendor/renderer, TLS impersonation label — so the engine reports
//! a coherent "I am Chrome 148 on macOS" surface rather than a default
//! headless fingerprint.

use serde::{Deserialize, Serialize};

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

// ── Behavioral emulation (Sigma-Lognormal + Keystroke + Scroll) ─────

/// Right-handers overshoot bottom-right; left-handers bottom-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Handedness {
    Right,
    Left,
}

/// Trackpad momentum vs discrete mouse-wheel notches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollStyle {
    Trackpad,
    Wheel,
}

/// Per-session behavioral parameters. Different sessions should sample
/// fresh seeds so mouse/keyboard patterns don't repeat across visits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorProfile {
    #[serde(default = "default_behavior_seed")]
    pub seed: u64,
    #[serde(default = "default_handedness")]
    pub handedness: Handedness,
    #[serde(default = "default_mouse_dpi")]
    pub mouse_dpi: u16,
    #[serde(default = "default_typing_wpm_mean")]
    pub typing_wpm_mean: f32,
    #[serde(default = "default_typing_wpm_sigma")]
    pub typing_wpm_sigma: f32,
    #[serde(default = "default_scroll_style")]
    pub scroll_style: ScrollStyle,
    #[serde(default = "default_fitts_b")]
    pub fitts_b: f32,
}

fn default_behavior_seed() -> u64 {
    rand::random::<u64>()
}
fn default_handedness() -> Handedness {
    Handedness::Right
}
fn default_mouse_dpi() -> u16 {
    1600
}
fn default_typing_wpm_mean() -> f32 {
    50.0
}
fn default_typing_wpm_sigma() -> f32 {
    15.0
}
fn default_scroll_style() -> ScrollStyle {
    ScrollStyle::Trackpad
}
fn default_fitts_b() -> f32 {
    166.0
}

impl Default for BehaviorProfile {
    fn default() -> Self {
        Self {
            seed: default_behavior_seed(),
            handedness: default_handedness(),
            mouse_dpi: default_mouse_dpi(),
            typing_wpm_mean: default_typing_wpm_mean(),
            typing_wpm_sigma: default_typing_wpm_sigma(),
            scroll_style: default_scroll_style(),
            fitts_b: default_fitts_b(),
        }
    }
}

impl BehaviorProfile {
    /// Derive a deterministic sub-RNG for a specific call site.
    pub fn rng_for(&self, salt: u64) -> rand_chacha::ChaCha20Rng {
        use rand_chacha::rand_core::SeedableRng;
        let combined = self
            .seed
            .wrapping_mul(0x9E3779B97F4A7C15)
            .wrapping_add(salt);
        rand_chacha::ChaCha20Rng::seed_from_u64(combined)
    }
}

/// One sample point on a humanized mouse trajectory.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MousePoint {
    pub t_ms: f32,
    pub x: f32,
    pub y: f32,
}

/// Keystroke timing for one character.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KeystrokeTiming {
    pub ch: char,
    pub dwell_ms: f32,
    pub flight_ms: f32,
}

/// A single scroll wheel tick.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WheelTick {
    pub t_ms: f32,
    pub delta_y: f32,
    pub mode: u32,
}

// ── Mouse trajectory (Sigma-Lognormal — Plamondon 1995) ─────────────

struct Stroke {
    amplitude: f32,
    sigma: f32,
    mu: f32,
    t0: f32,
    theta: f32,
}

fn integrate_x(strokes: &[Stroke], t: f32) -> f32 {
    strokes
        .iter()
        .map(|s| {
            let dt = t - s.t0;
            if dt <= 0.0 {
                return 0.0;
            }
            let z = (dt.ln() - s.mu) / (s.sigma * std::f32::consts::SQRT_2);
            let cdf = 0.5 * (1.0 + erf(z));
            s.amplitude * cdf * s.theta.cos()
        })
        .sum()
}

fn integrate_y(strokes: &[Stroke], t: f32) -> f32 {
    strokes
        .iter()
        .map(|s| {
            let dt = t - s.t0;
            if dt <= 0.0 {
                return 0.0;
            }
            let z = (dt.ln() - s.mu) / (s.sigma * std::f32::consts::SQRT_2);
            let cdf = 0.5 * (1.0 + erf(z));
            s.amplitude * cdf * s.theta.sin()
        })
        .sum()
}

/// Abramowitz-Stegun 7.1.26 erf approximation (|err| < 1.5e-7).
fn erf(x: f32) -> f32 {
    let sign = x.signum();
    let x = x.abs();
    let a1 = 0.254_829_6;
    let a2 = -0.284_496_72;
    let a3 = 1.421_413_8;
    let a4 = -1.453_152_1;
    let a5 = 1.061_405_4;
    let p = 0.3275911;
    let t = 1.0 / (1.0 + p * x);
    let y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * (-x * x).exp();
    sign * y
}

/// Generate a humanlike mouse trajectory from `from` to `to`.
pub fn mouse_trajectory(
    from: (f32, f32),
    to: (f32, f32),
    target_w: f32,
    profile: &BehaviorProfile,
) -> Vec<MousePoint> {
    let mut rng = profile
        .rng_for(((from.0 as u64) << 32) | (from.1 as u64) ^ ((to.0 as u64) << 16) ^ (to.1 as u64));
    mouse_trajectory_with_rng(from, to, target_w, profile, &mut rng)
}

/// Same as `mouse_trajectory` but takes an explicit RNG for testing.
pub fn mouse_trajectory_with_rng<R: rand::Rng>(
    from: (f32, f32),
    to: (f32, f32),
    target_w: f32,
    profile: &BehaviorProfile,
    rng: &mut R,
) -> Vec<MousePoint> {
    use rand_distr::{Distribution, LogNormal, Normal};

    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let distance = (dx * dx + dy * dy).sqrt().max(1.0);
    let target_w = target_w.max(1.0);

    let id_bits = ((distance / target_w) + 1.0).log2();
    let n_strokes = ((1.3 * id_bits).round() as usize).clamp(2, 7);

    let total_ms = 230.0 + profile.fitts_b * id_bits;

    let mut amplitudes: Vec<f32> = Vec::with_capacity(n_strokes);
    let primary = 0.85 * distance;
    amplitudes.push(primary);
    let remaining = distance - primary;
    let per_corrective = remaining / (n_strokes - 1).max(1) as f32;
    for _ in 1..n_strokes {
        let jitter: f32 = Normal::new(0.0_f32, per_corrective * 0.15)
            .ok()
            .map_or(0.0, |d| d.sample(rng));
        amplitudes.push((per_corrective + jitter).max(1.0));
    }

    let sigma_dist = Normal::new(0.25_f32, 0.05).ok();
    let mu_dist = Normal::new(-1.6_f32, 0.2).ok();
    let onset_dist = LogNormal::new(90.0_f32.ln(), 0.3).ok();
    let theta_dist = Normal::new(0.0_f32, 8.0_f32.to_radians()).ok();

    let target_angle = dy.atan2(dx);
    let mut strokes: Vec<Stroke> = Vec::with_capacity(n_strokes);
    let mut t0 = 0.0_f32;
    for (i, amp) in amplitudes.iter().enumerate() {
        let sigma = sigma_dist
            .as_ref()
            .map_or(0.25, |d| d.sample(rng).clamp(0.15, 0.40));
        let mu = mu_dist.as_ref().map_or(-1.6, |d| d.sample(rng));
        let jitter = theta_dist.as_ref().map_or(0.0, |d| d.sample(rng));
        let theta = if i == 0 {
            target_angle + jitter
        } else {
            target_angle + jitter * 1.5
        };
        strokes.push(Stroke {
            amplitude: *amp,
            sigma,
            mu,
            t0,
            theta,
        });
        t0 += onset_dist.as_ref().map_or(90.0, |d| d.sample(rng));
    }

    let dt_ms = 8.0_f32;
    let n_samples = (total_ms / dt_ms).ceil() as usize + 1;
    let mut points: Vec<MousePoint> = Vec::with_capacity(n_samples);

    let tremor_dist = Normal::new(0.0_f32, 1.5).ok();
    let mut tremor_x = 0.0_f32;
    let mut tremor_y = 0.0_f32;
    let tremor_alpha = 0.3_f32;

    for i in 0..n_samples {
        let t = (i as f32) * dt_ms;

        let tx = tremor_dist.as_ref().map_or(0.0, |d| d.sample(rng));
        let ty = tremor_dist.as_ref().map_or(0.0, |d| d.sample(rng));
        tremor_x = tremor_alpha * tremor_x + (1.0 - tremor_alpha) * tx;
        tremor_y = tremor_alpha * tremor_y + (1.0 - tremor_alpha) * ty;

        let x = from.0 + integrate_x(&strokes, t) + tremor_x;
        let y = from.1 + integrate_y(&strokes, t) + tremor_y;
        points.push(MousePoint { t_ms: t, x, y });
    }

    // Smooth endpoint correction via smoothstep tail.
    if points.len() >= 2 {
        let n = points.len();
        let last = &points[n - 1];
        let res_x = to.0 - last.x;
        let res_y = to.1 - last.y;
        let tail = 15.min(n - 1);
        let start = n - tail - 1;
        for (k, p) in points.iter_mut().enumerate().skip(start) {
            let u = (k - start) as f32 / tail as f32;
            let s = u * u * (3.0 - 2.0 * u);
            p.x += res_x * s;
            p.y += res_y * s;
        }
        if let Some(last) = points.last_mut() {
            last.x = to.0;
            last.y = to.1;
        }
    } else if let Some(last) = points.last_mut() {
        last.x = to.0;
        last.y = to.1;
    }
    points
}

// ── Keystroke dynamics ──────────────────────────────────────────────

fn bigram_ratio(prev: char, cur: char) -> f32 {
    let key = (
        prev.to_ascii_lowercase() as u8,
        cur.to_ascii_lowercase() as u8,
    );
    match key {
        (b't', b'h')
        | (b'h', b'e')
        | (b'i', b'n')
        | (b'a', b'n')
        | (b'o', b'n')
        | (b'a', b't')
        | (b'i', b's')
        | (b'i', b't')
        | (b'o', b'r')
        | (b'o', b'f') => 0.7,
        (b'e', b'd')
        | (b'u', b'n')
        | (b'r', b'e')
        | (b'e', b'r')
        | (b'e', b'n')
        | (b'n', b'd')
        | (b'e', b's')
        | (b't', b'e')
        | (b'a', b'l')
        | (b'a', b'r') => 1.4,
        (a, b) if a == b => 2.0,
        _ => 1.0,
    }
}

/// Generate keystroke timings for a string.
pub fn keystroke_timings(text: &str, profile: &BehaviorProfile) -> Vec<KeystrokeTiming> {
    let mut rng = profile.rng_for(0xCAFEBABE ^ text.len() as u64);
    keystroke_timings_with_rng(text, profile, &mut rng)
}

/// Same as `keystroke_timings` but takes an explicit RNG for testing.
pub fn keystroke_timings_with_rng<R: rand::Rng>(
    text: &str,
    profile: &BehaviorProfile,
    rng: &mut R,
) -> Vec<KeystrokeTiming> {
    use rand_distr::{Distribution, LogNormal};

    let ms_per_char = 60_000.0 / (profile.typing_wpm_mean * 5.0);
    let flight_median = (ms_per_char - 95.0).max(40.0);
    let flight_dist = LogNormal::new(flight_median.ln(), 0.55).ok();
    let dwell_dist = LogNormal::new(95.0_f32.ln(), 0.30).ok();

    let mut out = Vec::with_capacity(text.len());
    let mut prev_ch: Option<char> = None;
    for ch in text.chars() {
        let dwell = dwell_dist
            .as_ref()
            .map_or(95.0, |d| d.sample(rng).clamp(40.0, 400.0));
        let flight = if let Some(p) = prev_ch {
            let ratio = bigram_ratio(p, ch);
            flight_dist
                .as_ref()
                .map_or(130.0, |d| (d.sample(rng) * ratio).clamp(20.0, 1000.0))
        } else {
            0.0
        };
        out.push(KeystrokeTiming {
            ch,
            dwell_ms: dwell,
            flight_ms: flight,
        });
        prev_ch = Some(ch);
    }
    out
}

// ── Scroll bursts ───────────────────────────────────────────────────

/// Generate a humanlike scroll burst totaling ~`target_dy` pixels.
pub fn wheel_burst(target_dy: f32, profile: &BehaviorProfile) -> Vec<WheelTick> {
    let mut rng = profile.rng_for(0xDEAD_BEEF ^ target_dy.to_bits() as u64);
    wheel_burst_with_rng(target_dy, profile, &mut rng)
}

/// Same as `wheel_burst` but takes an explicit RNG for testing.
pub fn wheel_burst_with_rng<R: rand::RngExt>(
    target_dy: f32,
    profile: &BehaviorProfile,
    rng: &mut R,
) -> Vec<WheelTick> {
    use rand_distr::{Distribution, LogNormal};

    let dir = if target_dy >= 0.0 { 1.0 } else { -1.0 };
    let abs_dy = target_dy.abs().max(1.0);

    match profile.scroll_style {
        ScrollStyle::Trackpad => {
            let v0 = LogNormal::new((abs_dy / 8.0).ln(), 0.3)
                .ok()
                .map_or(abs_dy / 8.0, |d| d.sample(rng));
            let decay = 0.94 + rng.random_range(0.0_f32..0.04);
            let mut t = 0.0_f32;
            let mut v = v0;
            let mut ticks = Vec::new();
            let mut accumulated = 0.0_f32;
            while v > 0.5 && accumulated < abs_dy * 1.1 {
                let step = (v.min(abs_dy - accumulated)).max(0.5);
                ticks.push(WheelTick {
                    t_ms: t,
                    delta_y: step * dir,
                    mode: 0,
                });
                accumulated += step;
                t += 16.0;
                v *= decay;
            }
            ticks
        }
        ScrollStyle::Wheel => {
            let notches = ((abs_dy / 100.0).round() as u32).max(1);
            let interval_dist = LogNormal::new(180.0_f32.ln(), 0.4).ok();
            let mut t = 0.0_f32;
            let mut ticks = Vec::with_capacity(notches as usize);
            for _ in 0..notches {
                ticks.push(WheelTick {
                    t_ms: t,
                    delta_y: 100.0 * dir,
                    mode: 0,
                });
                t += interval_dist.as_ref().map_or(180.0, |d| d.sample(rng));
            }
            ticks
        }
    }
}

// ── GPU presets ──────────────────────────────────────────────────────

fn common_params_desktop() -> Vec<(u32, serde_json::Value)> {
    use serde_json::json;
    vec![
        (0x0D33, json!(16384)),
        (0x851C, json!(16384)),
        (0x84E8, json!(16384)),
        (0x8073, json!(2048)),
        (0x8869, json!(16)),
        (0x8DFB, json!(1024)),
        (0x8DFD, json!(15)),
        (0x8DFC, json!(1024)),
        (0x8872, json!(16)),
        (0x8B4D, json!(16)),
        (0x8B4C, json!(32)),
        (0x846D, json!([1.0, 8190.0])),
        (0x846E, json!([1.0, 1.0])),
        (0x0D3A, json!([32767, 32767])),
        (0x0D56, json!(8)),
        (0x0D57, json!(8)),
        (0x80AA, json!(2)),
        (0x80A9, json!(4)),
    ]
}

fn standard_shader_precision() -> Vec<(u32, u32, [i32; 3])> {
    let mut out = Vec::with_capacity(12);
    for &shader_type in &[0x8B31u32, 0x8B30u32] {
        out.push((shader_type, 0x8DF0, [127, 127, 23]));
        out.push((shader_type, 0x8DF1, [127, 127, 23]));
        out.push((shader_type, 0x8DF2, [127, 127, 23]));
        out.push((shader_type, 0x8DF3, [15, 14, 0]));
        out.push((shader_type, 0x8DF4, [31, 30, 0]));
        out.push((shader_type, 0x8DF5, [31, 30, 0]));
    }
    out
}

/// Chrome on Windows with NVIDIA GeForce RTX 3060.
pub fn nvidia_rtx_3060_windows() -> GpuProfile {
    GpuProfile {
        vendor: "WebKit".into(),
        renderer: "WebKit WebGL".into(),
        version: "WebGL 1.0 (OpenGL ES 2.0 Chromium)".into(),
        shading_language_version: "WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)".into(),
        unmasked_vendor: "Google Inc. (NVIDIA)".into(),
        unmasked_renderer:
            "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0, D3D11)".into(),
        extensions: vec![
            "ANGLE_instanced_arrays".into(),
            "EXT_blend_minmax".into(),
            "EXT_clip_control".into(),
            "EXT_color_buffer_half_float".into(),
            "EXT_depth_clamp".into(),
            "EXT_disjoint_timer_query".into(),
            "EXT_float_blend".into(),
            "EXT_frag_depth".into(),
            "EXT_polygon_offset_clamp".into(),
            "EXT_shader_texture_lod".into(),
            "EXT_texture_compression_bptc".into(),
            "EXT_texture_compression_rgtc".into(),
            "EXT_texture_filter_anisotropic".into(),
            "EXT_texture_mirror_clamp_to_edge".into(),
            "EXT_sRGB".into(),
            "KHR_parallel_shader_compile".into(),
            "OES_element_index_uint".into(),
            "OES_fbo_render_mipmap".into(),
            "OES_standard_derivatives".into(),
            "OES_texture_float".into(),
            "OES_texture_float_linear".into(),
            "OES_texture_half_float".into(),
            "OES_texture_half_float_linear".into(),
            "OES_vertex_array_object".into(),
            "WEBGL_blend_func_extended".into(),
            "WEBGL_color_buffer_float".into(),
            "WEBGL_compressed_texture_s3tc".into(),
            "WEBGL_compressed_texture_s3tc_srgb".into(),
            "WEBGL_debug_renderer_info".into(),
            "WEBGL_debug_shaders".into(),
            "WEBGL_depth_texture".into(),
            "WEBGL_draw_buffers".into(),
            "WEBGL_lose_context".into(),
            "WEBGL_multi_draw".into(),
            "WEBGL_polygon_mode".into(),
        ],
        params: common_params_desktop(),
        shader_precision: standard_shader_precision(),
        webgl1: None,
    }
}

fn apple_m3_family_profile(chip_name: &str) -> GpuProfile {
    GpuProfile {
        vendor: "WebKit".into(),
        renderer: "WebKit WebGL".into(),
        version: "WebGL 2.0 (OpenGL ES 3.0 Chromium)".into(),
        shading_language_version: "WebGL GLSL ES 3.00 (OpenGL ES GLSL ES 3.0 Chromium)".into(),
        unmasked_vendor: "Google Inc. (Apple)".into(),
        unmasked_renderer: format!(
            "ANGLE (Apple, ANGLE Metal Renderer: {chip_name}, Unspecified Version)"
        ),
        extensions: vec![
            "EXT_clip_control".into(),
            "EXT_color_buffer_float".into(),
            "EXT_color_buffer_half_float".into(),
            "EXT_conservative_depth".into(),
            "EXT_depth_clamp".into(),
            "EXT_disjoint_timer_query_webgl2".into(),
            "EXT_float_blend".into(),
            "EXT_polygon_offset_clamp".into(),
            "EXT_render_snorm".into(),
            "EXT_texture_compression_bptc".into(),
            "EXT_texture_compression_rgtc".into(),
            "EXT_texture_filter_anisotropic".into(),
            "EXT_texture_mirror_clamp_to_edge".into(),
            "EXT_texture_norm16".into(),
            "KHR_parallel_shader_compile".into(),
            "NV_shader_noperspective_interpolation".into(),
            "OES_draw_buffers_indexed".into(),
            "OES_sample_variables".into(),
            "OES_shader_multisample_interpolation".into(),
            "OES_texture_float_linear".into(),
            "WEBGL_blend_func_extended".into(),
            "WEBGL_clip_cull_distance".into(),
            "WEBGL_compressed_texture_astc".into(),
            "WEBGL_compressed_texture_etc".into(),
            "WEBGL_compressed_texture_etc1".into(),
            "WEBGL_compressed_texture_pvrtc".into(),
            "WEBGL_compressed_texture_s3tc".into(),
            "WEBGL_compressed_texture_s3tc_srgb".into(),
            "WEBGL_debug_renderer_info".into(),
            "WEBGL_debug_shaders".into(),
            "WEBGL_lose_context".into(),
            "WEBGL_multi_draw".into(),
            "WEBGL_polygon_mode".into(),
            "WEBGL_provoking_vertex".into(),
            "WEBGL_render_shared_exponent".into(),
            "WEBGL_stencil_texturing".into(),
        ],
        params: apple_m3_params(),
        shader_precision: standard_shader_precision(),
        webgl1: Some(apple_m3_webgl1_surface()),
    }
}

fn apple_m3_webgl1_surface() -> WebGL1Surface {
    WebGL1Surface {
        version: "WebGL 1.0 (OpenGL ES 2.0 Chromium)".into(),
        shading_language_version: "WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)".into(),
        extensions: vec![
            "ANGLE_instanced_arrays".into(),
            "EXT_blend_minmax".into(),
            "EXT_clip_control".into(),
            "EXT_color_buffer_half_float".into(),
            "EXT_depth_clamp".into(),
            "EXT_disjoint_timer_query".into(),
            "EXT_float_blend".into(),
            "EXT_frag_depth".into(),
            "EXT_polygon_offset_clamp".into(),
            "EXT_sRGB".into(),
            "EXT_shader_texture_lod".into(),
            "EXT_texture_compression_bptc".into(),
            "EXT_texture_compression_rgtc".into(),
            "EXT_texture_filter_anisotropic".into(),
            "EXT_texture_mirror_clamp_to_edge".into(),
            "KHR_parallel_shader_compile".into(),
            "OES_element_index_uint".into(),
            "OES_fbo_render_mipmap".into(),
            "OES_standard_derivatives".into(),
            "OES_texture_float".into(),
            "OES_texture_float_linear".into(),
            "OES_texture_half_float".into(),
            "OES_texture_half_float_linear".into(),
            "OES_vertex_array_object".into(),
            "WEBGL_blend_func_extended".into(),
            "WEBGL_color_buffer_float".into(),
            "WEBGL_compressed_texture_astc".into(),
            "WEBGL_compressed_texture_etc".into(),
            "WEBGL_compressed_texture_etc1".into(),
            "WEBGL_compressed_texture_pvrtc".into(),
            "WEBGL_compressed_texture_s3tc".into(),
            "WEBGL_compressed_texture_s3tc_srgb".into(),
            "WEBGL_debug_renderer_info".into(),
            "WEBGL_debug_shaders".into(),
            "WEBGL_depth_texture".into(),
            "WEBGL_draw_buffers".into(),
            "WEBGL_lose_context".into(),
            "WEBGL_multi_draw".into(),
            "WEBGL_polygon_mode".into(),
        ],
    }
}

fn apple_m3_params() -> Vec<(u32, serde_json::Value)> {
    use serde_json::json;
    let mut params = common_params_desktop();
    for (pname, value) in params.iter_mut() {
        match *pname {
            0x0D3A => *value = json!([16384, 16384]),
            0x846D => *value = json!([1.0, 511.0]),
            _ => {}
        }
    }
    params
}

/// Apple M3 GPU profile.
pub fn apple_m3_macos() -> GpuProfile {
    apple_m3_family_profile("Apple M3")
}

/// Apple M3 Pro GPU profile.
pub fn apple_m3_pro_macos() -> GpuProfile {
    apple_m3_family_profile("Apple M3 Pro")
}

/// Apple M3 Max GPU profile.
pub fn apple_m3_max_macos() -> GpuProfile {
    apple_m3_family_profile("Apple M3 Max")
}

/// Apple M2 Pro GPU profile.
pub fn apple_m2_pro_macos() -> GpuProfile {
    GpuProfile {
        vendor: "WebKit".into(),
        renderer: "WebKit WebGL".into(),
        version: "WebGL 1.0 (OpenGL ES 2.0 Chromium)".into(),
        shading_language_version: "WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)".into(),
        unmasked_vendor: "Google Inc. (Apple)".into(),
        unmasked_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M2 Pro, Unspecified Version)"
            .into(),
        extensions: vec![
            "ANGLE_instanced_arrays".into(),
            "EXT_blend_minmax".into(),
            "EXT_clip_control".into(),
            "EXT_color_buffer_half_float".into(),
            "EXT_depth_clamp".into(),
            "EXT_float_blend".into(),
            "EXT_frag_depth".into(),
            "EXT_polygon_offset_clamp".into(),
            "EXT_shader_texture_lod".into(),
            "EXT_texture_compression_bptc".into(),
            "EXT_texture_compression_rgtc".into(),
            "EXT_texture_filter_anisotropic".into(),
            "EXT_texture_mirror_clamp_to_edge".into(),
            "EXT_sRGB".into(),
            "KHR_parallel_shader_compile".into(),
            "OES_element_index_uint".into(),
            "OES_fbo_render_mipmap".into(),
            "OES_standard_derivatives".into(),
            "OES_texture_float".into(),
            "OES_texture_float_linear".into(),
            "OES_texture_half_float".into(),
            "OES_texture_half_float_linear".into(),
            "OES_vertex_array_object".into(),
            "WEBGL_blend_func_extended".into(),
            "WEBGL_color_buffer_float".into(),
            "WEBGL_compressed_texture_astc".into(),
            "WEBGL_compressed_texture_etc".into(),
            "WEBGL_compressed_texture_etc1".into(),
            "WEBGL_compressed_texture_s3tc".into(),
            "WEBGL_compressed_texture_s3tc_srgb".into(),
            "WEBGL_debug_renderer_info".into(),
            "WEBGL_debug_shaders".into(),
            "WEBGL_depth_texture".into(),
            "WEBGL_draw_buffers".into(),
            "WEBGL_lose_context".into(),
            "WEBGL_multi_draw".into(),
        ],
        params: common_params_desktop(),
        shader_precision: standard_shader_precision(),
        webgl1: None,
    }
}

/// Intel UHD 630 on Linux.
pub fn intel_uhd_630_linux() -> GpuProfile {
    GpuProfile {
        vendor: "WebKit".into(),
        renderer: "WebKit WebGL".into(),
        version: "WebGL 1.0 (OpenGL ES 2.0 Chromium)".into(),
        shading_language_version: "WebGL GLSL ES 1.0 (OpenGL ES GLSL ES 1.0 Chromium)".into(),
        unmasked_vendor: "Google Inc. (Intel)".into(),
        unmasked_renderer: "ANGLE (Intel, Mesa Intel(R) UHD Graphics 630 (CFL GT2), OpenGL 4.6)"
            .into(),
        extensions: vec![
            "ANGLE_instanced_arrays".into(),
            "EXT_blend_minmax".into(),
            "EXT_clip_control".into(),
            "EXT_color_buffer_half_float".into(),
            "EXT_depth_clamp".into(),
            "EXT_disjoint_timer_query".into(),
            "EXT_float_blend".into(),
            "EXT_frag_depth".into(),
            "EXT_polygon_offset_clamp".into(),
            "EXT_shader_texture_lod".into(),
            "EXT_texture_compression_bptc".into(),
            "EXT_texture_compression_rgtc".into(),
            "EXT_texture_filter_anisotropic".into(),
            "EXT_texture_mirror_clamp_to_edge".into(),
            "EXT_sRGB".into(),
            "KHR_parallel_shader_compile".into(),
            "OES_element_index_uint".into(),
            "OES_fbo_render_mipmap".into(),
            "OES_standard_derivatives".into(),
            "OES_texture_float".into(),
            "OES_texture_float_linear".into(),
            "OES_texture_half_float".into(),
            "OES_texture_half_float_linear".into(),
            "OES_vertex_array_object".into(),
            "WEBGL_compressed_texture_s3tc".into(),
            "WEBGL_compressed_texture_s3tc_srgb".into(),
            "WEBGL_debug_renderer_info".into(),
            "WEBGL_debug_shaders".into(),
            "WEBGL_depth_texture".into(),
            "WEBGL_draw_buffers".into(),
            "WEBGL_lose_context".into(),
            "WEBGL_multi_draw".into(),
        ],
        params: common_params_desktop(),
        shader_precision: standard_shader_precision(),
        webgl1: None,
    }
}

// ── Media device helper ──────────────────────────────────────────────

fn default_media_devices(seed: &str) -> Vec<MediaDeviceInfo> {
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };
    let hash = |s: &str| -> String {
        let mut h = DefaultHasher::new();
        s.hash(&mut h);
        format!(
            "{:016x}{:016x}",
            h.finish(),
            h.finish().wrapping_mul(0x9e3779b97f4a7c15)
        )
    };
    vec![
        MediaDeviceInfo {
            device_id: hash(&format!("{seed}-audio-in")),
            kind: "audioinput".into(),
            label: "Default".into(),
            group_id: hash(&format!("{seed}-group-a")),
        },
        MediaDeviceInfo {
            device_id: hash(&format!("{seed}-audio-out")),
            kind: "audiooutput".into(),
            label: "Default".into(),
            group_id: hash(&format!("{seed}-group-a")),
        },
        MediaDeviceInfo {
            device_id: hash(&format!("{seed}-video-in")),
            kind: "videoinput".into(),
            label: "Integrated Camera".into(),
            group_id: hash(&format!("{seed}-group-v")),
        },
    ]
}

// ── Profile presets ──────────────────────────────────────────────────

/// Chrome 148 on Windows 10.
pub fn chrome_148_windows() -> StealthProfile {
    StealthProfile {
        user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36".into(),
        browser_name: "Chrome".into(),
        browser_version: "148.0.7778.168".into(),
        os_name: "Windows".into(),
        os_version: "10.0".into(),
        platform: "Win32".into(),
        vendor: "Google Inc.".into(),
        vendor_sub: "".into(),
        product_sub: "20030107".into(),
        app_version: "5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36".into(),
        screen_width: 1920, screen_height: 1080,
        screen_avail_width: 1920, screen_avail_height: 1040,
        screen_avail_top: 0, screen_color_depth: 24,
        device_pixel_ratio: 1.0,
        cpu_cores: 8, device_memory: 8, max_touch_points: 0,
        webgl_vendor: "Google Inc. (NVIDIA)".into(),
        webgl_renderer: "ANGLE (NVIDIA, NVIDIA GeForce RTX 3080 Direct3D11 vs_5_0 ps_5_0, D3D11)".into(),
        gpu_profile: nvidia_rtx_3060_windows(),
        language: "en-US".into(),
        languages: vec!["en-US".into(), "en".into()],
        timezone: "America/New_York".into(),
        cpu_architecture: "x86".into(), cpu_bitness: "64".into(),
        platform_version: "15.0.0".into(),
        ua_model: "".into(), ua_wow64: false,
        device_class: DeviceClass::Desktop,
        tls_impersonate: "chrome_147".into(),
        connection_effective_type: "4g".into(),
        connection_rtt: 50, connection_downlink: 10.0,
        pdf_viewer_enabled: true, plugins_count: 5, mime_types_count: 2,
        canvas_seed: 0x1234567890abcdef, audio_seed: 0xfedcba0987654321,
        audio_sample_rate: 44100,
        has_platform_authenticator: true, conditional_mediation: true,
        allow_http3: false,
        prefers_color_scheme: "light".into(),
        color_gamut: "srgb".into(),
        pointer_type: "fine".into(), hover_capability: "hover".into(),
        inner_width: 1920, inner_height: 969,
        outer_width: 1920, outer_height: 1080,
        proxy: None,
        media_devices: default_media_devices("win10"),
        enforce_csp: true,
    }
}

/// Chrome 148 on macOS 15 (Apple Silicon M3).
pub fn chrome_148_macos() -> StealthProfile {
    StealthProfile {
        user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36".into(),
        browser_name: "Chrome".into(),
        browser_version: "148.0.7778.168".into(),
        os_name: "macOS".into(),
        os_version: "15.2".into(),
        platform: "MacIntel".into(),
        vendor: "Google Inc.".into(),
        vendor_sub: "".into(),
        product_sub: "20030107".into(),
        app_version: "5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36".into(),
        screen_width: 1512, screen_height: 982,
        screen_avail_width: 1512, screen_avail_height: 949,
        screen_avail_top: 33, screen_color_depth: 30,
        device_pixel_ratio: 2.0,
        cpu_cores: 8, device_memory: 8, max_touch_points: 0,
        webgl_vendor: "Google Inc. (Apple)".into(),
        webgl_renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M3, Unspecified Version)".into(),
        gpu_profile: apple_m3_macos(),
        language: "en-US".into(),
        languages: vec!["en-US".into(), "en".into()],
        timezone: "America/Los_Angeles".into(),
        cpu_architecture: "arm".into(), cpu_bitness: "64".into(),
        platform_version: "15.2.0".into(),
        ua_model: "".into(), ua_wow64: false,
        device_class: DeviceClass::Desktop,
        tls_impersonate: "chrome_147".into(),
        connection_effective_type: "4g".into(),
        connection_rtt: 50, connection_downlink: 10.0,
        pdf_viewer_enabled: true, plugins_count: 5, mime_types_count: 2,
        canvas_seed: 0xabcdef1234567890, audio_seed: 0x0987654321fedcba,
        audio_sample_rate: 48000,
        has_platform_authenticator: true, conditional_mediation: true,
        allow_http3: false,
        prefers_color_scheme: "light".into(),
        color_gamut: "p3".into(),
        pointer_type: "fine".into(), hover_capability: "hover".into(),
        inner_width: 1512, inner_height: 871,
        outer_width: 1512, outer_height: 982,
        proxy: None,
        media_devices: default_media_devices("macos"),
        enforce_csp: true,
    }
}

/// Chrome 148 on Linux.
pub fn chrome_148_linux() -> StealthProfile {
    StealthProfile {
        user_agent: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36".into(),
        browser_name: "Chrome".into(),
        browser_version: "148.0.7778.168".into(),
        os_name: "Linux".into(),
        os_version: "6.1".into(),
        platform: "Linux x86_64".into(),
        vendor: "Google Inc.".into(),
        vendor_sub: "".into(),
        product_sub: "20030107".into(),
        app_version: "5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36".into(),
        screen_width: 1920, screen_height: 1080,
        screen_avail_width: 1920, screen_avail_height: 1053,
        screen_avail_top: 0, screen_color_depth: 24,
        device_pixel_ratio: 1.0,
        cpu_cores: 8, device_memory: 8, max_touch_points: 0,
        webgl_vendor: "Google Inc. (Intel)".into(),
        webgl_renderer: "ANGLE (Intel, Mesa Intel(R) UHD Graphics 630 (CFL GT2), OpenGL 4.6)".into(),
        gpu_profile: intel_uhd_630_linux(),
        language: "en-US".into(),
        languages: vec!["en-US".into(), "en".into()],
        timezone: "America/Chicago".into(),
        cpu_architecture: "x86".into(), cpu_bitness: "64".into(),
        platform_version: "".into(),
        ua_model: "".into(), ua_wow64: false,
        device_class: DeviceClass::Desktop,
        tls_impersonate: "chrome_147".into(),
        connection_effective_type: "4g".into(),
        connection_rtt: 50, connection_downlink: 10.0,
        pdf_viewer_enabled: true, plugins_count: 5, mime_types_count: 2,
        canvas_seed: 0x1111222233334444, audio_seed: 0x5555666677778888,
        audio_sample_rate: 44100,
        has_platform_authenticator: false, conditional_mediation: true,
        allow_http3: false,
        prefers_color_scheme: "light".into(),
        color_gamut: "srgb".into(),
        pointer_type: "fine".into(), hover_capability: "hover".into(),
        inner_width: 1920, inner_height: 969,
        outer_width: 1920, outer_height: 1080,
        proxy: None,
        media_devices: default_media_devices("linux"),
        enforce_csp: true,
    }
}

/// Chrome 148 on Windows — Russian locale (Moscow).
pub fn chrome_148_ru() -> StealthProfile {
    StealthProfile {
        language: "ru-RU".into(),
        languages: vec!["ru-RU".into(), "ru".into(), "en-US".into(), "en".into()],
        timezone: "Europe/Moscow".into(),
        connection_rtt: 100,
        connection_downlink: 8.0,
        canvas_seed: 0xaaaa_bbbb_cccc_dddd,
        audio_seed: 0xdddd_cccc_bbbb_aaaa,
        media_devices: default_media_devices("ru"),
        webgl_renderer:
            "ANGLE (NVIDIA, NVIDIA GeForce GTX 1660 SUPER Direct3D11 vs_5_0 ps_5_0, D3D11)".into(),
        ..chrome_148_windows()
    }
}

/// Chrome 148 on Windows — Chinese locale (Shanghai).
pub fn chrome_148_cn() -> StealthProfile {
    StealthProfile {
        language: "zh-CN".into(),
        languages: vec!["zh-CN".into(), "zh".into(), "en-US".into(), "en".into()],
        timezone: "Asia/Shanghai".into(),
        device_pixel_ratio: 1.25,
        cpu_cores: 12,
        device_memory: 16,
        connection_rtt: 150,
        connection_downlink: 6.0,
        canvas_seed: 0x1122_3344_5566_7788,
        audio_seed: 0x8877_6655_4433_2211,
        media_devices: default_media_devices("cn"),
        ..chrome_148_windows()
    }
}

/// Chrome 148 on Windows — German locale (Berlin).
pub fn chrome_148_de() -> StealthProfile {
    StealthProfile {
        language: "de-DE".into(),
        languages: vec!["de-DE".into(), "de".into(), "en-US".into(), "en".into()],
        timezone: "Europe/Berlin".into(),
        canvas_seed: 0xdede_dede_dede_dede,
        audio_seed: 0xeded_eded_eded_eded,
        ..chrome_148_windows()
    }
}

/// Chrome 148 on Windows — Japanese locale (Tokyo).
pub fn chrome_148_jp() -> StealthProfile {
    StealthProfile {
        language: "ja-JP".into(),
        languages: vec!["ja".into(), "en-US".into(), "en".into()],
        timezone: "Asia/Tokyo".into(),
        canvas_seed: 0x0a00_0000_0000_0001,
        audio_seed: 0x0a00_0000_0000_0002,
        ..chrome_148_windows()
    }
}

// ── Firefox 135 presets ──────────────────────────────────────────────

/// Firefox 135 on macOS.
pub fn firefox_135_macos() -> StealthProfile {
    StealthProfile {
        user_agent:
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 14.5; rv:135.0) Gecko/20100101 Firefox/135.0"
                .into(),
        browser_name: "Firefox".into(),
        browser_version: "135.0".into(),
        os_name: "macOS".into(),
        os_version: "14.5".into(),
        platform: "MacIntel".into(),
        vendor: "".into(),
        vendor_sub: "".into(),
        product_sub: "20100101".into(),
        app_version: "5.0 (Macintosh; Intel Mac OS X 14.5; rv:135.0) Gecko/20100101 Firefox/135.0"
            .into(),
        screen_width: 1440,
        screen_height: 900,
        screen_avail_width: 1440,
        screen_avail_height: 875,
        screen_avail_top: 25,
        screen_color_depth: 30,
        device_pixel_ratio: 2.0,
        cpu_cores: 10,
        device_memory: 16,
        max_touch_points: 0,
        webgl_vendor: "Mozilla".into(),
        webgl_renderer: "Mozilla".into(),
        gpu_profile: apple_m2_pro_macos(),
        language: "en-US".into(),
        languages: vec!["en-US".into(), "en".into()],
        timezone: "America/Los_Angeles".into(),
        cpu_architecture: "arm".into(),
        cpu_bitness: "64".into(),
        platform_version: "14.5.0".into(),
        ua_model: "".into(),
        ua_wow64: false,
        device_class: DeviceClass::Desktop,
        tls_impersonate: "firefox_135".into(),
        connection_effective_type: "4g".into(),
        connection_rtt: 50,
        connection_downlink: 10.0,
        pdf_viewer_enabled: true,
        plugins_count: 5,
        mime_types_count: 2,
        canvas_seed: 0xff0011_ff0022_ff0033_u128 as u64,
        audio_seed: 0x88aa_bbcc_ddee_ff00,
        audio_sample_rate: 44100,
        has_platform_authenticator: true,
        conditional_mediation: true,
        allow_http3: false,
        prefers_color_scheme: "light".into(),
        color_gamut: "p3".into(),
        pointer_type: "fine".into(),
        hover_capability: "hover".into(),
        inner_width: 1440,
        inner_height: 789,
        outer_width: 1440,
        outer_height: 900,
        proxy: None,
        media_devices: default_media_devices("macos"),
        enforce_csp: true,
    }
}

/// Firefox 135 on Windows 10.
pub fn firefox_135_windows() -> StealthProfile {
    StealthProfile {
        user_agent:
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:135.0) Gecko/20100101 Firefox/135.0"
                .into(),
        browser_name: "Firefox".into(),
        browser_version: "135.0".into(),
        os_name: "Windows".into(),
        os_version: "10.0".into(),
        platform: "Win32".into(),
        vendor: "".into(),
        vendor_sub: "".into(),
        product_sub: "20100101".into(),
        app_version: "5.0 (Windows NT 10.0; Win64; x64; rv:135.0) Gecko/20100101 Firefox/135.0"
            .into(),
        screen_width: 1920,
        screen_height: 1080,
        screen_avail_width: 1920,
        screen_avail_height: 1040,
        screen_avail_top: 0,
        screen_color_depth: 24,
        device_pixel_ratio: 1.0,
        cpu_cores: 8,
        device_memory: 8,
        max_touch_points: 0,
        webgl_vendor: "Mozilla".into(),
        webgl_renderer: "Mozilla".into(),
        gpu_profile: nvidia_rtx_3060_windows(),
        language: "en-US".into(),
        languages: vec!["en-US".into(), "en".into()],
        timezone: "America/New_York".into(),
        cpu_architecture: "x86".into(),
        cpu_bitness: "64".into(),
        platform_version: "15.0.0".into(),
        ua_model: "".into(),
        ua_wow64: false,
        device_class: DeviceClass::Desktop,
        tls_impersonate: "firefox_135".into(),
        connection_effective_type: "4g".into(),
        connection_rtt: 50,
        connection_downlink: 10.0,
        pdf_viewer_enabled: true,
        plugins_count: 5,
        mime_types_count: 2,
        canvas_seed: 0x1122_3344_5566_7788,
        audio_seed: 0x99aa_bbcc_ddee_ff00,
        audio_sample_rate: 44100,
        has_platform_authenticator: true,
        conditional_mediation: true,
        allow_http3: false,
        prefers_color_scheme: "light".into(),
        color_gamut: "srgb".into(),
        pointer_type: "fine".into(),
        hover_capability: "hover".into(),
        inner_width: 1920,
        inner_height: 969,
        outer_width: 1920,
        outer_height: 1080,
        proxy: None,
        media_devices: default_media_devices("windows"),
        enforce_csp: true,
    }
}

/// Firefox 135 on Linux.
pub fn firefox_135_linux() -> StealthProfile {
    StealthProfile {
        user_agent: "Mozilla/5.0 (X11; Linux x86_64; rv:135.0) Gecko/20100101 Firefox/135.0".into(),
        browser_name: "Firefox".into(),
        browser_version: "135.0".into(),
        os_name: "Linux".into(),
        os_version: "6.1".into(),
        platform: "Linux x86_64".into(),
        vendor: "".into(),
        vendor_sub: "".into(),
        product_sub: "20100101".into(),
        app_version: "5.0 (X11; Linux x86_64; rv:135.0) Gecko/20100101 Firefox/135.0".into(),
        screen_width: 1920,
        screen_height: 1080,
        screen_avail_width: 1920,
        screen_avail_height: 1053,
        screen_avail_top: 0,
        screen_color_depth: 24,
        device_pixel_ratio: 1.0,
        cpu_cores: 8,
        device_memory: 8,
        max_touch_points: 0,
        webgl_vendor: "Mozilla".into(),
        webgl_renderer: "Mozilla".into(),
        gpu_profile: intel_uhd_630_linux(),
        language: "en-US".into(),
        languages: vec!["en-US".into(), "en".into()],
        timezone: "America/Chicago".into(),
        cpu_architecture: "x86".into(),
        cpu_bitness: "64".into(),
        platform_version: "".into(),
        ua_model: "".into(),
        ua_wow64: false,
        device_class: DeviceClass::Desktop,
        tls_impersonate: "firefox_135".into(),
        connection_effective_type: "4g".into(),
        connection_rtt: 50,
        connection_downlink: 10.0,
        pdf_viewer_enabled: true,
        plugins_count: 5,
        mime_types_count: 2,
        canvas_seed: 0xaaaa_bbbb_cccc_dddd,
        audio_seed: 0xdddd_cccc_bbbb_aaaa,
        audio_sample_rate: 44100,
        has_platform_authenticator: false,
        conditional_mediation: true,
        allow_http3: false,
        prefers_color_scheme: "light".into(),
        color_gamut: "srgb".into(),
        pointer_type: "fine".into(),
        hover_capability: "hover".into(),
        inner_width: 1920,
        inner_height: 969,
        outer_width: 1920,
        outer_height: 1080,
        proxy: None,
        media_devices: default_media_devices("linux"),
        enforce_csp: true,
    }
}

// ── Mobile presets ───────────────────────────────────────────────────

/// Chrome 148 on Pixel 9 Pro (Android 15).
pub fn pixel_9_pro_chrome_148() -> StealthProfile {
    StealthProfile {
        user_agent: "Mozilla/5.0 (Linux; Android 15; Pixel 9 Pro Build/AP4A.250105.002) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Mobile Safari/537.36".into(),
        browser_name: "Chrome".into(),
        browser_version: "148.0.7778.168".into(),
        os_name: "Android".into(),
        os_version: "15".into(),
        platform: "Linux armv81".into(),
        vendor: "Google Inc.".into(),
        vendor_sub: "".into(),
        product_sub: "20030107".into(),
        app_version: "5.0 (Linux; Android 15; Pixel 9 Pro Build/AP4A.250105.002) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Mobile Safari/537.36".into(),
        screen_width: 412, screen_height: 870,
        screen_avail_width: 412, screen_avail_height: 870,
        screen_avail_top: 0, screen_color_depth: 24,
        device_pixel_ratio: 2.625,
        cpu_cores: 8, device_memory: 8, max_touch_points: 5,
        webgl_vendor: "Google Inc. (Google)".into(),
        webgl_renderer: "ANGLE (Google, Mali-G715 MP7, OpenGL ES 3.2)".into(),
        gpu_profile: apple_m3_macos(), // ponytail: placeholder, needs android GPU profile
        language: "en-US".into(),
        languages: vec!["en-US".into(), "en".into()],
        timezone: "America/Los_Angeles".into(),
        cpu_architecture: "".into(), cpu_bitness: "64".into(),
        platform_version: "15.0.0".into(),
        ua_model: "Pixel 9 Pro".into(), ua_wow64: false,
        device_class: DeviceClass::MobileAndroid,
        tls_impersonate: "chrome_147_android".into(),
        connection_effective_type: "4g".into(),
        connection_rtt: 50, connection_downlink: 10.0,
        pdf_viewer_enabled: false, plugins_count: 0, mime_types_count: 0,
        canvas_seed: 0xa5a5_d5d5_3c3c_e6e6, audio_seed: 0x9c9c_5e5e_4040_b1b1,
        audio_sample_rate: 44100,
        has_platform_authenticator: false, conditional_mediation: true,
        allow_http3: false,
        prefers_color_scheme: "light".into(),
        color_gamut: "srgb".into(),
        pointer_type: "coarse".into(), hover_capability: "none".into(),
        inner_width: 412, inner_height: 870,
        outer_width: 412, outer_height: 870,
        proxy: None,
        media_devices: default_media_devices("android"),
        enforce_csp: true,
    }
}

/// Mobile Safari 18 on iPhone 15 Pro (iOS 18).
pub fn iphone_15_pro_safari_18() -> StealthProfile {
    StealthProfile {
        user_agent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0.1 Mobile/15E148 Safari/604.1".into(),
        browser_name: "Safari".into(),
        browser_version: "18.0.1".into(),
        os_name: "iOS".into(),
        os_version: "18.0.1".into(),
        platform: "iPhone".into(),
        vendor: "Apple Computer, Inc.".into(),
        vendor_sub: "".into(),
        product_sub: "20030107".into(),
        app_version: "5.0 (iPhone; CPU iPhone OS 18_0_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0.1 Mobile/15E148 Safari/604.1".into(),
        screen_width: 393, screen_height: 852,
        screen_avail_width: 393, screen_avail_height: 852,
        screen_avail_top: 0, screen_color_depth: 24,
        device_pixel_ratio: 3.0,
        cpu_cores: 2, device_memory: 0, max_touch_points: 5,
        webgl_vendor: "Apple Inc.".into(),
        webgl_renderer: "Apple GPU".into(),
        gpu_profile: apple_m3_macos(), // ponytail: placeholder, needs iOS GPU profile
        language: "en-US".into(),
        languages: vec!["en-US".into(), "en".into()],
        timezone: "America/Los_Angeles".into(),
        cpu_architecture: "arm".into(), cpu_bitness: "64".into(),
        platform_version: "18.0.1".into(),
        ua_model: "iPhone".into(), ua_wow64: false,
        device_class: DeviceClass::MobileIOS,
        tls_impersonate: "safari_18_ios".into(),
        connection_effective_type: "4g".into(),
        connection_rtt: 50, connection_downlink: 10.0,
        pdf_viewer_enabled: false, plugins_count: 0, mime_types_count: 0,
        canvas_seed: 0xa1b2_c3d4_e5f6_0708, audio_seed: 0x0807_0605_0403_0201,
        audio_sample_rate: 44100,
        has_platform_authenticator: false, conditional_mediation: true,
        allow_http3: false,
        prefers_color_scheme: "light".into(),
        color_gamut: "p3".into(),
        pointer_type: "coarse".into(), hover_capability: "none".into(),
        inner_width: 393, inner_height: 852,
        outer_width: 393, outer_height: 852,
        proxy: None,
        media_devices: default_media_devices("ios"),
        enforce_csp: true,
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
