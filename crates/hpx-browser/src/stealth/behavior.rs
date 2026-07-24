//! Behavioral emulation — mouse trajectory (sigma-lognormal), keyboard
//! dynamics, and scroll simulation.

use serde::{Deserialize, Serialize};

// ── Behavioral enums and profile ─────────────────────────────────────

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
