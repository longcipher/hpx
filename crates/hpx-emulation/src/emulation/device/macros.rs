macro_rules! settings_order {
    () => {
        SettingsOrder::builder()
            .extend([
                SettingId::HeaderTableSize,
                SettingId::EnablePush,
                SettingId::MaxConcurrentStreams,
                SettingId::InitialWindowSize,
                SettingId::MaxFrameSize,
                SettingId::MaxHeaderListSize,
                SettingId::EnableConnectProtocol,
                SettingId::NoRfc7540Priorities,
            ])
            .build()
    };
}

macro_rules! pseudo_order {
    () => {
        PseudoOrder::builder()
            .extend([
                PseudoId::Method,
                PseudoId::Authority,
                PseudoId::Scheme,
                PseudoId::Path,
            ])
            .build()
    };
}

macro_rules! header_chrome_sec_ch_ua {
    ($headers:expr, $ua:expr, $platform:expr, $is_mobile:expr) => {
        let mobile = if $is_mobile { "?1" } else { "?0" };
        $headers.insert("sec-ch-ua", HeaderValue::from_static($ua));
        $headers.insert("sec-ch-ua-mobile", HeaderValue::from_static(mobile));
        $headers.insert("sec-ch-ua-platform", HeaderValue::from_static($platform));
    };
}

macro_rules! header_sec_fetch {
    ($headers:expr) => {
        $headers.insert("sec-fetch-dest", HeaderValue::from_static("document"));
        $headers.insert("sec-fetch-mode", HeaderValue::from_static("navigate"));
        $headers.insert("sec-fetch-site", HeaderValue::from_static("none"));
    };
}

macro_rules! header_chrome_ua {
    ($headers:expr, $ua:expr) => {
        $headers.insert(USER_AGENT, HeaderValue::from_static($ua));
    };
}

macro_rules! header_chrome_accept {
    ($headers:expr) => {
        $headers.insert(ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9"));
        #[cfg(feature = "emulation-compression")]
        $headers.insert(
            ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br"),
        );
        $headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
    };
    (zstd, $headers:expr) => {
        $headers.insert(ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9"));
        #[cfg(feature = "emulation-compression")]
        $headers.insert(
            ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br, zstd"),
        );
        $headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
    }
}

macro_rules! header_firefox_accept {
    ($headers:expr) => {
        $headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
        );
        #[cfg(feature = "emulation-compression")]
        $headers.insert(
            ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br"),
        );
        $headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.5"));
    };
    (zstd, $headers:expr) => {
        $headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
        );
        #[cfg(feature = "emulation-compression")]
        $headers.insert(
            ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br, zstd"),
        );
        $headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.5"));
    };
}

macro_rules! header_firefox_ua {
    ($headers:expr, $ua:expr) => {
        $headers.insert(
            HeaderName::from_static("te"),
            HeaderValue::from_static("trailers"),
        );
        $headers.insert(USER_AGENT, HeaderValue::from_static($ua));
    };
}

macro_rules! join {
    ($sep:expr, $first:expr $(, $rest:expr)*) => {
        concat!($first $(, $sep, $rest)*)
    };
}

