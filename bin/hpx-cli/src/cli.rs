use clap::{ArgAction, Parser, Subcommand, ValueEnum};

/// HTTP method to use for requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "UPPER")]
pub(crate) enum Method {
    #[default]
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
}

impl Method {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Patch => "PATCH",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }
}

/// Output format for response body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputFormat {
    #[default]
    Auto,
    Json,
    Text,
    Raw,
}

/// Color mode for terminal output.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum ColorMode {
    #[default]
    Auto,
    On,
    Off,
}

/// Download management commands.
#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    /// Download management commands.
    #[command(subcommand)]
    Dl(DlCommands),
}

#[derive(Debug, Subcommand)]
pub(crate) enum DlCommands {
    /// Add a new download.
    Add {
        /// URL to download.
        url: String,
        /// Destination file path.
        #[arg(short, long)]
        output: Option<String>,
        /// Priority (low, normal, high, critical).
        #[arg(short, long, default_value = "normal")]
        priority: String,
        /// Speed limit (e.g. "1MB/s", "500KB/s", "1024" bytes/s).
        #[arg(long)]
        speed_limit: Option<String>,
        /// Checksum for integrity verification (e.g. "sha256:<hex>").
        #[arg(long)]
        checksum: Option<String>,
        /// Mirror URLs to try on failure (can be repeated).
        #[arg(long = "mirror", action = ArgAction::Append)]
        mirrors: Vec<String>,
        /// Maximum number of parallel connections per download.
        #[arg(long = "max-connections")]
        max_connections: Option<usize>,
        /// Set request headers (can be repeated, format: "Name:Value").
        #[arg(short = 'H', long = "header", value_name = "NAME:VALUE")]
        headers: Vec<String>,
        /// Use a proxy for this download (e.g. "http://proxy:8080", "socks5://proxy:1080").
        #[arg(long, value_name = "URL")]
        proxy: Option<String>,
        /// Number of retries on transient errors for this download.
        #[arg(long, value_name = "NUM")]
        retry: Option<u32>,
    },
    /// Pause a download.
    Pause {
        /// Download ID.
        id: String,
    },
    /// Resume a paused download.
    Resume {
        /// Download ID.
        id: String,
    },
    /// Remove a download.
    Remove {
        /// Download ID.
        id: String,
    },
    /// List all downloads.
    List {
        /// Output format [auto, json, text].
        #[arg(long, value_enum, default_value_t)]
        format: OutputFormat,
    },
    /// Show download status.
    Status {
        /// Download ID.
        id: String,
        /// Output format [auto, json, text].
        #[arg(long, value_enum, default_value_t)]
        format: OutputFormat,
    },
}

/// High-performance HTTP client and download engine.
#[derive(Debug, Parser)]
#[command(
    name = "hpx",
    about = "High-performance HTTP client and download engine",
    disable_help_flag = true,
    after_help = "Examples:\n  hpx httpbin.org/get\n  hpx -X POST httpbin.org/post -d '{\"key\":\"value\"}'\n  hpx -H 'Authorization: Bearer token' https://api.example.com\n  hpx ws://echo.websocket.org"
)]
pub(crate) struct Cli {
    /// The URL to make a request to (for direct HTTP/WS requests).
    pub url: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,

    /// HTTP method to use.
    #[arg(
        short = 'X',
        long = "method",
        value_enum,
        default_value_t,
        env = "HPX_METHOD"
    )]
    pub method: Method,

    /// Set request headers (can be repeated).
    #[arg(short = 'H', long = "header", value_name = "NAME:VALUE")]
    pub headers: Vec<String>,

    /// Send a request body.
    #[arg(short = 'd', long = "data", value_name = "[@]VALUE")]
    pub data: Option<String>,

    /// Send a JSON request body.
    #[arg(short = 'j', long = "json", value_name = "[@]VALUE")]
    pub json: Option<String>,

    /// Write the response body to a file.
    #[arg(short = 'o', long = "output", value_name = "PATH")]
    pub output: Option<String>,

    /// Enable HTTP basic authentication.
    #[arg(long, value_name = "USER:PASS")]
    pub basic: Option<String>,

    /// Enable HTTP bearer authentication.
    #[arg(long, value_name = "TOKEN")]
    pub bearer: Option<String>,

    /// Timeout for the request in seconds.
    #[arg(short = 't', long, value_name = "SECONDS", env = "HPX_TIMEOUT")]
    pub timeout: Option<f64>,

    /// Maximum number of redirects.
    #[arg(long, value_name = "NUM", env = "HPX_REDIRECTS")]
    pub redirects: Option<usize>,

    /// Output format [auto, json, text, raw].
    #[arg(long, value_enum, default_value_t)]
    pub format: OutputFormat,

    /// Color mode [auto, on, off].
    #[arg(long, value_enum, default_value_t)]
    pub color: ColorMode,

    /// Print verbose output.
    #[arg(short = 'v', long, action = ArgAction::Count)]
    pub verbose: u8,

    /// Print only errors to stderr.
    #[arg(short = 's', long)]
    pub silent: bool,

    /// Print request info without sending.
    #[arg(long)]
    pub dry_run: bool,

    /// Follow redirects.
    #[arg(short = 'L', long, env = "HPX_FOLLOW")]
    pub follow: bool,

    /// Print timing waterfall.
    #[arg(short = 'T', long = "timing")]
    pub timing: bool,

    /// Send URL-encoded form data (key=value pairs).
    #[arg(short = 'f', long = "form", value_name = "KEY=VALUE", action = clap::ArgAction::Append)]
    pub form: Vec<String>,

    /// Send multipart form data.
    #[arg(long = "multipart", value_name = "KEY=VALUE", action = clap::ArgAction::Append)]
    pub multipart: Vec<String>,

    /// Add a file to multipart form (name=@filepath).
    #[arg(long = "multipart-file", value_name = "KEY=@PATH", action = clap::ArgAction::Append)]
    pub multipart_file: Vec<String>,

    /// Set a cookie (can be repeated).
    #[arg(long = "cookie", value_name = "NAME=VALUE")]
    pub cookie: Vec<String>,

    /// Save cookies to file after request.
    #[arg(long = "cookie-jar", value_name = "PATH")]
    pub cookie_jar: Option<String>,

    /// Use a proxy (http, https, socks5).
    #[arg(long, value_name = "URL", env = "HPX_PROXY")]
    pub proxy: Option<String>,

    /// Path to the download state database.
    #[arg(long, value_name = "PATH")]
    pub storage_path: Option<std::path::PathBuf>,

    /// Maximum number of concurrent downloads.
    #[arg(long, value_name = "NUM")]
    pub max_concurrent: Option<usize>,

    /// Number of retries on transient errors.
    #[arg(long, value_name = "NUM", default_value = "0")]
    pub retry: u32,

    /// Automatically reconnect WebSocket on disconnection.
    #[arg(long)]
    pub reconnect: bool,

    /// Maximum number of WebSocket reconnection attempts (0 = unlimited).
    #[arg(long, value_name = "NUM", default_value = "0")]
    pub reconnect_max: u32,

    /// Copy response body to clipboard.
    #[arg(long)]
    pub clipboard: bool,

    /// Print version.
    #[arg(short = 'V', long)]
    pub version: bool,

    /// Print help.
    #[arg(short = 'h', long)]
    pub help: bool,
}

