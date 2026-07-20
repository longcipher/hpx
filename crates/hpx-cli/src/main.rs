//! hpx CLI — High Performance HTTP Client command-line interface.
//!
//! This binary provides a simple CLI for making HTTP requests with hpx,
//! supporting HTTP/3, browser emulation, and more.

use clap::Parser;

#[cfg(feature = "boring")]
mod emulation {
    use std::str::FromStr;

    use hpx::EmulationFactory;
    use hpx_util::Emulation;

    /// Wrapper around `hpx_util::Emulation` that implements `FromStr` for CLI
    /// parsing.
    #[derive(Clone, Debug)]
    pub struct BrowserEmulation(Emulation);

    impl FromStr for BrowserEmulation {
        type Err = String;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let emulation = match s {
                // Chrome
                "Chrome96" => Emulation::Chrome96,
                "Chrome100" => Emulation::Chrome100,
                "Chrome101" => Emulation::Chrome101,
                "Chrome104" => Emulation::Chrome104,
                "Chrome105" => Emulation::Chrome105,
                "Chrome106" => Emulation::Chrome106,
                "Chrome107" => Emulation::Chrome107,
                "Chrome108" => Emulation::Chrome108,
                "Chrome109" => Emulation::Chrome109,
                "Chrome110" => Emulation::Chrome110,
                "Chrome114" => Emulation::Chrome114,
                "Chrome116" => Emulation::Chrome116,
                "Chrome117" => Emulation::Chrome117,
                "Chrome118" => Emulation::Chrome118,
                "Chrome119" => Emulation::Chrome119,
                "Chrome120" => Emulation::Chrome120,
                "Chrome123" => Emulation::Chrome123,
                "Chrome124" => Emulation::Chrome124,
                "Chrome126" => Emulation::Chrome126,
                "Chrome127" => Emulation::Chrome127,
                "Chrome128" => Emulation::Chrome128,
                "Chrome129" => Emulation::Chrome129,
                "Chrome130" => Emulation::Chrome130,
                "Chrome131" => Emulation::Chrome131,
                "Chrome132" => Emulation::Chrome132,
                "Chrome133" => Emulation::Chrome133,
                "Chrome134" => Emulation::Chrome134,
                "Chrome135" => Emulation::Chrome135,
                "Chrome136" => Emulation::Chrome136,
                "Chrome137" => Emulation::Chrome137,
                "Chrome138" => Emulation::Chrome138,
                "Chrome139" => Emulation::Chrome139,
                "Chrome140" => Emulation::Chrome140,
                "Chrome141" => Emulation::Chrome141,
                "Chrome142" => Emulation::Chrome142,
                "Chrome143" => Emulation::Chrome143,

                // Edge
                "Edge96" => Emulation::Edge96,
                "Edge101" => Emulation::Edge101,
                "Edge122" => Emulation::Edge122,
                "Edge127" => Emulation::Edge127,
                "Edge131" => Emulation::Edge131,
                "Edge134" => Emulation::Edge134,
                "Edge135" => Emulation::Edge135,
                "Edge136" => Emulation::Edge136,
                "Edge137" => Emulation::Edge137,
                "Edge138" => Emulation::Edge138,
                "Edge139" => Emulation::Edge139,
                "Edge140" => Emulation::Edge140,
                "Edge141" => Emulation::Edge141,
                "Edge142" => Emulation::Edge142,

                // Opera
                "Opera116" => Emulation::Opera116,
                "Opera117" => Emulation::Opera117,
                "Opera118" => Emulation::Opera118,
                "Opera119" => Emulation::Opera119,

                // Safari
                "Safari14" => Emulation::Safari14,
                "Safari15_3" => Emulation::Safari15_3,
                "Safari15_5" => Emulation::Safari15_5,
                "Safari15_6_1" => Emulation::Safari15_6_1,
                "Safari16" => Emulation::Safari16,
                "Safari16_5" => Emulation::Safari16_5,
                "Safari17_0" => Emulation::Safari17_0,
                "Safari17_2_1" => Emulation::Safari17_2_1,
                "Safari17_4_1" => Emulation::Safari17_4_1,
                "Safari17_5" => Emulation::Safari17_5,
                "Safari17_6" => Emulation::Safari17_6,
                "Safari18" => Emulation::Safari18,
                "Safari18_2" => Emulation::Safari18_2,
                "Safari18_3" => Emulation::Safari18_3,
                "Safari18_3_1" => Emulation::Safari18_3_1,
                "Safari18_5" => Emulation::Safari18_5,
                "Safari26" => Emulation::Safari26,
                "Safari26_1" => Emulation::Safari26_1,
                "Safari26_2" => Emulation::Safari26_2,
                "SafariIos17_2" => Emulation::SafariIos17_2,
                "SafariIos17_4_1" => Emulation::SafariIos17_4_1,
                "SafariIos16_5" => Emulation::SafariIos16_5,
                "SafariIos18_1_1" => Emulation::SafariIos18_1_1,
                "SafariIos26" => Emulation::SafariIos26,
                "SafariIos26_2" => Emulation::SafariIos26_2,
                "SafariIPad18" => Emulation::SafariIPad18,
                "SafariIPad26" => Emulation::SafariIPad26,
                "SafariIpad26_2" => Emulation::SafariIpad26_2,

                // Firefox
                "Firefox88" => Emulation::Firefox88,
                "Firefox109" => Emulation::Firefox109,
                "Firefox117" => Emulation::Firefox117,
                "Firefox128" => Emulation::Firefox128,
                "Firefox133" => Emulation::Firefox133,
                "Firefox135" => Emulation::Firefox135,
                "Firefox136" => Emulation::Firefox136,
                "Firefox139" => Emulation::Firefox139,
                "Firefox142" => Emulation::Firefox142,
                "Firefox143" => Emulation::Firefox143,
                "Firefox144" => Emulation::Firefox144,
                "Firefox145" => Emulation::Firefox145,
                "Firefox146" => Emulation::Firefox146,
                "FirefoxPrivate135" => Emulation::FirefoxPrivate135,
                "FirefoxPrivate136" => Emulation::FirefoxPrivate136,
                "FirefoxAndroid135" => Emulation::FirefoxAndroid135,

                // OkHttp
                "OkHttp3_9" => Emulation::OkHttp3_9,
                "OkHttp3_11" => Emulation::OkHttp3_11,
                "OkHttp3_13" => Emulation::OkHttp3_13,
                "OkHttp3_14" => Emulation::OkHttp3_14,
                "OkHttp4_9" => Emulation::OkHttp4_9,
                "OkHttp4_10" => Emulation::OkHttp4_10,
                "OkHttp4_12" => Emulation::OkHttp4_12,
                "OkHttp5" => Emulation::OkHttp5,

                _ => {
                    return Err(format!(
                        "unknown browser emulation: '{s}'\n\
                         Use one of: Chrome143, Chrome96–Chrome143, \
                         Firefox88–Firefox146, Safari14–Safari26, \
                         Edge96–Edge142, Opera116–Opera119, \
                         OkHttp3_9–OkHttp5"
                    ));
                }
            };
            Ok(BrowserEmulation(emulation))
        }
    }

    impl EmulationFactory for BrowserEmulation {
        fn emulation(self) -> hpx::Emulation {
            self.0.emulation()
        }
    }
}