macro_rules! mod_generator {
    (
        $mod_name:ident,
        $tls_options:expr,
        $http2_options:expr,
        $header_initializer:ident,
        [($default_os:ident, $default_sec_ch_ua:tt, $default_ua:tt) $(, ($other_os:ident, $other_sec_ch_ua:tt, $other_ua:tt))*]
    ) => {
        pub(crate) mod $mod_name {
            use super::*;

            pub(crate) fn emulation(option: EmulationOption) -> Emulation {
                let default_headers = if !option.skip_headers {
                    let default_headers = match option.emulation_os {
                        $(
                            EmulationOS::$other_os => $header_initializer(
                                $other_sec_ch_ua,
                                $other_ua,
                                option.emulation_os,
                            ),
                        )*
                        _ => $header_initializer(
                            $default_sec_ch_ua,
                            $default_ua,
                            EmulationOS::$default_os,
                        ),
                    };
                    Some(default_headers)
                } else {
                    None
                };

                build_emulation(option, default_headers)
            }

            pub(crate) fn build_emulation(
                option: EmulationOption,
                default_headers: Option<HeaderMap>
            ) -> Emulation {
                let mut builder = Emulation::builder().tls_options($tls_options);

                if !option.skip_http2 {
                    builder = builder.http2_options($http2_options);
                }

                if let Some(headers) = default_headers {
                    builder = builder.headers(headers);
                }

                builder.build()
            }
        }
    };
    // Variant with http3_options
    (
        $mod_name:ident,
        $tls_options:expr,
        $http2_options:expr,
        $http3_options:expr,
        $header_initializer:ident,
        [($default_os:ident, $default_sec_ch_ua:tt, $default_ua:tt) $(, ($other_os:ident, $other_sec_ch_ua:tt, $other_ua:tt))*]
    ) => {
        pub(crate) mod $mod_name {
            use super::*;

            pub(crate) fn emulation(option: EmulationOption) -> Emulation {
                let default_headers = if !option.skip_headers {
                    let default_headers = match option.emulation_os {
                        $(
                            EmulationOS::$other_os => $header_initializer(
                                $other_sec_ch_ua,
                                $other_ua,
                                option.emulation_os,
                            ),
                        )*
                        _ => $header_initializer(
                            $default_sec_ch_ua,
                            $default_ua,
                            EmulationOS::$default_os,
                        ),
                    };
                    Some(default_headers)
                } else {
                    None
                };

                build_emulation(option, default_headers)
            }

            pub(crate) fn build_emulation(
                option: EmulationOption,
                default_headers: Option<HeaderMap>
            ) -> Emulation {
                let mut builder = Emulation::builder()
                    .tls_options($tls_options);
                #[cfg(feature = "http3")]
                {
                    builder = builder.http3_options($http3_options);
                }

                if !option.skip_http2 {
                    builder = builder.http2_options($http2_options);
                }

                if let Some(headers) = default_headers {
                    builder = builder.headers(headers);
                }

                builder.build()
            }
        }
    };
    (
        $mod_name:ident,
        $build_emulation:expr,
        $header_initializer:ident,
        [($default_os:ident, $default_sec_ch_ua:tt, $default_ua:tt) $(, ($other_os:ident, $other_sec_ch_ua:tt, $other_ua:tt))*]
    ) => {
        pub(crate) mod $mod_name {
            use super::*;

            pub(crate) fn emulation(option: EmulationOption) -> Emulation {
                let default_headers = if !option.skip_headers {
                    let default_headers = match option.emulation_os {
                        $(
                            EmulationOS::$other_os => $header_initializer(
                                $other_sec_ch_ua,
                                $other_ua,
                                option.emulation_os,
                            ),
                        )*
                        _ => $header_initializer(
                            $default_sec_ch_ua,
                            $default_ua,
                            EmulationOS::$default_os,
                        ),
                    };
                    Some(default_headers)
                } else {
                    None
                };

                $build_emulation(option, default_headers)
            }
        }
    };
}

/// Produces [`Http3Options`] for a given browser profile.
///
/// This macro is used internally by the `mod_generator!` macro to wire
/// HTTP/3 options into emulation profiles. It is also exported for
/// advanced users who want to construct `Http3Options` with browser-specific
/// defaults without going through the full emulation builder.
///
/// # Examples
///
/// ```
/// use hpx::http3::Http3Options;
/// use hpx_emulation::http3_options;
///
/// let opts = http3_options!(Chrome143);
/// assert_eq!(opts, Http3Options::default());
/// ```
#[cfg(feature = "http3")]
#[macro_export]
macro_rules! http3_options {
    (Chrome143) => {
        hpx::http3::Http3Options::default()
    };
    (Firefox88) => {
        hpx::http3::Http3Options::customize(|opts| {
            opts.max_idle_timeout = Some(std::time::Duration::from_secs(30));
            opts.max_concurrent_bidi_streams = Some(100);
            opts.max_concurrent_uni_streams = Some(100);
            opts.congestion_bbr = false;
            opts.initial_max_data = Some(2 * 1024 * 1024);
            opts.initial_max_stream_data_bidi_local = Some(1024 * 1024);
            opts.initial_max_stream_data_bidi_remote = Some(1024 * 1024);
            opts.initial_max_stream_data_uni = Some(1024 * 1024);
            opts.qpack_max_table_capacity = Some(65536);
            opts.qpack_blocked_streams = Some(20);
            opts.initial_packet_padding = Some(1232);
        })
    };
    (Safari14) => {
        hpx::http3::Http3Options::customize(|opts| {
            opts.max_idle_timeout = Some(std::time::Duration::from_secs(30));
            opts.max_concurrent_bidi_streams = Some(100);
            opts.max_concurrent_uni_streams = Some(100);
            opts.congestion_bbr = false;
            opts.initial_max_data = Some(1024 * 1024);
            opts.initial_max_stream_data_bidi_local = Some(1024 * 1024);
            opts.initial_max_stream_data_bidi_remote = Some(1024 * 1024);
            opts.initial_max_stream_data_uni = Some(1024 * 1024);
            opts.qpack_max_table_capacity = Some(0);
            opts.qpack_blocked_streams = Some(0);
            opts.initial_packet_padding = None;
        })
    };
    (Chrome96) => {
        hpx::http3::Http3Options::customize(|opts| {
            opts.max_idle_timeout = Some(std::time::Duration::from_secs(30));
            opts.max_concurrent_bidi_streams = Some(100);
            opts.stream_receive_window = Some(8 * 1024 * 1024);
            opts.qpack_max_table_capacity = Some(4096);
            opts.qpack_blocked_streams = Some(100);
            opts.enable_0rtt = false;
            opts.initial_packet_padding = Some(1200);
        })
    };
    (Edge96) => {
        $crate::http3_options!(Chrome96)
    };
}

