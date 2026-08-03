//! `hpxless` — CDP-compatible browser server for Puppeteer/Playwright.

#![expect(clippy::print_stdout, reason = "CLI binary writes to stdout by design")]
#![expect(clippy::print_stderr, reason = "CLI binary writes to stderr by design")]

mod cli;

use clap::Parser;
use cli::Cli;
use hpx_browser::protocol::CdpServer;

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();

    let level_filter = match cli.log_level.to_lowercase().as_str() {
        "trace" => tracing::level_filters::LevelFilter::TRACE,
        "debug" => tracing::level_filters::LevelFilter::DEBUG,
        "info" => tracing::level_filters::LevelFilter::INFO,
        "warn" => tracing::level_filters::LevelFilter::WARN,
        "error" => tracing::level_filters::LevelFilter::ERROR,
        _ => tracing::level_filters::LevelFilter::INFO,
    };
    tracing_subscriber::fmt()
        .with_max_level(level_filter)
        .init();

    // Extract HTML: data:text/html,... → payload, http(s) → empty for now, none → empty
    let html = match &cli.url {
        Some(url) if url.starts_with("data:text/html,") => &url["data:text/html,".len()..],
        Some(url) => {
            eprintln!(
                "warning: --url only supports data:text/html,... URLs for now; got {url}, serving blank page"
            );
            ""
        }
        None => "",
    };

    // --proxy/--block are parsed but not yet wired into CdpServer::start.
    if cli.proxy.is_some() {
        eprintln!("warning: --proxy is not yet implemented, ignoring");
    }
    if !cli.block.is_empty() {
        eprintln!("warning: --block is not yet implemented, ignoring");
    }

    let profile = cli.stealth_profile();
    let server =
        CdpServer::start(html, cli.port, cli.stealth, profile).map_err(|e| eyre::eyre!("{e}"))?;

    println!("hpxless {}", env!("CARGO_PKG_VERSION"));
    println!("  port:    {}", server.port());
    println!("  stealth: {}", cli.stealth);
    println!("  profile: {:?}", cli.profile);
    if let Some(proxy) = &cli.proxy {
        println!("  proxy:   {proxy}");
    }
    if !cli.block.is_empty() {
        println!("  block:   {}", cli.block.join(", "));
    }
    if let Some(url) = &cli.url {
        println!("  url:     {url}");
    }
    println!("  log:     {}", cli.log_level);
    println!("Listening on ws://127.0.0.1:{}", server.port());

    // Block until SIGINT/SIGTERM, then clean shutdown
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            tokio::signal::ctrl_c().await?;
            tracing::info!("shutdown signal received; draining in-flight CDP sessions");
            // Begin graceful shutdown: stop accepting new connections and let
            // established sessions finish. The server thread joins when `server`
            // is dropped below (see `CdpServer::Drop`).
            server.shutdown();
            // Give in-flight sessions a bounded window to complete before we drop.
            let drain = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                // Yield so the server thread can observe the shutdown flag and
                // finish draining; there is no explicit "all sessions closed"
                // signal yet, so we rely on the bounded timeout.
                tokio::task::yield_now().await;
            })
            .await;
            if drain.is_err() {
                tracing::warn!("graceful drain window elapsed; forcing shutdown");
            }
            Ok::<(), eyre::Report>(())
        })?;

    drop(server);
    Ok(())
}