fn parse_pairs(items: &[String], sep: char) -> Vec<(String, String)> {
    items
        .iter()
        .filter_map(|item| {
            item.split_once(sep)
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
        })
        .collect()
}

impl Cli {
    /// Returns true if the URL is a WebSocket URL.
    pub(crate) fn is_websocket_url(&self) -> bool {
        self.url.as_deref().is_some_and(|u| {
            let lower = u.to_ascii_lowercase();
            lower.starts_with("ws://") || lower.starts_with("wss://")
        })
    }

    /// Parse headers into (name, value) pairs.
    pub(crate) fn parsed_headers(&self) -> Vec<(String, String)> {
        parse_pairs(&self.headers, ':')
            .into_iter()
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .collect()
    }

    /// Parse --form fields into (key, value) pairs.
    pub(crate) fn parsed_form_fields(&self) -> Vec<(String, String)> {
        parse_pairs(&self.form, '=')
    }

    /// Parse --form fields, returning file references for @-prefixed values.
    pub(crate) fn parsed_form_fields_with_files(&self) -> Vec<(String, FormValue)> {
        self.form
            .iter()
            .filter_map(|f| {
                let (key, value) = f.split_once('=')?;
                let form_value = if let Some(path) = value.strip_prefix('@') {
                    FormValue::File(path.to_string())
                } else {
                    FormValue::Text(value.to_string())
                };
                Some((key.to_string(), form_value))
            })
            .collect()
    }

    /// Returns true if any form field contains a file reference (@ prefix).
    pub(crate) fn has_form_file_references(&self) -> bool {
        self.form
            .iter()
            .any(|f| f.split_once('=').is_some_and(|(_, v)| v.starts_with('@')))
    }

    /// Parse --multipart fields into (key, value) pairs.
    pub(crate) fn parsed_multipart_fields(&self) -> Vec<(String, String)> {
        parse_pairs(&self.multipart, '=')
    }

    /// Parse --multipart-file fields into (key, path) pairs.
    pub(crate) fn parsed_multipart_files(&self) -> Vec<(String, String)> {
        parse_pairs(&self.multipart_file, '=')
            .into_iter()
            .filter_map(|(k, v)| Some((k, v.strip_prefix('@')?.to_owned())))
            .collect()
    }

    /// Parse --cookie values into (name, value) pairs.
    pub(crate) fn parsed_cookies(&self) -> Vec<(String, String)> {
        parse_pairs(&self.cookie, '=')
            .into_iter()
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .collect()
    }
}

/// A form field value that can be either plain text or a file reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormValue {
    /// Plain text value.
    Text(String),
    /// File path reference (from @-prefixed value).
    File(String),
}

/// Parse a proxy URL string into a [`hpx_dl::ProxyConfig`].
///
/// Detects the protocol kind from the URL scheme:
/// - `"http://..."` → [`ProxyKind::Http`]
/// - `"https://..."` → [`ProxyKind::Https`]
/// - `"socks5://..."` → [`ProxyKind::Socks5`]
///
/// Returns an error for unrecognized schemes.
pub(crate) fn parse_proxy_config(url: &str) -> eyre::Result<hpx_dl::ProxyConfig> {
    let kind = if url.starts_with("http://") {
        hpx_dl::ProxyKind::Http
    } else if url.starts_with("https://") {
        hpx_dl::ProxyKind::Https
    } else if url.starts_with("socks5://") {
        hpx_dl::ProxyKind::Socks5
    } else {
        return Err(eyre::eyre!(
            "unknown proxy scheme in '{url}', expected one of: http://, https://, socks5://"
        ));
    };
    Ok(hpx_dl::ProxyConfig {
        url: url.to_string(),
        kind,
    })
}

