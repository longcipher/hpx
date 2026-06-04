# hpx-cli HTTP & WebSocket Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform `bin/hpx-cli` from a download-only CLI into a modern HTTP client (like `curl`/`fetch`) with WebSocket support, using the `hpx` library as the backend.

**Architecture:** Top-level CLI where `hpx <url>` makes an HTTP request directly (fetch-style). WebSocket detected via `ws://`/`wss://` URL scheme. `dl` subcommand retained for download management. Modular source layout: `cli.rs` for arg parsing, `http.rs` for HTTP execution, `ws.rs` for WebSocket, `output.rs` for formatting.

**Tech Stack:** `clap` (derive), `hpx` (HTTP/WS client), `tokio` (async runtime), `serde_json` (JSON formatting), `color-print` (ANSI colors)

---

## File Structure

```text
bin/hpx-cli/
├── Cargo.toml          # Add hpx, tokio, serde_json, color-print deps
└── src/
    ├── main.rs         # Entry point, runtime setup, dispatch
    ├── cli.rs          # Cli struct, all CLI flags, method/body parsing
    ├── http.rs         # HTTP request execution via hpx
    ├── ws.rs           # WebSocket execution via hpx
    └── output.rs       # Response formatting, color output, status printing
```

---

### Task 1: Update Cargo.toml with new dependencies

**Files:**

- Modify: `bin/hpx-cli/Cargo.toml`

- [ ] **Step 1: Update Cargo.toml**

Replace `bin/hpx-cli/Cargo.toml` contents:

```toml
[package]
name = "hpx-cli"
version.workspace = true
description = "CLI for hpx — high-performance HTTP client and download engine"
repository.workspace = true
authors.workspace = true
license.workspace = true
edition.workspace = true

[[bin]]
name = "hpx"
path = "src/main.rs"
doc = false

[dependencies]
clap = { workspace = true, features = ["derive"] }
color-print = "0.3"
eyre = { workspace = true }
futures-util = { workspace = true }
hpx = { workspace = true, features = ["http1", "http2", "ws", "json", "stream", "tracing", "boring"] }
hpx-dl = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "io-std", "io-util", "signal", "fs"] }
tracing-subscriber = { workspace = true }
url = { workspace = true }
uuid = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p hpx-cli`
Expected: Compiles (may have warnings about unused imports in existing main.rs)

---

### Task 2: Create CLI argument definitions

**Files:**

- Create: `bin/hpx-cli/src/cli.rs`

- [ ] **Step 1: Create cli.rs with all CLI structures**

```rust
use clap::{ArgAction, Parser, ValueEnum};

/// HTTP method to use for requests.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
#[value(rename_all = "UPPER")]
pub enum Method {
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
    pub fn as_str(self) -> &'static str {
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
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Auto,
    Json,
    Text,
    Raw,
}

/// Color mode for terminal output.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum ColorMode {
    #[default]
    Auto,
    On,
    Off,
}

/// High-performance HTTP client and download engine.
#[derive(Debug, Parser)]
#[command(
    name = "hpx",
    about = "High-performance HTTP client and download engine",
    disable_help_flag = true,
    after_help = "Examples:\n  hpx httpbin.org/get\n  hpx -X POST httpbin.org/post -d '{\"key\":\"value\"}'\n  hpx -H 'Authorization: Bearer token' https://api.example.com\n  hpx ws://echo.websocket.org"
)]
pub struct Cli {
    /// The URL to make a request to.
    pub url: Option<String>,

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

    /// Print timing information.
    #[arg(short = 'T', long = "timing")]
    pub timing: bool,

    /// Print version.
    #[arg(short = 'V', long)]
    pub version: bool,

    /// Print help.
    #[arg(short = 'h', long)]
    pub help: bool,
}

impl Cli {
    /// Returns true if the URL is a WebSocket URL.
    pub fn is_websocket_url(&self) -> bool {
        self.url
            .as_deref()
            .map(|u| {
                let lower = u.to_ascii_lowercase();
                lower.starts_with("ws://") || lower.starts_with("wss://")
            })
            .unwrap_or(false)
    }

    /// Parse headers into (name, value) pairs.
    pub fn parsed_headers(&self) -> Vec<(String, String)> {
        self.headers
            .iter()
            .filter_map(|h| {
                let (name, value) = h.split_once(':')?;
                Some((name.trim().to_string(), value.trim().to_string()))
            })
            .collect()
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p hpx-cli`
Expected: Compiles with cli.rs (main.rs still has old code, will be updated next)

