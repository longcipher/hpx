//! Browser preset constructors — complete `StealthProfile` instances for
//! Chrome, Firefox, Safari on various platforms and locales.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use super::{
    DeviceClass, MediaDeviceInfo, StealthProfile,
    gpu_presets::{
        apple_m2_pro_macos, apple_m3_macos, intel_uhd_630_linux, nvidia_rtx_3060_windows,
    },
};

// ── Media device helper ──────────────────────────────────────────────

pub(crate) fn default_media_devices(seed: &str) -> Vec<MediaDeviceInfo> {
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

// ── Chrome 148 presets ───────────────────────────────────────────────

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