/// Parse header strings (from `-H` flags) into (name, value) pairs.
///
/// Filters out entries that don't contain `:`.
pub(crate) fn parsed_dl_headers(headers: &[String]) -> Vec<(String, String)> {
    parse_pairs(headers, ':')
        .into_iter()
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

/// Parse a human-readable speed string into bytes per second.
///
/// Supported formats:
/// - `"1024"` → raw bytes per second
/// - `"1KB/s"`, `"500KB/s"` → kilobytes per second
/// - `"1MB/s"`, `"2.5MB/s"` → megabytes per second
/// - `"1GB/s"` → gigabytes per second
pub(crate) fn parse_speed_limit(s: &str) -> eyre::Result<u64> {
    let s = s.trim();

    if let Some(n) = s.strip_suffix("GB/s") {
        let n: f64 = n.trim().parse()?;
        return Ok((n * 1_073_741_824.0) as u64);
    }
    if let Some(n) = s.strip_suffix("MB/s") {
        let n: f64 = n.trim().parse()?;
        return Ok((n * 1_048_576.0) as u64);
    }
    if let Some(n) = s.strip_suffix("KB/s") {
        let n: f64 = n.trim().parse()?;
        return Ok((n * 1_024.0) as u64);
    }

    let bytes: u64 = s.parse()?;
    Ok(bytes)
}

/// Parse a checksum specification string in the format `"algorithm:hex_value"`.
///
/// Supported algorithms: `md5`, `sha1`, `sha256`.
/// The algorithm name is case-insensitive.
pub(crate) fn parse_checksum(s: &str) -> eyre::Result<hpx_dl::ChecksumSpec> {
    let s = s.trim();
    let (algo_str, expected) = s
        .split_once(':')
        .ok_or_else(|| eyre::eyre!("invalid checksum format, expected 'algorithm:hex_value'"))?;

    if expected.is_empty() {
        return Err(eyre::eyre!("checksum hash value must not be empty"));
    }

    let algorithm = match algo_str.to_lowercase().as_str() {
        "md5" => hpx_dl::HashAlgorithm::Md5,
        "sha1" => hpx_dl::HashAlgorithm::Sha1,
        "sha256" => hpx_dl::HashAlgorithm::Sha256,
        other => {
            return Err(eyre::eyre!(
                "unknown hash algorithm '{other}', expected one of: md5, sha1, sha256"
            ));
        }
    };

    Ok(hpx_dl::ChecksumSpec {
        algorithm,
        expected: expected.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn parse_speed_limit_raw_bytes() {
        assert_eq!(parse_speed_limit("1024").unwrap(), 1024);
        assert_eq!(parse_speed_limit("1").unwrap(), 1);
        assert_eq!(parse_speed_limit("0").unwrap(), 0);
    }

    #[test]
    fn parse_speed_limit_kilobytes() {
        assert_eq!(parse_speed_limit("1KB/s").unwrap(), 1_024);
        assert_eq!(parse_speed_limit("500KB/s").unwrap(), 512_000);
        assert_eq!(parse_speed_limit("10 KB/s").unwrap(), 10_240);
    }

    #[test]
    fn parse_speed_limit_megabytes() {
        assert_eq!(parse_speed_limit("1MB/s").unwrap(), 1_048_576);
        assert_eq!(parse_speed_limit("2.5MB/s").unwrap(), 2_621_440);
        assert_eq!(parse_speed_limit("10 MB/s").unwrap(), 10_485_760);
    }

    #[test]
    fn parse_speed_limit_gigabytes() {
        assert_eq!(parse_speed_limit("1GB/s").unwrap(), 1_073_741_824);
        assert_eq!(parse_speed_limit("2GB/s").unwrap(), 2_147_483_648);
    }

    #[test]
    fn parse_speed_limit_invalid_empty() {
        assert!(parse_speed_limit("").is_err());
    }

    #[test]
    fn parse_speed_limit_invalid_text() {
        assert!(parse_speed_limit("abc").is_err());
    }

    #[test]
    fn parse_speed_limit_invalid_unit() {
        assert!(parse_speed_limit("1TB/s").is_err());
    }

    #[test]
    fn parse_speed_limit_invalid_format() {
        assert!(parse_speed_limit("MB/s").is_err());
    }

    #[test]
    fn parse_speed_limit_whitespace_handling() {
        assert_eq!(parse_speed_limit("  1MB/s  ").unwrap(), 1_048_576);
        assert_eq!(parse_speed_limit("  1024  ").unwrap(), 1024);
    }

    #[test]
    fn parse_checksum_sha256() {
        let spec = parse_checksum(
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        )
        .unwrap();
        assert_eq!(spec.algorithm, hpx_dl::HashAlgorithm::Sha256);
        assert_eq!(
            spec.expected,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn parse_checksum_md5() {
        let spec = parse_checksum("md5:d41d8cd98f00b204e9800998ecf8427e").unwrap();
        assert_eq!(spec.algorithm, hpx_dl::HashAlgorithm::Md5);
        assert_eq!(spec.expected, "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn parse_checksum_sha1() {
        let spec = parse_checksum("sha1:da39a3ee5e6b4b0d3255bfef95601890afd80709").unwrap();
        assert_eq!(spec.algorithm, hpx_dl::HashAlgorithm::Sha1);
        assert_eq!(spec.expected, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn parse_checksum_sha256_uppercase() {
        let spec = parse_checksum(
            "SHA256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        )
        .unwrap();
        assert_eq!(spec.algorithm, hpx_dl::HashAlgorithm::Sha256);
    }

    #[test]
    fn parse_checksum_invalid_no_colon() {
        assert!(parse_checksum("e3b0c44298fc1c149afbf4c8996fb924").is_err());
    }

    #[test]
    fn parse_checksum_invalid_unknown_algorithm() {
        assert!(parse_checksum("sha512:abc123").is_err());
    }

    #[test]
    fn parse_checksum_invalid_empty_string() {
        assert!(parse_checksum("").is_err());
    }

    #[test]
    fn parse_checksum_invalid_no_hash_value() {
        assert!(parse_checksum("sha256:").is_err());
    }

    #[test]
    fn parse_checksum_invalid_empty_algorithm() {
        assert!(parse_checksum(":abc123").is_err());
    }

    #[test]
    fn dl_add_mirror_single() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "hpx",
            "dl",
            "add",
            "https://example.com/file.bin",
            "--mirror",
            "https://mirror1.example.com/file.bin",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add { mirrors, .. }) => {
                assert_eq!(mirrors, vec!["https://mirror1.example.com/file.bin"]);
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn dl_add_mirror_multiple() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "hpx",
            "dl",
            "add",
            "https://example.com/file.bin",
            "--mirror",
            "https://mirror1.example.com/file.bin",
            "--mirror",
            "https://mirror2.example.com/file.bin",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add { mirrors, .. }) => {
                assert_eq!(
                    mirrors,
                    vec![
                        "https://mirror1.example.com/file.bin",
                        "https://mirror2.example.com/file.bin"
                    ]
                );
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn dl_add_mirror_none() {
        use clap::Parser;
        let cli =
            Cli::try_parse_from(["hpx", "dl", "add", "https://example.com/file.bin"]).unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add { mirrors, .. }) => {
                assert!(mirrors.is_empty());
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn dl_add_max_connections() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "hpx",
            "dl",
            "add",
            "https://example.com/file.bin",
            "--max-connections",
            "8",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add {
                max_connections, ..
            }) => {
                assert_eq!(max_connections, Some(8));
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn dl_add_max_connections_none() {
        use clap::Parser;
        let cli =
            Cli::try_parse_from(["hpx", "dl", "add", "https://example.com/file.bin"]).unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add {
                max_connections, ..
            }) => {
                assert_eq!(max_connections, None);
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn dl_add_header_single() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "hpx",
            "dl",
            "add",
            "https://example.com/file.bin",
            "-H",
            "Authorization:Bearer token123",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add { headers, .. }) => {
                assert_eq!(headers, vec!["Authorization:Bearer token123"]);
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn dl_add_header_multiple() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "hpx",
            "dl",
            "add",
            "https://example.com/file.bin",
            "-H",
            "Authorization:Bearer token123",
            "--header",
            "X-Custom:value",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add { headers, .. }) => {
                assert_eq!(
                    headers,
                    vec!["Authorization:Bearer token123", "X-Custom:value"]
                );
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn dl_add_header_none() {
        use clap::Parser;
        let cli =
            Cli::try_parse_from(["hpx", "dl", "add", "https://example.com/file.bin"]).unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add { headers, .. }) => {
                assert!(headers.is_empty());
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn parsed_dl_headers_filters_invalid() {
        let headers = vec![
            "Authorization:Bearer token123".to_string(),
            "X-Custom: value with spaces".to_string(),
            " NoColon".to_string(),
        ];
        let parsed = super::parsed_dl_headers(&headers);
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed[0],
            ("Authorization".to_string(), "Bearer token123".to_string())
        );
        assert_eq!(
            parsed[1],
            ("X-Custom".to_string(), "value with spaces".to_string())
        );
    }

    #[test]
    fn parsed_dl_headers_empty() {
        let headers: Vec<String> = vec![];
        let parsed = super::parsed_dl_headers(&headers);
        assert!(parsed.is_empty());
    }

    #[test]
    fn dl_add_proxy_http() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "hpx",
            "dl",
            "add",
            "https://example.com/file.bin",
            "--proxy",
            "http://proxy:8080",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add { proxy, .. }) => {
                assert_eq!(proxy.as_deref(), Some("http://proxy:8080"));
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn dl_add_proxy_socks5() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "hpx",
            "dl",
            "add",
            "https://example.com/file.bin",
            "--proxy",
            "socks5://proxy:1080",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add { proxy, .. }) => {
                assert_eq!(proxy.as_deref(), Some("socks5://proxy:1080"));
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn dl_add_proxy_none() {
        use clap::Parser;
        let cli =
            Cli::try_parse_from(["hpx", "dl", "add", "https://example.com/file.bin"]).unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add { proxy, .. }) => {
                assert!(proxy.is_none());
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn dl_add_retry_some() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "hpx",
            "dl",
            "add",
            "https://example.com/file.bin",
            "--retry",
            "5",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add { retry, .. }) => {
                assert_eq!(retry, Some(5));
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn dl_add_retry_none() {
        use clap::Parser;
        let cli =
            Cli::try_parse_from(["hpx", "dl", "add", "https://example.com/file.bin"]).unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add { retry, .. }) => {
                assert_eq!(retry, None);
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn dl_list_format_json() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "dl", "list", "--format", "json"]).unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::List { format }) => {
                assert_eq!(format, OutputFormat::Json);
            }
            _ => panic!("expected DlCommands::List"),
        }
    }

    #[test]
    fn dl_list_format_default() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "dl", "list"]).unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::List { format }) => {
                assert_eq!(format, OutputFormat::Auto);
            }
            _ => panic!("expected DlCommands::List"),
        }
    }

    #[test]
    fn dl_status_format_json() {
        use clap::Parser;
        let cli =
            Cli::try_parse_from(["hpx", "dl", "status", "test-id", "--format", "json"]).unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Status { id, format }) => {
                assert_eq!(id, "test-id");
                assert_eq!(format, OutputFormat::Json);
            }
            _ => panic!("expected DlCommands::Status"),
        }
    }

    #[test]
    fn dl_status_format_default() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "dl", "status", "test-id"]).unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Status { id, format }) => {
                assert_eq!(id, "test-id");
                assert_eq!(format, OutputFormat::Auto);
            }
            _ => panic!("expected DlCommands::Status"),
        }
    }

    #[test]
    fn global_retry_default() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.retry, 0);
    }

    #[test]
    fn global_retry_custom() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "--retry", "5", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.retry, 5);
    }

    #[test]
    fn parse_proxy_config_http() {
        let config = super::parse_proxy_config("http://proxy:8080").unwrap();
        assert_eq!(config.url, "http://proxy:8080");
        assert_eq!(config.kind, hpx_dl::ProxyKind::Http);
    }

    #[test]
    fn parse_proxy_config_https() {
        let config = super::parse_proxy_config("https://proxy:443").unwrap();
        assert_eq!(config.url, "https://proxy:443");
        assert_eq!(config.kind, hpx_dl::ProxyKind::Https);
    }

    #[test]
    fn parse_proxy_config_socks5() {
        let config = super::parse_proxy_config("socks5://proxy:1080").unwrap();
        assert_eq!(config.url, "socks5://proxy:1080");
        assert_eq!(config.kind, hpx_dl::ProxyKind::Socks5);
    }

    #[test]
    fn parse_proxy_config_unknown_scheme() {
        assert!(super::parse_proxy_config("ftp://proxy:21").is_err());
    }

    #[test]
    fn parse_proxy_config_no_scheme() {
        assert!(super::parse_proxy_config("proxy:8080").is_err());
    }

    #[test]
    fn global_storage_path_default() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "httpbin.org/get"]).unwrap();
        assert!(cli.storage_path.is_none());
    }

    #[test]
    fn global_storage_path_custom() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "--storage-path", "/tmp/hpx.db", "httpbin.org/get"])
            .unwrap();
        assert_eq!(
            cli.storage_path,
            Some(std::path::PathBuf::from("/tmp/hpx.db"))
        );
    }

    #[test]
    fn global_max_concurrent_default() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "httpbin.org/get"]).unwrap();
        assert!(cli.max_concurrent.is_none());
    }

    #[test]
    fn global_max_concurrent_custom() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "--max-concurrent", "8", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.max_concurrent, Some(8));
    }

    #[test]
    fn reconnect_flag_default() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "ws://localhost"]).unwrap();
        assert!(!cli.reconnect);
    }

    #[test]
    fn reconnect_flag_enabled() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "--reconnect", "ws://localhost"]).unwrap();
        assert!(cli.reconnect);
    }

    #[test]
    fn reconnect_max_default() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "ws://localhost"]).unwrap();
        assert_eq!(cli.reconnect_max, 0);
    }

    #[test]
    fn reconnect_max_custom() {
        use clap::Parser;
        let cli = Cli::try_parse_from([
            "hpx",
            "--reconnect",
            "--reconnect-max",
            "5",
            "ws://localhost",
        ])
        .unwrap();
        assert!(cli.reconnect);
        assert_eq!(cli.reconnect_max, 5);
    }

    #[test]
    fn env_var_timeout() {
        use clap::Parser;
        // SAFETY: single-threaded test, no concurrent env access
        unsafe { std::env::set_var("HPX_TIMEOUT", "42") };
        let cli = Cli::try_parse_from(["hpx", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.timeout, Some(42.0));
        unsafe { std::env::remove_var("HPX_TIMEOUT") };
    }

    #[test]
    fn env_var_timeout_flag_overrides() {
        use clap::Parser;
        // SAFETY: single-threaded test, no concurrent env access
        unsafe { std::env::set_var("HPX_TIMEOUT", "42") };
        let cli = Cli::try_parse_from(["hpx", "--timeout", "10", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.timeout, Some(10.0));
        unsafe { std::env::remove_var("HPX_TIMEOUT") };
    }

    #[test]
    fn env_var_method() {
        use clap::Parser;
        // SAFETY: single-threaded test, no concurrent env access
        unsafe { std::env::set_var("HPX_METHOD", "POST") };
        let cli = Cli::try_parse_from(["hpx", "httpbin.org/post"]).unwrap();
        assert_eq!(cli.method, Method::Post);
        unsafe { std::env::remove_var("HPX_METHOD") };
    }

    #[test]
    fn env_var_method_flag_overrides() {
        use clap::Parser;
        // SAFETY: single-threaded test, no concurrent env access
        unsafe { std::env::set_var("HPX_METHOD", "POST") };
        let cli = Cli::try_parse_from(["hpx", "-X", "GET", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.method, Method::Get);
        unsafe { std::env::remove_var("HPX_METHOD") };
    }

    #[test]
    fn env_var_redirects() {
        use clap::Parser;
        // SAFETY: single-threaded test, no concurrent env access
        unsafe { std::env::set_var("HPX_REDIRECTS", "5") };
        let cli = Cli::try_parse_from(["hpx", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.redirects, Some(5));
        unsafe { std::env::remove_var("HPX_REDIRECTS") };
    }

    #[test]
    fn env_var_proxy() {
        use clap::Parser;
        // SAFETY: single-threaded test, no concurrent env access
        unsafe { std::env::set_var("HPX_PROXY", "http://proxy:8080") };
        let cli = Cli::try_parse_from(["hpx", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.proxy.as_deref(), Some("http://proxy:8080"));
        unsafe { std::env::remove_var("HPX_PROXY") };
    }

    #[test]
    fn env_var_follow() {
        use clap::Parser;
        // SAFETY: single-threaded test, no concurrent env access
        unsafe { std::env::set_var("HPX_FOLLOW", "true") };
        let cli = Cli::try_parse_from(["hpx", "httpbin.org/get"]).unwrap();
        assert!(cli.follow);
        unsafe { std::env::remove_var("HPX_FOLLOW") };
    }

    #[test]
    fn parsed_form_fields_text_only() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "-f", "name=alice", "-f", "age=30"]).unwrap();
        let fields = cli.parsed_form_fields();
        assert_eq!(
            fields,
            vec![
                ("name".to_string(), "alice".to_string()),
                ("age".to_string(), "30".to_string()),
            ]
        );
    }

    #[test]
    fn parsed_form_fields_with_files_text_only() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "-f", "name=alice"]).unwrap();
        let fields = cli.parsed_form_fields_with_files();
        assert_eq!(
            fields,
            vec![("name".to_string(), FormValue::Text("alice".to_string()))]
        );
    }

    #[test]
    fn parsed_form_fields_with_files_at_prefix() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "-f", "file=@/tmp/test.txt"]).unwrap();
        let fields = cli.parsed_form_fields_with_files();
        assert_eq!(
            fields,
            vec![(
                "file".to_string(),
                FormValue::File("/tmp/test.txt".to_string())
            )]
        );
    }

    #[test]
    fn parsed_form_fields_with_files_mixed() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "-f", "name=alice", "-f", "avatar=@/tmp/photo.png"])
            .unwrap();
        let fields = cli.parsed_form_fields_with_files();
        assert_eq!(fields.len(), 2);
        assert_eq!(
            fields[0],
            ("name".to_string(), FormValue::Text("alice".to_string()))
        );
        assert_eq!(
            fields[1],
            (
                "avatar".to_string(),
                FormValue::File("/tmp/photo.png".to_string())
            )
        );
    }

    #[test]
    fn has_form_file_references_true() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "-f", "file=@/tmp/test.txt"]).unwrap();
        assert!(cli.has_form_file_references());
    }

    #[test]
    fn has_form_file_references_false() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "-f", "name=alice"]).unwrap();
        assert!(!cli.has_form_file_references());
    }

    #[test]
    fn has_form_file_references_empty() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx"]).unwrap();
        assert!(!cli.has_form_file_references());
    }

    #[test]
    fn form_value_at_only_prefix() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "-f", "data=@"]).unwrap();
        let fields = cli.parsed_form_fields_with_files();
        assert_eq!(
            fields,
            vec![("data".to_string(), FormValue::File(String::new()))]
        );
    }

    #[test]
    fn parsed_form_fields_with_files_read_file_content() {
        use clap::Parser;
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.txt");
        let file_content = b"Hello from file upload!";
        std::fs::write(&file_path, file_content).unwrap();

        let cli =
            Cli::try_parse_from(["hpx", "-f", &format!("file=@{}", file_path.display())]).unwrap();

        let fields = cli.parsed_form_fields_with_files();
        assert_eq!(fields.len(), 1);

        if let FormValue::File(ref path) = fields[0].1 {
            let content = std::fs::read(path).unwrap();
            assert_eq!(content, file_content);
        } else {
            panic!("expected FormValue::File");
        }
    }

    #[test]
    fn parsed_form_fields_with_files_nonexistent_path() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "-f", "file=@/nonexistent/path.txt"]).unwrap();

        let fields = cli.parsed_form_fields_with_files();
        assert_eq!(fields.len(), 1);

        if let FormValue::File(ref path) = fields[0].1 {
            assert!(!std::path::Path::new(path).exists());
        } else {
            panic!("expected FormValue::File");
        }
    }

    #[test]
    fn parsed_form_fields_preserves_original_for_non_at() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["hpx", "-f", "key=value=with=equals"]).unwrap();

        let fields = cli.parsed_form_fields_with_files();
        assert_eq!(fields.len(), 1);
        assert_eq!(
            fields[0].1,
            FormValue::Text("value=with=equals".to_string())
        );
    }

    proptest::proptest! {
        #[test]
        fn parse_speed_limit_raw_bytes_roundtrip(n in 0u64..1_000_000_000) {
            let s = format!("{n}");
            let parsed = parse_speed_limit(&s).unwrap();
            prop_assert_eq!(parsed, n);
        }

        #[test]
        fn parse_speed_limit_kb_roundtrip(n in 0u64..1_000_000) {
            let s = format!("{n}KB/s");
            let parsed = parse_speed_limit(&s).unwrap();
            prop_assert_eq!(parsed, n * 1_024);
        }

        #[test]
        fn parse_speed_limit_mb_roundtrip(n in 0u64..10_000) {
            let s = format!("{n}MB/s");
            let parsed = parse_speed_limit(&s).unwrap();
            prop_assert_eq!(parsed, n * 1_048_576);
        }

        #[test]
        fn parse_speed_limit_gb_roundtrip(n in 0u64..100) {
            let s = format!("{n}GB/s");
            let parsed = parse_speed_limit(&s).unwrap();
            prop_assert_eq!(parsed, n * 1_073_741_824);
        }

        #[test]
        fn prop_parsed_headers_valid(name in "[a-zA-Z][a-zA-Z0-9_]{0,29}", value in "[a-zA-Z0-9 _,./;:=?&#]{0,100}") {
            let header_str = format!("{name}:{value}");
            let cli = Cli::try_parse_from(["hpx", "-H", &header_str]).unwrap();
            let parsed = cli.parsed_headers();
            prop_assert_eq!(parsed.len(), 1);
            prop_assert_eq!(&parsed[0].0, name.trim());
            prop_assert_eq!(&parsed[0].1, value.trim());
        }

        #[test]
        fn prop_parsed_cookies_valid(name in "[a-zA-Z][a-zA-Z0-9_]{0,29}", value in "[a-zA-Z0-9 _,./;:=?&#]{0,100}") {
            let cookie_str = format!("{name}={value}");
            let cli = Cli::try_parse_from(["hpx", "--cookie", &cookie_str]).unwrap();
            let parsed = cli.parsed_cookies();
            prop_assert_eq!(parsed.len(), 1);
            prop_assert_eq!(&parsed[0].0, name.trim());
            prop_assert_eq!(&parsed[0].1, value.trim());
        }
    }

    // ── is_websocket_url ──

    #[test]
    fn is_websocket_url_ws() {
        let cli = Cli::try_parse_from(["hpx", "ws://localhost:8080"]).unwrap();
        assert!(cli.is_websocket_url());
    }

    #[test]
    fn is_websocket_url_wss() {
        let cli = Cli::try_parse_from(["hpx", "wss://example.com"]).unwrap();
        assert!(cli.is_websocket_url());
    }

    #[test]
    fn is_websocket_url_uppercase() {
        let cli = Cli::try_parse_from(["hpx", "WS://localhost"]).unwrap();
        assert!(cli.is_websocket_url());
    }

    #[test]
    fn is_websocket_url_http() {
        let cli = Cli::try_parse_from(["hpx", "http://example.com"]).unwrap();
        assert!(!cli.is_websocket_url());
    }

    #[test]
    fn is_websocket_url_none() {
        let cli = Cli::try_parse_from(["hpx"]).unwrap();
        assert!(!cli.is_websocket_url());
    }

    // ── parsed_headers edge cases ──

    #[test]
    fn parsed_headers_multiple_colons() {
        let cli = Cli::try_parse_from(["hpx", "-H", "Authorization:Bearer token:extra"]).unwrap();
        let parsed = cli.parsed_headers();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "Authorization");
        assert_eq!(parsed[0].1, "Bearer token:extra");
    }

    #[test]
    fn parsed_headers_empty_value() {
        let cli = Cli::try_parse_from(["hpx", "-H", "X-Empty:"]).unwrap();
        let parsed = cli.parsed_headers();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "X-Empty");
        assert_eq!(parsed[0].1, "");
    }

    #[test]
    fn parsed_headers_value_with_colon_space() {
        let cli = Cli::try_parse_from(["hpx", "-H", "Host: example.com:8080"]).unwrap();
        let parsed = cli.parsed_headers();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "Host");
        assert_eq!(parsed[0].1, "example.com:8080");
    }

    #[test]
    fn parsed_headers_no_colon_filtered() {
        let cli = Cli::try_parse_from(["hpx", "-H", "NoColon", "-H", "Valid:value"]).unwrap();
        let parsed = cli.parsed_headers();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], ("Valid".to_string(), "value".to_string()));
    }

    #[test]
    fn parsed_headers_empty_list() {
        let cli = Cli::try_parse_from(["hpx"]).unwrap();
        let parsed = cli.parsed_headers();
        assert!(parsed.is_empty());
    }

    #[test]
    fn parsed_headers_whitespace_trimming() {
        let cli = Cli::try_parse_from(["hpx", "-H", "  Name  :  Value  "]).unwrap();
        let parsed = cli.parsed_headers();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "Name");
        assert_eq!(parsed[0].1, "Value");
    }

    #[test]
    fn parsed_headers_multiple_headers() {
        let cli = Cli::try_parse_from([
            "hpx",
            "-H",
            "Accept:application/json",
            "-H",
            "Content-Type:text/plain",
            "-H",
            "X-Custom:hello",
        ])
        .unwrap();
        let parsed = cli.parsed_headers();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].0, "Accept");
        assert_eq!(parsed[1].0, "Content-Type");
        assert_eq!(parsed[2].0, "X-Custom");
    }

    // ── parsed_cookies edge cases ──

    #[test]
    fn parsed_cookies_basic() {
        let cli = Cli::try_parse_from(["hpx", "--cookie", "session=abc123"]).unwrap();
        let parsed = cli.parsed_cookies();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "session");
        assert_eq!(parsed[0].1, "abc123");
    }

    #[test]
    fn parsed_cookies_multiple_equals() {
        let cli = Cli::try_parse_from(["hpx", "--cookie", "token=a=b=c"]).unwrap();
        let parsed = cli.parsed_cookies();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "token");
        assert_eq!(parsed[0].1, "a=b=c");
    }

    #[test]
    fn parsed_cookies_empty_value() {
        let cli = Cli::try_parse_from(["hpx", "--cookie", "empty="]).unwrap();
        let parsed = cli.parsed_cookies();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "empty");
        assert_eq!(parsed[0].1, "");
    }

    #[test]
    fn parsed_cookies_no_equals_filtered() {
        let cli =
            Cli::try_parse_from(["hpx", "--cookie", "invalid", "--cookie", "valid=ok"]).unwrap();
        let parsed = cli.parsed_cookies();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], ("valid".to_string(), "ok".to_string()));
    }

    #[test]
    fn parsed_cookies_whitespace_trimming() {
        let cli = Cli::try_parse_from(["hpx", "--cookie", "  name  =  value  "]).unwrap();
        let parsed = cli.parsed_cookies();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "name");
        assert_eq!(parsed[0].1, "value");
    }

    #[test]
    fn parsed_cookies_multiple_cookies() {
        let cli = Cli::try_parse_from([
            "hpx", "--cookie", "a=1", "--cookie", "b=2", "--cookie", "c=3",
        ])
        .unwrap();
        let parsed = cli.parsed_cookies();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].0, "a");
        assert_eq!(parsed[1].0, "b");
        assert_eq!(parsed[2].0, "c");
    }

    #[test]
    fn parsed_cookies_empty_list() {
        let cli = Cli::try_parse_from(["hpx"]).unwrap();
        let parsed = cli.parsed_cookies();
        assert!(parsed.is_empty());
    }

    // ── parsed_multipart_fields ──

    #[test]
    fn parsed_multipart_fields_basic() {
        let cli =
            Cli::try_parse_from(["hpx", "--multipart", "name=alice", "--multipart", "age=30"])
                .unwrap();
        let parsed = cli.parsed_multipart_fields();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], ("name".to_string(), "alice".to_string()));
        assert_eq!(parsed[1], ("age".to_string(), "30".to_string()));
    }

    #[test]
    fn parsed_multipart_fields_empty_value() {
        let cli = Cli::try_parse_from(["hpx", "--multipart", "field="]).unwrap();
        let parsed = cli.parsed_multipart_fields();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "field");
        assert_eq!(parsed[0].1, "");
    }

    #[test]
    fn parsed_multipart_fields_no_equals_filtered() {
        let cli = Cli::try_parse_from(["hpx", "--multipart", "invalid"]).unwrap();
        let parsed = cli.parsed_multipart_fields();
        assert!(parsed.is_empty());
    }

    #[test]
    fn parsed_multipart_fields_value_with_equals() {
        let cli = Cli::try_parse_from(["hpx", "--multipart", "data=foo=bar"]).unwrap();
        let parsed = cli.parsed_multipart_fields();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "data");
        assert_eq!(parsed[0].1, "foo=bar");
    }

    #[test]
    fn parsed_multipart_fields_empty_list() {
        let cli = Cli::try_parse_from(["hpx"]).unwrap();
        let parsed = cli.parsed_multipart_fields();
        assert!(parsed.is_empty());
    }

    // ── parsed_multipart_files ──

    #[test]
    fn parsed_multipart_files_basic() {
        let cli =
            Cli::try_parse_from(["hpx", "--multipart-file", "avatar=@/tmp/photo.png"]).unwrap();
        let parsed = cli.parsed_multipart_files();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "avatar");
        assert_eq!(parsed[0].1, "/tmp/photo.png");
    }

    #[test]
    fn parsed_multipart_files_no_at_filtered() {
        let cli = Cli::try_parse_from(["hpx", "--multipart-file", "data=just-a-value"]).unwrap();
        let parsed = cli.parsed_multipart_files();
        assert!(parsed.is_empty());
    }

    #[test]
    fn parsed_multipart_files_empty_path() {
        let cli = Cli::try_parse_from(["hpx", "--multipart-file", "file=@"]).unwrap();
        let parsed = cli.parsed_multipart_files();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "file");
        assert_eq!(parsed[0].1, "");
    }

    #[test]
    fn parsed_multipart_files_no_equals_filtered() {
        let cli = Cli::try_parse_from(["hpx", "--multipart-file", "noequals"]).unwrap();
        let parsed = cli.parsed_multipart_files();
        assert!(parsed.is_empty());
    }

    // ── Cli::parse flag combinations ──

    #[test]
    fn cli_minimal_url_only() {
        let cli = Cli::try_parse_from(["hpx", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.url.as_deref(), Some("httpbin.org/get"));
        assert_eq!(cli.method, Method::Get);
        assert!(cli.headers.is_empty());
        assert!(cli.data.is_none());
        assert!(cli.json.is_none());
        assert!(cli.output.is_none());
        assert!(!cli.follow);
        assert!(!cli.silent);
        assert!(!cli.dry_run);
        assert_eq!(cli.verbose, 0);
        assert_eq!(cli.retry, 0);
    }

    #[test]
    fn cli_method_post_with_data() {
        let cli = Cli::try_parse_from([
            "hpx",
            "-X",
            "POST",
            "-d",
            "{\"key\":\"value\"}",
            "httpbin.org/post",
        ])
        .unwrap();
        assert_eq!(cli.method, Method::Post);
        assert_eq!(cli.data.as_deref(), Some("{\"key\":\"value\"}"));
    }

    #[test]
    fn cli_method_put() {
        let cli = Cli::try_parse_from(["hpx", "-X", "PUT", "httpbin.org/put"]).unwrap();
        assert_eq!(cli.method, Method::Put);
    }

    #[test]
    fn cli_method_delete() {
        let cli = Cli::try_parse_from(["hpx", "-X", "DELETE", "httpbin.org/delete"]).unwrap();
        assert_eq!(cli.method, Method::Delete);
    }

    #[test]
    fn cli_method_patch() {
        let cli = Cli::try_parse_from(["hpx", "-X", "PATCH", "httpbin.org/patch"]).unwrap();
        assert_eq!(cli.method, Method::Patch);
    }

    #[test]
    fn cli_method_head() {
        let cli = Cli::try_parse_from(["hpx", "-X", "HEAD", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.method, Method::Head);
    }

    #[test]
    fn cli_method_options() {
        let cli = Cli::try_parse_from(["hpx", "-X", "OPTIONS", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.method, Method::Options);
    }

    #[test]
    fn cli_verbose_count() {
        let cli = Cli::try_parse_from(["hpx", "-v", "-v", "-v", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.verbose, 3);
    }

    #[test]
    fn cli_all_flags_combined() {
        let cli = Cli::try_parse_from([
            "hpx",
            "-X",
            "POST",
            "-H",
            "Content-Type:application/json",
            "-H",
            "Authorization:Bearer token",
            "-d",
            "{\"data\":1}",
            "-o",
            "response.json",
            "-t",
            "30",
            "-L",
            "-v",
            "--format",
            "json",
            "--color",
            "off",
            "httpbin.org/post",
        ])
        .unwrap();
        assert_eq!(cli.method, Method::Post);
        assert_eq!(cli.headers.len(), 2);
        assert_eq!(cli.data.as_deref(), Some("{\"data\":1}"));
        assert_eq!(cli.output.as_deref(), Some("response.json"));
        assert_eq!(cli.timeout, Some(30.0));
        assert!(cli.follow);
        assert_eq!(cli.verbose, 1);
        assert_eq!(cli.format, OutputFormat::Json);
    }

    #[test]
    fn cli_json_data() {
        let cli =
            Cli::try_parse_from(["hpx", "-j", "{\"name\":\"test\"}", "httpbin.org/post"]).unwrap();
        assert_eq!(cli.json.as_deref(), Some("{\"name\":\"test\"}"));
    }

    #[test]
    fn cli_basic_auth() {
        let cli = Cli::try_parse_from(["hpx", "--basic", "user:pass", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.basic.as_deref(), Some("user:pass"));
    }

    #[test]
    fn cli_bearer_auth() {
        let cli = Cli::try_parse_from(["hpx", "--bearer", "mytoken", "httpbin.org/get"]).unwrap();
        assert_eq!(cli.bearer.as_deref(), Some("mytoken"));
    }

    #[test]
    fn cli_dry_run() {
        let cli = Cli::try_parse_from(["hpx", "--dry-run", "httpbin.org/get"]).unwrap();
        assert!(cli.dry_run);
    }

    #[test]
    fn cli_clipboard() {
        let cli = Cli::try_parse_from(["hpx", "--clipboard", "httpbin.org/get"]).unwrap();
        assert!(cli.clipboard);
    }

    #[test]
    fn cli_timing() {
        let cli = Cli::try_parse_from(["hpx", "-T", "httpbin.org/get"]).unwrap();
        assert!(cli.timing);
    }

    #[test]
    fn cli_silent() {
        let cli = Cli::try_parse_from(["hpx", "-s", "httpbin.org/get"]).unwrap();
        assert!(cli.silent);
    }

    #[test]
    fn cli_form_multiple() {
        let cli = Cli::try_parse_from([
            "hpx",
            "-f",
            "name=alice",
            "-f",
            "age=30",
            "-f",
            "city=nyc",
            "httpbin.org/post",
        ])
        .unwrap();
        assert_eq!(cli.form.len(), 3);
    }

    #[test]
    fn cli_multipart_multiple() {
        let cli = Cli::try_parse_from([
            "hpx",
            "--multipart",
            "field1=val1",
            "--multipart",
            "field2=val2",
            "httpbin.org/post",
        ])
        .unwrap();
        assert_eq!(cli.multipart.len(), 2);
    }

    #[test]
    fn cli_multipart_file_multiple() {
        let cli = Cli::try_parse_from([
            "hpx",
            "--multipart-file",
            "avatar=@/tmp/photo.png",
            "--multipart-file",
            "doc=@/tmp/doc.pdf",
            "httpbin.org/post",
        ])
        .unwrap();
        assert_eq!(cli.multipart_file.len(), 2);
    }

    #[test]
    fn cli_cookie_multiple() {
        let cli = Cli::try_parse_from([
            "hpx",
            "--cookie",
            "session=abc",
            "--cookie",
            "theme=dark",
            "httpbin.org/get",
        ])
        .unwrap();
        assert_eq!(cli.cookie.len(), 2);
    }

    #[test]
    fn cli_cookie_jar() {
        let cli =
            Cli::try_parse_from(["hpx", "--cookie-jar", "/tmp/cookies.txt", "httpbin.org/get"])
                .unwrap();
        assert_eq!(cli.cookie_jar.as_deref(), Some("/tmp/cookies.txt"));
    }

    #[test]
    fn cli_proxy_flag() {
        let cli = Cli::try_parse_from(["hpx", "--proxy", "http://proxy:8080", "httpbin.org/get"])
            .unwrap();
        assert_eq!(cli.proxy.as_deref(), Some("http://proxy:8080"));
    }

    #[test]
    fn cli_basic_with_colon_in_password() {
        let cli =
            Cli::try_parse_from(["hpx", "--basic", "user:pass:with:colons", "httpbin.org/get"])
                .unwrap();
        assert_eq!(cli.basic.as_deref(), Some("user:pass:with:colons"));
    }

    #[test]
    fn cli_reconnect_with_max() {
        let cli = Cli::try_parse_from([
            "hpx",
            "--reconnect",
            "--reconnect-max",
            "10",
            "ws://localhost",
        ])
        .unwrap();
        assert!(cli.reconnect);
        assert_eq!(cli.reconnect_max, 10);
    }

    #[test]
    fn cli_dl_add_all_options() {
        let cli = Cli::try_parse_from([
            "hpx",
            "dl",
            "add",
            "https://example.com/file.bin",
            "-o",
            "file.bin",
            "--priority",
            "high",
            "--speed-limit",
            "1MB/s",
            "--checksum",
            "sha256:abc123",
            "--mirror",
            "https://mirror1.example.com/file.bin",
            "--mirror",
            "https://mirror2.example.com/file.bin",
            "--max-connections",
            "4",
            "-H",
            "Authorization:Bearer token",
            "--proxy",
            "http://proxy:8080",
            "--retry",
            "3",
        ])
        .unwrap();
        match cli.command.unwrap() {
            Commands::Dl(DlCommands::Add {
                url,
                output,
                priority,
                speed_limit,
                checksum,
                mirrors,
                max_connections,
                headers,
                proxy,
                retry,
            }) => {
                assert_eq!(url, "https://example.com/file.bin");
                assert_eq!(output.as_deref(), Some("file.bin"));
                assert_eq!(priority, "high");
                assert_eq!(speed_limit.as_deref(), Some("1MB/s"));
                assert_eq!(checksum.as_deref(), Some("sha256:abc123"));
                assert_eq!(mirrors.len(), 2);
                assert_eq!(max_connections, Some(4));
                assert_eq!(headers.len(), 1);
                assert_eq!(proxy.as_deref(), Some("http://proxy:8080"));
                assert_eq!(retry, Some(3));
            }
            _ => panic!("expected DlCommands::Add"),
        }
    }

    #[test]
    fn download_status_json_serialization() {
        use hpx_dl::{DownloadId, DownloadPriority, DownloadState, DownloadStatus};

        let id = DownloadId::from(
            uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        );
        let status = DownloadStatus {
            id,
            url: "https://example.com/file.bin".to_string(),
            state: DownloadState::Downloading,
            bytes_downloaded: 1024,
            total_bytes: Some(4096),
            priority: DownloadPriority::Normal,
        };

        let json = serde_json::to_string(&status).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["id"], "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(parsed["url"], "https://example.com/file.bin");
        assert_eq!(parsed["state"], "Downloading");
        assert_eq!(parsed["bytes_downloaded"], 1024);
        assert_eq!(parsed["total_bytes"], 4096);
        assert_eq!(parsed["priority"], "Normal");
    }

    #[test]
    fn download_status_json_array_serialization() {
        use hpx_dl::{DownloadId, DownloadPriority, DownloadState, DownloadStatus};

        let id1 = DownloadId::from(
            uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        );
        let id2 = DownloadId::from(
            uuid::Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap(),
        );
        let statuses = vec![
            DownloadStatus {
                id: id1,
                url: "https://example.com/a.bin".to_string(),
                state: DownloadState::Queued,
                bytes_downloaded: 0,
                total_bytes: None,
                priority: DownloadPriority::Low,
            },
            DownloadStatus {
                id: id2,
                url: "https://example.com/b.bin".to_string(),
                state: DownloadState::Completed,
                bytes_downloaded: 2048,
                total_bytes: Some(2048),
                priority: DownloadPriority::High,
            },
        ];

        let json = serde_json::to_string(&statuses).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_array());
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["state"], "Queued");
        assert_eq!(arr[1]["state"], "Completed");
        assert_eq!(arr[0]["total_bytes"], serde_json::Value::Null);
        assert_eq!(arr[1]["total_bytes"], 2048);
    }
}
