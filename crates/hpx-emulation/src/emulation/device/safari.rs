#[macro_use]
mod http2;
#[macro_use]
mod tls;
mod header;

use header::*;
use tls::*;

use super::*;

macro_rules! mod_generator {
    ($mod_name:ident, $tls_options:expr, $http2_options:expr, $header_initializer:ident, $ua:expr) => {
        pub(crate) mod $mod_name {
            use super::*;

            pub fn emulation(option: EmulationOption) -> Emulation {
                let default_headers = if !option.skip_headers {
                    Some($header_initializer($ua))
                } else {
                    None
                };

                build_emulation(option, default_headers)
            }

            pub fn build_emulation(
                option: EmulationOption,
                default_headers: Option<HeaderMap>,
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
    ($mod_name:ident, $tls_options:expr, $http2_options:expr, $http3_options:expr, $header_initializer:ident, $ua:expr) => {
        pub(crate) mod $mod_name {
            use super::*;

            pub fn emulation(option: EmulationOption) -> Emulation {
                let default_headers = if !option.skip_headers {
                    Some($header_initializer($ua))
                } else {
                    None
                };

                build_emulation(option, default_headers)
            }

            pub fn build_emulation(
                option: EmulationOption,
                default_headers: Option<HeaderMap>,
            ) -> Emulation {
                let mut builder = Emulation::builder().tls_options($tls_options);
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

    ($mod_name:ident, $build_emulation:expr, $header_initializer:ident, $ua:expr) => {
        pub(crate) mod $mod_name {
            use super::*;

            pub fn emulation(option: EmulationOption) -> Emulation {
                let default_headers = if !option.skip_headers {
                    Some($header_initializer($ua))
                } else {
                    None
                };

                $build_emulation(option, default_headers)
            }
        }
    };
}

mod_generator!(
    safari14,
    tls_options!(1, CIPHER_LIST_1),
    http2_options!(1),
    hpx::http3::Http3Options::customize(|opts| {
        opts.max_idle_timeout = Some(std::time::Duration::from_secs(30));
        opts.max_concurrent_bidi_streams = Some(1000);
        opts.stream_receive_window = Some(4 * 1024 * 1024);
        opts.qpack_max_table_capacity = Some(0);
        opts.enable_0rtt = false;
    }),
    header_initializer_for_14,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/14.1.2 Safari/605.1.15"
);

mod_generator!(
    safari15_3,
    tls_options!(1, CIPHER_LIST_1),
    http2_options!(4),
    header_initializer_for_15,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.3 Safari/605.1.15"
);

mod_generator!(
    safari15_5,
    safari15_3::build_emulation,
    header_initializer_for_15,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.5 Safari/605.1.15"
);

mod_generator!(
    safari15_6_1,
    tls_options!(1, CIPHER_LIST_2),
    http2_options!(4),
    header_initializer_for_15,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.6.1 Safari/605.1.15"
);

mod_generator!(
    safari16,
    safari15_6_1::build_emulation,
    header_initializer_for_16_17,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Safari/605.1.15"
);

mod_generator!(
    safari16_5,
    safari15_6_1::build_emulation,
    header_initializer_for_16_17,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.5 Safari/605.1.15"
);

mod_generator!(
    safari17_4_1,
    safari15_6_1::build_emulation,
    header_initializer_for_16_17,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4.1 Safari/605.1.15"
);

mod_generator!(
    safari_ios_16_5,
    tls_options!(1, CIPHER_LIST_2),
    http2_options!(1),
    header_initializer_for_16_17,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 16_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.5 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari17_0,
    tls_options!(1, CIPHER_LIST_2),
    http2_options!(5),
    header_initializer_for_16_17,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
);

mod_generator!(
    safari17_2_1,
    safari17_0::build_emulation,
    header_initializer_for_16_17,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2.1 Safari/605.1.15"
);

mod_generator!(
    safari17_5,
    safari17_0::build_emulation,
    header_initializer_for_16_17,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15"
);

mod_generator!(
    safari17_6,
    safari17_0::build_emulation,
    header_initializer_for_16_17,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15"
);

mod_generator!(
    safari_ios_17_2,
    tls_options!(1, CIPHER_LIST_2),
    http2_options!(2),
    header_initializer_for_16_17,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari_ios_17_4_1,
    safari_ios_17_2::build_emulation,
    header_initializer_for_16_17,
    "Mozilla/5.0 (iPad; CPU OS 17_4_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4.1 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari18,
    tls_options!(1, CIPHER_LIST_2),
    http2_options!(3),
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15"
);

mod_generator!(
    safari_ipad_18,
    safari18::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari_ios_18_1_1,
    safari18::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1.1 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari18_2,
    tls_options!(2, CIPHER_LIST_2, SIGALGS_LIST_2),
    http2_options!(3),
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Safari/605.1.15"
);

mod_generator!(
    safari18_3,
    safari18_2::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3 Safari/605.1.15"
);

mod_generator!(
    safari18_3_1,
    safari18_2::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3.1 Safari/605.1.15"
);

mod_generator!(
    safari18_5,
    tls_options!(2, CIPHER_LIST_2, SIGALGS_LIST_2),
    http2_options!(6),
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15"
);

mod_generator!(
    safari26,
    tls_options!(3, CIPHER_LIST_3, SIGALGS_LIST_2, CURVES_2),
    http2_options!(6),
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15"
);

mod_generator!(
    safari_ipad_26,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPad; CPU OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari_ios_26,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari26_1,
    safari18_5::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.1 Safari/605.1.15"
);

mod_generator!(
    safari_ios_26_2,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.2 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari26_2,
    safari18_5::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.2 Safari/605.1.15"
);

mod_generator!(
    safari_ipad_26_2,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPad; CPU OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.2 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari19,
    safari18_2::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Safari/605.1.15"
);

mod_generator!(
    safari_ios_19,
    safari18_2::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 19_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari_ipad_19,
    safari18_2::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPad; CPU OS 19_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari20,
    safari18_2::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/20.0 Safari/605.1.15"
);

mod_generator!(
    safari_ios_20,
    safari18_2::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 20_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/20.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari_ipad_20,
    safari18_2::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPad; CPU OS 20_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/20.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari21,
    safari18_2::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/21.0 Safari/605.1.15"
);

mod_generator!(
    safari_ios_21,
    safari18_2::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 21_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/21.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari_ipad_21,
    safari18_2::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPad; CPU OS 21_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/21.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari22,
    safari18_5::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/22.0 Safari/605.1.15"
);

mod_generator!(
    safari_ios_22,
    safari18_5::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 22_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/22.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari_ipad_22,
    safari18_5::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPad; CPU OS 22_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/22.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari23,
    safari18_5::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/23.0 Safari/605.1.15"
);

mod_generator!(
    safari_ios_23,
    safari18_5::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 23_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/23.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari_ipad_23,
    safari18_5::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPad; CPU OS 23_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/23.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari24,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/24.0 Safari/605.1.15"
);

mod_generator!(
    safari_ios_24,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 24_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/24.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari_ipad_24,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPad; CPU OS 24_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/24.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari25,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/25.0 Safari/605.1.15"
);

mod_generator!(
    safari_ios_25,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 25_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/25.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari_ipad_25,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPad; CPU OS 25_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/25.0 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari26_3,
    safari18_5::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.3 Safari/605.1.15"
);

mod_generator!(
    safari_ios_26_3,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_8 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.3 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari_ipad_26_3,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPad; CPU OS 18_8 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.3 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari26_4,
    safari18_5::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.4 Safari/605.1.15"
);

mod_generator!(
    safari_ios_26_4,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_9 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.4 Mobile/15E148 Safari/604.1"
);

mod_generator!(
    safari_ipad_26_4,
    safari26::build_emulation,
    header_initializer_for_18,
    "Mozilla/5.0 (iPad; CPU OS 18_9 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.4 Mobile/15E148 Safari/604.1"
);