---

### Task 3: Create output formatting module

**Files:**

- Create: `bin/hpx-cli/src/output.rs`

- [ ] **Step 1: Create output.rs with formatting utilities**

```rust
use std::io::{self, Write};

use crate::cli::ColorMode;

pub fn use_color(mode: ColorMode, is_terminal: bool) -> bool {
    match mode {
        ColorMode::On => true,
        ColorMode::Off => false,
        ColorMode::Auto => is_terminal,
    }
}

pub fn print_status_line(
    status: u16,
    version: &str,
    color: bool,
) {
    let mut stderr = io::stderr().lock();
    let _ = if color {
        let code = status_color_code(status);
        write!(
            stderr,
            "\x1b[{code}m{status}\x1b[0m {version}\n"
        )
    } else {
        write!(stderr, "{status} {version}\n")
    };
}

pub fn print_headers(
    headers: &[(String, String)],
    verbose: bool,
    color: bool,
) {
    let mut stderr = io::stderr().lock();
    for (name, value) in headers {
        let _ = if color && verbose {
            write!(
                stderr,
                "\x1b[1;36m{name}\x1b[0m: {value}\n"
            )
        } else {
            write!(stderr, "{name}: {value}\n")
        };
    }
}

pub fn print_request_line(method: &str, url: &str, color: bool) {
    let mut stderr = io::stderr().lock();
    let _ = if color {
        write!(
            stderr,
            "\x1b[1;33m{method}\x1b[0m \x1b[1;36m{url}\x1b[0m\n"
        )
    } else {
        write!(stderr, "{method} {url}\n")
    };
}

pub fn write_body(body: &[u8], output: Option<&str>) -> eyre::Result<()> {
    if let Some(path) = output {
        std::fs::write(path, body)?;
    } else {
        let mut stdout = io::stdout().lock();
        stdout.write_all(body)?;
    }
    Ok(())
}

pub fn format_json_pretty(body: &[u8]) -> eyre::Result<String> {
    let value: serde_json::Value = serde_json::from_slice(body)?;
    Ok(serde_json::to_string_pretty(&value)?)
}

pub fn is_terminal() -> bool {
    std::io::stdout().is_terminal()
}

fn status_color_code(status: u16) -> u8 {
    match status {
        200..=299 => 32, // green
        300..=399 => 33, // yellow
        400..=499 => 31, // red
        500..=599 => 31, // red
        _ => 0,
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p hpx-cli`

---

### Task 4: Create HTTP execution module

**Files:**

- Create: `bin/hpx-cli/src/http.rs`

- [ ] **Step 1: Create http.rs with HTTP request execution**

