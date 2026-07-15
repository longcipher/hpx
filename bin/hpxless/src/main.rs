//! `hpxless` — CDP-compatible browser server for Puppeteer/Playwright.

#![allow(clippy::print_stdout)]
#![allow(clippy::print_stderr)]
#![allow(missing_docs)]
#![allow(linker_messages)]

mod cli;

use clap::Parser;
use cli::Cli;
use ecdysis::Ecdysis;
use hpx_browser::protocol::CdpServer;

fn main() -> eyre::Result<()> {
    let ecd = Ecdysis::default();
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
        Some(_) => "", // ponytail: real URL navigation deferred to later task
        None => "",
    };

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
            // ponytail: no in-flight navigations to drain yet; add timeout when URL nav lands
            Ok::<(), eyre::Report>(())
        })?;

    drop(server);
    ecd.quit();
    Ok(())
}