#[cfg(all(test, feature = "http3"))]
mod http3_options_tests {
    #[test]
    fn chrome_143_http3_options_matches_default() {
        let opts = crate::http3_options!(Chrome143);
        let default = hpx::http3::Http3Options::default();
        assert_eq!(opts, default);
    }

    #[test]
    fn firefox_88_http3_options_matches_real_firefox() {
        let opts = crate::http3_options!(Firefox88);

        // QUIC transport parameters (neqo ConnectionParameters::default())
        assert_eq!(
            opts.max_idle_timeout,
            Some(std::time::Duration::from_secs(30))
        );
        assert_eq!(opts.max_concurrent_bidi_streams, Some(100));
        assert_eq!(opts.max_concurrent_uni_streams, Some(100));
        assert!(!opts.congestion_bbr); // Cubic, not BBR
        assert_eq!(opts.initial_max_data, Some(2 * 1024 * 1024)); // 2 MiB
        assert_eq!(opts.initial_max_stream_data_bidi_local, Some(1024 * 1024)); // 1 MiB
        assert_eq!(opts.initial_max_stream_data_bidi_remote, Some(1024 * 1024)); // 1 MiB
        assert_eq!(opts.initial_max_stream_data_uni, Some(1024 * 1024)); // 1 MiB

        // QPACK parameters (neqo-qpack Settings::default())
        assert_eq!(opts.qpack_max_table_capacity, Some(65536));
        assert_eq!(opts.qpack_blocked_streams, Some(20));

        // Firefox pads Initial packet to 1232 bytes
        assert_eq!(opts.initial_packet_padding, Some(1232));
    }

    #[test]
    fn safari_14_http3_options_matches_real_safari() {
        let opts = crate::http3_options!(Safari14);

        // QUIC transport parameters (WebKit Network.framework defaults)
        assert_eq!(
            opts.max_idle_timeout,
            Some(std::time::Duration::from_secs(30))
        );
        assert_eq!(opts.max_concurrent_bidi_streams, Some(100));
        assert_eq!(opts.max_concurrent_uni_streams, Some(100));
        assert!(!opts.congestion_bbr); // Cubic, not BBR
        assert_eq!(opts.initial_max_data, Some(1024 * 1024)); // 1 MiB
        assert_eq!(opts.initial_max_stream_data_bidi_local, Some(1024 * 1024)); // 1 MiB
        assert_eq!(opts.initial_max_stream_data_bidi_remote, Some(1024 * 1024)); // 1 MiB
        assert_eq!(opts.initial_max_stream_data_uni, Some(1024 * 1024)); // 1 MiB

        // QPACK parameters (WebKit doesn't use QPACK dynamic table)
        assert_eq!(opts.qpack_max_table_capacity, Some(0));
        assert_eq!(opts.qpack_blocked_streams, Some(0));

        // Safari doesn't pad Initial packet
        assert_eq!(opts.initial_packet_padding, None);
    }

    #[test]
    fn chrome_96_http3_options_matches_real_chrome() {
        let opts = crate::http3_options!(Chrome96);

        // QUIC transport parameters (Chrome 96 baseline)
        assert_eq!(
            opts.max_idle_timeout,
            Some(std::time::Duration::from_secs(30))
        );
        assert_eq!(opts.max_concurrent_bidi_streams, Some(100));
        assert_eq!(opts.stream_receive_window, Some(8 * 1024 * 1024)); // 8 MiB
        assert_eq!(opts.qpack_max_table_capacity, Some(4096));
        assert_eq!(opts.qpack_blocked_streams, Some(100));
        assert!(!opts.enable_0rtt); // 0-RTT disabled in Chrome 96
        assert_eq!(opts.initial_packet_padding, Some(1200));
    }