```rust
use std::time::Duration;

use eyre::{Result, WrapErr};
use hpx::header::{self, HeaderValue};

use crate::cli::{Cli, Method};
use crate::output;

pub async fn execute(cli: &Cli) -> Result<i32> {
    let url = cli
        .url
        .as_deref()
        .ok_or_else(|| eyre::eyre!("URL is required"))?;

    let method = http::Method::from_bytes(cli.method.as_str().as_bytes())
        .wrap_err("invalid HTTP method")?;

    let client = hpx::Client::new();
    let mut builder = client.request(method, url);

    // Apply headers
    for (name, value) in cli.parsed_headers() {
        builder = builder.header(name.as_str(), value.as_str());
    }

    // Apply basic auth
    if let Some(ref credentials) = cli.basic {
        let (user, pass) = credentials
            .split_once(':')
            .ok_or_else(|| eyre::eyre!("basic auth must be USER:PASS"))?;
        let encoded = base64_encode(format!("{user}:{pass}"));
        builder = builder.header(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Basic {encoded}"))
                .wrap_err("invalid basic auth header")?,
        );
    }

    // Apply bearer auth
    if let Some(ref token) = cli.bearer {
        builder = builder.header(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .wrap_err("invalid bearer auth header")?,
        );
    }

    // Apply timeout
    if let Some(seconds) = cli.timeout {
        builder = builder.timeout(Duration::from_secs_f64(seconds));
    }

    // Apply body
    if let Some(ref data) = cli.json {
        let body = if let Some(path) = data.strip_prefix('@') {
            std::fs::read(path).wrap_err("failed to read JSON file")?
        } else {
            data.as_bytes().to_vec()
        };
        builder = builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(body);
    } else if let Some(ref data) = cli.data {
        let body = if let Some(path) = data.strip_prefix('@') {
            std::fs::read(path).wrap_err("failed to read data file")?
        } else {
            data.as_bytes().to_vec()
        };
        builder = builder.body(body);
    }

    // Dry run
    if cli.dry_run {
        if !cli.silent {
            output::print_request_line(cli.method.as_str(), url, output::use_color(cli.color, output::is_terminal()));
            for (name, value) in cli.parsed_headers() {
                eprintln!("  {name}: {value}");
            }
        }
        return Ok(0);
    }

    let response = builder.send().await.wrap_err("request failed")?;

    let status = response.status();
    let status_code = status.as_u16();

    // Print response headers in verbose mode
    if cli.verbose >= 1 && !cli.silent {
        let color = output::use_color(cli.color, output::is_terminal());
        let version = format!("{:?}", response.version());
        output::print_status_line(status_code, &version, color);

        let resp_headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value.to_str().ok().map(|v| (name.to_string(), v.to_string()))
            })
            .collect();
        output::print_headers(&resp_headers, cli.verbose >= 2, color);
    }

    // Read response body
    let body = response.bytes().await.wrap_err("failed to read response body")?;

    // Format and output body
    let stdout_is_terminal = output::is_terminal();
    let formatted = match cli.format {
        crate::cli::OutputFormat::Raw => body.to_vec(),
        crate::cli::OutputFormat::Text => body.to_vec(),
        crate::cli::OutputFormat::Json => {
            match output::format_json_pretty(&body) {
                Ok(s) => s.into_bytes(),
                Err(_) => body.to_vec(),
            }
        }
        crate::cli::OutputFormat::Auto => {
            if stdout_is_terminal && looks_like_json(&body) {
                match output::format_json_pretty(&body) {
                    Ok(s) => s.into_bytes(),
                    Err(_) => body.to_vec(),
                }
            } else {
                body.to_vec()
            }
        }
    };

    output::write_body(&formatted, cli.output.as_deref())?;

    // Exit with error status for 4xx/5xx
    if status_code >= 400 {
        std::process::exit(status_code as i32);
    }

    Ok(0)
}

fn looks_like_json(body: &[u8]) -> bool {
    let trimmed = body.iter().position(|&b| !b.is_ascii_whitespace()).map(|i| &body[i..]);
    matches!(trimmed, Some(b"{" | b"["))
}

fn base64_encode(input: String) -> String {
    use std::io::Write;
    let mut engine = base64::engine::general_purpose::STANDARD;
    let mut buf = Vec::new();
    // Simple base64 encode without extra dependency
    // Use a minimal implementation
    base64_encode_inner(&input.as_bytes(), &mut buf);
    String::from_utf8(buf).unwrap_or_default()
}

fn base64_encode_inner(input: &[u8], output: &mut Vec<u8>) {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        output.push(CHARS[((triple >> 18) & 0x3F) as usize]);
        output.push(CHARS[((triple >> 12) & 0x3F) as usize]);
        if chunk.len() > 1 {
            output.push(CHARS[((triple >> 6) & 0x3F) as usize]);
        } else {
            output.push(b'=');
        }
        if chunk.len() > 2 {
            output.push(CHARS[(triple & 0x3F) as usize]);
        } else {
            output.push(b'=');
        }
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p hpx-cli`

---

### Task 5: Create WebSocket execution module

**Files:**

- Create: `bin/hpx-cli/src/ws.rs`

- [ ] **Step 1: Create ws.rs with WebSocket support**

