use clap::{ArgAction, Parser, Subcommand, ValueEnum};

/// HTTP method to use for requests.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
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
    List,
    /// Show download status.
    Status {
        /// Download ID.
        id: String,
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
    #[arg(short = 'X', long = "method", value_enum, default_value_t)]
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
    #[arg(short = 't', long, value_name = "SECONDS")]
    pub timeout: Option<f64>,

    /// Maximum number of redirects.
    #[arg(long, value_name = "NUM")]
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
    #[arg(short = 'L', long)]
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
    #[arg(long, value_name = "URL")]
    pub proxy: Option<String>,

    /// Number of retries on transient errors.
    #[arg(long, value_name = "NUM", default_value = "0")]
    pub retry: u32,

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
        self.headers
            .iter()
            .filter_map(|h| {
                let (name, value) = h.split_once(':')?;
                Some((name.trim().to_string(), value.trim().to_string()))
            })
            .collect()
    }

    /// Parse --form fields into (key, value) pairs.
    pub(crate) fn parsed_form_fields(&self) -> Vec<(String, String)> {
        self.form
            .iter()
            .filter_map(|f| {
                let (key, value) = f.split_once('=')?;
                Some((key.to_string(), value.to_string()))
            })
            .collect()
    }

    /// Parse --multipart fields into (key, value) pairs.
    pub(crate) fn parsed_multipart_fields(&self) -> Vec<(String, String)> {
        self.multipart
            .iter()
            .filter_map(|f| {
                let (key, value) = f.split_once('=')?;
                Some((key.to_string(), value.to_string()))
            })
            .collect()
    }

    /// Parse --multipart-file fields into (key, path) pairs.
    pub(crate) fn parsed_multipart_files(&self) -> Vec<(String, String)> {
        self.multipart_file
            .iter()
            .filter_map(|f| {
                let (key, value) = f.split_once('=')?;
                Some((key.to_string(), value.strip_prefix('@')?.to_string()))
            })
            .collect()
    }

    /// Parse --cookie values into (name, value) pairs.
    pub(crate) fn parsed_cookies(&self) -> Vec<(String, String)> {
        self.cookie
            .iter()
            .filter_map(|c| {
                let (name, value) = c.split_once('=')?;
                Some((name.trim().to_string(), value.trim().to_string()))
            })
            .collect()
    }
}
