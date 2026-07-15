use clap::{Parser, ValueEnum};
use hpx_browser::stealth::StealthProfile;

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum ProfileArg {
    #[default]
    Chrome,
    Firefox,
    Safari,
}

impl ProfileArg {
    pub(crate) fn stealth_profile(self) -> StealthProfile {
        match self {
            Self::Chrome => StealthProfile::default(),
            Self::Firefox => hpx_browser::stealth::firefox_135_macos(),
            Self::Safari => hpx_browser::stealth::iphone_15_pro_safari_18(),
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "hpxless",
    about = "CDP-compatible browser server for Puppeteer/Playwright"
)]
pub(crate) struct Cli {
    /// WebSocket port for CDP server
    #[arg(long, default_value_t = 9222, env = "HPXLESS_PORT")]
    pub(crate) port: u16,

    /// Enable stealth mode (anti-detection)
    #[arg(long, default_value_t = false)]
    pub(crate) stealth: bool,

    /// Browser profile to emulate
    #[arg(long, value_enum, default_value_t)]
    pub(crate) profile: ProfileArg,

    /// Proxy URL (e.g. http://127.0.0.1:8080)
    #[arg(long, env = "HPXLESS_PROXY")]
    pub(crate) proxy: Option<String>,

    /// URL patterns to block (repeatable)
    #[arg(long = "block", value_delimiter = ',')]
    pub(crate) block: Vec<String>,

    /// URL to navigate on connect
    #[arg(long)]
    pub(crate) url: Option<String>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info", env = "HPXLESS_LOG_LEVEL")]
    pub(crate) log_level: String,
}

impl Cli {
    #[cfg(test)]
    pub(crate) fn proxy_config(&self) -> eyre::Result<Option<hpx::Proxy>> {
        match &self.proxy {
            Some(url) => Ok(Some(hpx::Proxy::all(url.as_str())?)),
            None => Ok(None),
        }
    }

    pub(crate) fn stealth_profile(&self) -> Option<StealthProfile> {
        self.stealth.then(|| self.profile.stealth_profile())
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn parse_proxy_socks5() {
        let cli = Cli::parse_from(["hpxless", "--proxy", "socks5://127.0.0.1:1080"]);
        let proxy = cli.proxy_config().unwrap().expect("proxy should be set");
        drop(proxy);
    }

    #[test]
    fn parse_proxy_http() {
        let cli = Cli::parse_from(["hpxless", "--proxy", "http://127.0.0.1:8080"]);
        let proxy = cli.proxy_config().unwrap().expect("proxy should be set");
        drop(proxy);
    }

    #[test]
    fn parse_proxy_https() {
        let cli = Cli::parse_from(["hpxless", "--proxy", "https://127.0.0.1:8443"]);
        let proxy = cli.proxy_config().unwrap().expect("proxy should be set");
        drop(proxy);
    }

    #[test]
    fn no_proxy() {
        let cli = Cli::parse_from(["hpxless"]);
        assert!(cli.proxy_config().unwrap().is_none());
    }

    #[test]
    fn chrome_profile_maps_to_default() {
        let p = ProfileArg::Chrome.stealth_profile();
        assert_eq!(p.browser_name, "Chrome");
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }

    #[test]
    fn firefox_profile_maps_correctly() {
        let p = ProfileArg::Firefox.stealth_profile();
        assert_eq!(p.browser_name, "Firefox");
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }

    #[test]
    fn safari_profile_maps_correctly() {
        let p = ProfileArg::Safari.stealth_profile();
        assert_eq!(p.browser_name, "Safari");
        assert!(p.validate().is_ok(), "{:?}", p.validate());
    }

    #[test]
    fn stealth_disabled_returns_none() {
        let cli = Cli::parse_from(["hpxless"]);
        assert!(!cli.stealth);
        assert!(cli.stealth_profile().is_none());
    }

    #[test]
    fn stealth_enabled_returns_some() {
        let cli = Cli::parse_from(["hpxless", "--stealth"]);
        assert!(cli.stealth);
        let profile = cli.stealth_profile().expect("should have profile");
        assert_eq!(profile.browser_name, "Chrome"); // default profile
    }

    #[test]
    fn stealth_with_explicit_firefox_profile() {
        let cli = Cli::parse_from(["hpxless", "--stealth", "--profile", "firefox"]);
        let profile = cli.stealth_profile().expect("should have profile");
        assert_eq!(profile.browser_name, "Firefox");
    }
}