```rust
use std::io::{self, BufRead, Read, Write};

use eyre::{Result, WrapErr};
use futures_util::{SinkExt, StreamExt};
use hpx::header::{self, HeaderValue};
use hpx::ws::message::Message;

use crate::cli::Cli;
use crate::output;

pub async fn execute(cli: &Cli) -> Result<i32> {
    let url = cli
        .url
        .as_deref()
        .ok_or_else(|| eyre::eyre!("WebSocket URL is required"))?;

    let client = hpx::Client::new();
    let mut builder = client.websocket(url);

    // Apply headers
    for (name, value) in cli.parsed_headers() {
        builder = builder.header(name.as_str(), value.as_str());
    }

    // Apply basic auth
    if let Some(ref credentials) = cli.basic {
        let (user, pass) = credentials
            .split_once(':')
            .ok_or_else(|| eyre::eyre!("basic auth must be USER:PASS"))?;
        let encoded = base64_encode(format!("{user}:{pass}"));
        builder = builder.header(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Basic {encoded}"))
                .wrap_err("invalid basic auth header")?,
        );
    }

    // Apply bearer auth
    if let Some(ref token) = cli.bearer {
        builder = builder.header(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .wrap_err("invalid bearer auth header")?,
        );
    }

    // Dry run
    if cli.dry_run {
        if !cli.silent {
            let color = output::use_color(cli.color, output::is_terminal());
            output::print_request_line("GET", url, color);
            for (name, value) in cli.parsed_headers() {
                eprintln!("  {name}: {value}");
            }
        }
        return Ok(0);
    }

    let response = builder.send().await.wrap_err("WebSocket handshake failed")?;

    let ws = response.into_websocket().await.wrap_err("WebSocket upgrade failed")?;

    let (mut tx, mut rx) = ws.split();

    // Send initial message if provided via --data or --json
    let initial_message = cli
        .json
        .as_deref()
        .or(cli.data.as_deref())
        .map(|s| {
            if let Some(path) = s.strip_prefix('@') {
                std::fs::read(path).ok()
            } else {
                Some(s.as_bytes().to_vec())
            }
        })
        .flatten();

    if let Some(msg) = initial_message {
        let ws_msg = if String::from_utf8(msg.clone()).is_ok() {
            Message::Text(String::from_utf8(msg).unwrap())
        } else {
            Message::Binary(msg.into())
        };
        tx.send(ws_msg).await.wrap_err("failed to send initial message")?;
    }

    // If stdin is a terminal, enable interactive mode (read from stdin line by line)
    let stdin_is_terminal = io::stdin().is_terminal();

    if stdin_is_terminal {
        // Interactive mode: read lines from stdin, send as text messages
        let color = output::use_color(cli.color, output::is_terminal());
        if !cli.silent && color {
            eprintln!("\x1b[2mConnected to {url}. Type messages and press Enter.\x1b[0m");
        } else if !cli.silent {
            eprintln!("Connected to {url}. Type messages and press Enter.");
        }

        let stdin = io::stdin();
        let mut reader = stdin.lock();
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Err(e) = tx.send(Message::Text(trimmed.to_string())).await {
                        eprintln!("Send error: {e}");
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("Read error: {e}");
                    break;
                }
            }
        }
    } else {
        // Non-interactive: read from stdin (piped input)
        let mut stdin = io::stdin().lock();
        let mut buf = Vec::new();
        stdin.read_to_end(&mut buf)?;

        if !buf.is_empty() {
            let msg = if String::from_utf8(buf.clone()).is_ok() {
                Message::Text(String::from_utf8(buf).unwrap())
            } else {
                Message::Binary(buf.into())
            };
            tx.send(msg).await.wrap_err("failed to send message")?;
        }
    }

    // Receive messages until connection closes
    let color = output::use_color(cli.color, output::is_terminal());
    while let Some(msg) = rx.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if cli.format == crate::cli::OutputFormat::Raw {
                    print!("{text}");
                } else if color && output::looks_like_json_str(&text) {
                    match output::format_json_pretty(text.as_bytes()) {
                        Ok(pretty) => print!("{pretty}\n"),
                        Err(_) => println!("{text}"),
                    }
                } else {
                    println!("{text}");
                }
            }
            Ok(Message::Binary(bytes)) => {
                if cli.output.is_some() {
                    output::write_body(&bytes, cli.output.as_deref())?;
                } else if io::stdout().is_terminal() && !output::bytes_appear_printable(&bytes) {
                    if !cli.silent {
                        eprintln!("[binary message: {} bytes]", bytes.len());
                    }
                } else {
                    io::stdout().write_all(&bytes)?;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("Receive error: {e}");
                break;
            }
        }
    }

    Ok(0)
}

fn base64_encode(input: String) -> String {
    let mut buf = Vec::new();
    base64_encode_inner(input.as_bytes(), &mut buf);
    String::from_utf8(buf).unwrap_or_default()
}

fn base64_encode_inner(input: &[u8], output: &mut Vec<u8>) {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        output.push(CHARS[((triple >> 18) & 0x3F) as usize]);
        output.push(CHARS[((triple >> 12) & 0x3F) as usize]);
        if chunk.len() > 1 {
            output.push(CHARS[((triple >> 6) & 0x3F) as usize]);
        } else {
            output.push(b'=');
        }
        if chunk.len() > 2 {
            output.push(CHARS[(triple & 0x3F) as usize]);
        } else {
            output.push(b'=');
        }
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p hpx-cli`