/// High Performance HTTP Client
#[derive(Parser, Debug)]
#[command(name = "hpx", version, about)]
struct Cli {
    /// URL to send the GET request to
    url: String,

    /// Force HTTP/3 only (no fallback to HTTP/2 or HTTP/1.1)
    #[cfg(feature = "http3")]
    #[arg(
        long,
        conflicts_with = "prefer_http3",
        help_heading = "HTTP/3 Options"
    )]
    http3: bool,

    /// Prefer HTTP/3 with fallback to HTTP/2/HTTP/1.1 (via Alt-Svc discovery)
    #[cfg(feature = "http3")]
    #[arg(
        long,
        conflicts_with = "http3",
        help_heading = "HTTP/3 Options"
    )]
    prefer_http3: bool,

    /// Browser emulation profile (requires `boring` feature).
    /// Examples: Chrome143, Firefox128, Safari18, Edge131.
    #[cfg(feature = "boring")]
    #[arg(
        long,
        value_name = "BROWSER",
        help_heading = "Emulation Options"
    )]
    emulation: Option<emulation::BrowserEmulation>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    #[cfg_attr(
        not(any(feature = "http3", feature = "boring")),
        allow(unused_mut)
    )]
    let mut builder = hpx::Client::builder();

    #[cfg(feature = "http3")]
    {
        if cli.http3 {
            builder = builder.http3_only();
        }
        if cli.prefer_http3 {
            builder = builder.prefer_http3();
        }
    }

    #[cfg(feature = "boring")]
    if let Some(emu) = cli.emulation {
        builder = builder.emulation(emu);
    }

    let client = builder.build()?;
    let resp = client.get(&cli.url).send().await?;

    println!("Status: {}", resp.status());
    println!("Version: {:?}", resp.version());
    for (name, value) in resp.headers() {
        println!("{}: {}", name, value.to_str().unwrap_or("<binary>"));
    }
    println!();
    println!("{}", resp.text().await?);

    Ok(())
}