    #[test]
    fn edge_96_http3_options_matches_real_edge() {
        let opts = crate::http3_options!(Edge96);

        // Edge 96 is Chromium-based, same HTTP/3 options as Chrome 96
        assert_eq!(
            opts.max_idle_timeout,
            Some(std::time::Duration::from_secs(30))
        );
        assert_eq!(opts.max_concurrent_bidi_streams, Some(100));
        assert_eq!(opts.stream_receive_window, Some(8 * 1024 * 1024)); // 8 MiB
        assert_eq!(opts.qpack_max_table_capacity, Some(4096));
        assert_eq!(opts.qpack_blocked_streams, Some(100));
        assert!(!opts.enable_0rtt); // 0-RTT disabled
        assert_eq!(opts.initial_packet_padding, Some(1200));
    }

    #[test]
    fn quic_transport_params_match_browser_fingerprint() {
        let chrome143 = crate::http3_options!(Chrome143);
        let chrome96 = crate::http3_options!(Chrome96);
        let firefox88 = crate::http3_options!(Firefox88);
        let safari14 = crate::http3_options!(Safari14);
        let edge96 = crate::http3_options!(Edge96);

        // Chrome 143 vs Chrome 96: different enable_0rtt and initial_packet_padding
        assert_ne!(chrome143.enable_0rtt, chrome96.enable_0rtt);
        assert_ne!(
            chrome143.initial_packet_padding,
            chrome96.initial_packet_padding
        );

        // Chrome 143 vs Firefox 88: different initial_max_data and qpack params
        assert_ne!(chrome143.initial_max_data, firefox88.initial_max_data);
        assert_ne!(
            chrome143.qpack_max_table_capacity,
            firefox88.qpack_max_table_capacity
        );
        assert_ne!(
            chrome143.qpack_blocked_streams,
            firefox88.qpack_blocked_streams
        );

        // Chrome 143 vs Safari 14: different initial_max_data and qpack params
        assert_ne!(chrome143.initial_max_data, safari14.initial_max_data);
        assert_ne!(
            chrome143.qpack_max_table_capacity,
            safari14.qpack_max_table_capacity
        );
        assert_ne!(
            chrome143.qpack_blocked_streams,
            safari14.qpack_blocked_streams
        );

        // Firefox 88 vs Safari 14: different initial_max_data, qpack params, and padding
        assert_ne!(firefox88.initial_max_data, safari14.initial_max_data);
        assert_ne!(
            firefox88.qpack_max_table_capacity,
            safari14.qpack_max_table_capacity
        );
        assert_ne!(
            firefox88.initial_packet_padding,
            safari14.initial_packet_padding
        );

        // Edge 96 vs Chrome 143: different enable_0rtt
        assert_ne!(edge96.enable_0rtt, chrome143.enable_0rtt);

        // Edge 96 vs Firefox 88: different qpack params
        assert_ne!(
            edge96.qpack_max_table_capacity,
            firefox88.qpack_max_table_capacity
        );
        assert_ne!(
            edge96.qpack_blocked_streams,
            firefox88.qpack_blocked_streams
        );
    }

    #[test]
    fn quic_initial_packet_padding_matches_browser() {
        let chrome143 = crate::http3_options!(Chrome143);
        let chrome96 = crate::http3_options!(Chrome96);
        let firefox88 = crate::http3_options!(Firefox88);
        let safari14 = crate::http3_options!(Safari14);
        let edge96 = crate::http3_options!(Edge96);

        // Chrome 143: no explicit padding (None)
        assert_eq!(chrome143.initial_packet_padding, None);

        // Chrome 96: pads to 1200 bytes
        assert_eq!(chrome96.initial_packet_padding, Some(1200));

        // Firefox 88: pads to 1232 bytes
        assert_eq!(firefox88.initial_packet_padding, Some(1232));

        // Safari 14: no padding
        assert_eq!(safari14.initial_packet_padding, None);

        // Edge 96: same as Chrome 96 (1200 bytes)
        assert_eq!(edge96.initial_packet_padding, Some(1200));
    }
}