---

### Task 6: Update output.rs with additional helpers

**Files:**

- Modify: `bin/hpx-cli/src/output.rs`

- [ ] **Step 1: Add missing helper functions**

Add these functions to `output.rs`:

```rust
pub fn looks_like_json_str(s: &str) -> bool {
    let trimmed = s.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

pub fn bytes_appear_printable(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return true;
    }
    let preview = if bytes.len() > 1024 { &bytes[..1024] } else { bytes };
    if preview.contains(&0) {
        return false;
    }
    let printable = preview
        .iter()
        .filter(|&&b| b >= 0x20 || b == b'\n' || b == b'\r' || b == b'\t')
        .count();
    printable as f64 / preview.len() as f64 >= 0.9
}
```

---

### Task 7: Rewrite main.rs as entry point and dispatcher

**Files:**

- Modify: `bin/hpx-cli/src/main.rs`

- [ ] **Step 1: Rewrite main.rs**

Replace the entire contents of `bin/hpx-cli/src/main.rs`:

```rust
//! `hpx` — CLI for high-performance HTTP client and download engine.

#![allow(clippy::print_stdout)]
#![allow(clippy::print_stderr)]
#![allow(missing_docs)]

mod cli;
mod http;
mod output;
mod ws;

use clap::Parser;
use cli::Cli;

fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    if cli.version {
        println!("hpx {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if cli.help {
        let mut cmd = Cli::command();
        cmd.print_help()?;
        return Ok(());
    }

    // Validate URL is provided
    if cli.url.is_none() {
        let mut cmd = Cli::command();
        cmd.print_help()?;
        eprintln!("\nError: URL is required");
        std::process::exit(1);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let code = if cli.is_websocket_url() {
        runtime.block_on(ws::execute(&cli))
    } else {
        runtime.block_on(http::execute(&cli))
    };

    match code {
        Ok(0) => Ok(()),
        Ok(n) => std::process::exit(n),
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p hpx-cli`
Expected: Compiles successfully

---

### Task 8: Add `dl` subcommand back (download manager)

**Files:**

- Modify: `bin/hpx-cli/src/cli.rs`
- Modify: `bin/hpx-cli/src/main.rs`
- [ ] **Step 1: Add DlCommands to cli.rs**

Add to the end of `cli.rs`:

```rust
/// Download management commands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Download management commands.
    #[command(subcommand)]
    Dl(DlCommands),
}

#[derive(Debug, Subcommand)]
pub enum DlCommands {
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
```

Update the `Cli` struct to add optional subcommand:

```rust
pub struct Cli {
    /// The URL to make a request to (for direct HTTP/WS requests).
    pub url: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,

    // ... rest of fields
}
```

- [ ] **Step 2: Update main.rs to handle dl subcommand**

Update the main dispatch in `main.rs`:

```rust
fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    if cli.version {
        println!("hpx {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if cli.help {
        let mut cmd = Cli::command();
        cmd.print_help()?;
        return Ok(());
    }

    // Handle dl subcommand
    if let Some(cli::Commands::Dl(dl_cmd)) = cli.command {
        return handle_dl_command(dl_cmd);
    }

    // Validate URL is provided for HTTP/WS
    if cli.url.is_none() {
        let mut cmd = Cli::command();
        cmd.print_help()?;
        eprintln!("\nError: URL is required");
        std::process::exit(1);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let code = if cli.is_websocket_url() {
        runtime.block_on(ws::execute(&cli))
    } else {
        runtime.block_on(http::execute(&cli))
    };

    match code {
        Ok(0) => Ok(()),
        Ok(n) => std::process::exit(n),
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}

fn handle_dl_command(cmd: cli::DlCommands) -> eyre::Result<()> {
    let engine = hpx_dl::DownloadEngine::builder().build()?;

    match cmd {
        cli::DlCommands::Add { url, output, priority } => {
            let destination = output.unwrap_or_else(|| {
                url.split('/').next_back().unwrap_or("download").to_string()
            });
            let priority = parse_priority(&priority)?;
            let request = hpx_dl::DownloadRequest::builder(&url, &destination)
                .priority(priority)
                .build();
            let id = engine.add(request)?;
            println!("Added download {id}");
        }
        cli::DlCommands::Pause { id } => {
            let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
            engine.pause(download_id)?;
            println!("Paused {download_id}");
        }
        cli::DlCommands::Resume { id } => {
            let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
            engine.resume(download_id)?;
            println!("Resumed {download_id}");
        }
        cli::DlCommands::Remove { id } => {
            let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
            engine.remove(download_id)?;
            println!("Removed {download_id}");
        }
        cli::DlCommands::List => {
            let downloads = engine.list()?;
            if downloads.is_empty() {
                println!("No downloads.");
            } else {
                for status in &downloads {
                    println!(
                        "{}  {}  {}  {}/{} bytes  {:?}",
                        status.id,
                        status.state,
                        status.url,
                        status.bytes_downloaded,
                        status
                            .total_bytes
                            .map_or_else(|| "?".to_string(), |t| t.to_string()),
                        status.priority,
                    );
                }
            }
        }
        cli::DlCommands::Status { id } => {
            let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
            let status = engine.status(download_id)?;
            println!(
                "ID:       {}\nURL:      {}\nState:    {}\nProgress: {}/{} bytes\nPriority: {:?}",
                status.id,
                status.url,
                status.state,
                status.bytes_downloaded,
                status
                    .total_bytes
                    .map_or_else(|| "?".to_string(), |t| t.to_string()),
                status.priority,
            );
        }
    }

    Ok(())
}

fn parse_priority(s: &str) -> eyre::Result<hpx_dl::DownloadPriority> {
    match s.to_lowercase().as_str() {
        "low" => Ok(hpx_dl::DownloadPriority::Low),
        "normal" => Ok(hpx_dl::DownloadPriority::Normal),
        "high" => Ok(hpx_dl::DownloadPriority::High),
        "critical" => Ok(hpx_dl::DownloadPriority::Critical),
        other => Err(eyre::eyre!(
            "unknown priority '{other}', expected one of: low, normal, high, critical"
        )),
    }
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p hpx-cli`
Expected: Compiles successfully

---

### Task 9: Fix compilation issues and refine

**Files:**

- Various

- [ ] **Step 1: Build and fix any errors**

Run: `cargo build -p hpx-cli 2>&1`
Fix any compilation errors that arise from API mismatches.

Common issues to watch for:

- `hpx::Client::new()` API
- `hpx::websocket()` API
- `response.bytes()` vs `response.text()` method names
- Feature flags for hpx crate
- Import paths for ws module
- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p hpx-cli -- -D warnings`
Fix any lint warnings.

- [ ] **Step 3: Run tests**

Run: `cargo test -p hpx-cli`
Ensure all tests pass (if any).

---

### Task 10: Manual smoke test

- [ ] **Step 1: Build the binary**

Run: `cargo build -p hpx-cli`

- [ ] **Step 2: Test basic HTTP GET**

Run: `./target/debug/hpx httpbin.org/get`
Expected: Returns JSON response body

- [ ] **Step 3: Test with verbose flag**

Run: `./target/debug/hpx -v httpbin.org/get`
Expected: Shows status line and headers, then response body

- [ ] **Step 4: Test POST with JSON body**

Run: `./target/debug/hpx -X POST httpbin.org/post -j '{"hello":"world"}'`
Expected: Returns posted JSON

- [ ] **Step 5: Test WebSocket**

Run: `echo "hello" | ./target/debug/hpx ws://echo.websocket.org`
Expected: Receives echoed message

- [ ] **Step 6: Test dl subcommand**

Run: `./target/debug/hpx dl list`
Expected: Lists downloads (empty or existing)

- [ ] **Step 7: Test dry run**

Run: `./target/debug/hpx --dry-run httpbin.org/get`
Expected: Prints request info without sending

---

### Task 11: Final cleanup and commit

- [ ] **Step 1: Remove unused imports and dead code warnings**

Run: `cargo build -p hpx-cli 2>&1 | grep warning`
Fix any remaining warnings.

- [ ] **Step 2: Format code**

Run: `cargo fmt -p hpx-cli`

- [ ] **Step 3: Final verification**

Run: `cargo check -p hpx-cli && cargo clippy -p hpx-cli -- -D warnings`
Expected: Clean build with no warnings

- [ ] **Step 4: Commit**

```bash
git add bin/hpx-cli/
git commit -m "feat(hpx-cli): add HTTP client and WebSocket support

Transform hpx-cli from a download-only tool into a modern HTTP client
inspired by fetch. Supports HTTP/1.1, HTTP/2, WebSocket, JSON formatting,
color output, basic/bearer auth, and retains the dl subcommand for
download management."
```
